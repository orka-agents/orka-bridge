//! Facade tests reuse real pinned-image, filesystem and transaction fixtures.
use super::*;
use crate::local_vm::{
    LocalFactory, LocalProgress, LocalVm, LocalVmStatus, StartFuture, UpgradeTarget,
};
use std::sync::{Arc, Mutex};

use crate::engine::{types::EngineRequest, TransportFuture};
use crate::local_vm::LocalSession;
use crate::runtime::{
    types::{BackendHandle, LaunchSpec, VmBackend},
    RuntimeWorker,
};

struct Backend;
impl VmBackend for Backend {
    fn create_state(&mut self, spec: &LaunchSpec) -> Result<(), RuntimeError> {
        // Stand in only for the platform formatter, never for journal admission.
        fs::OpenOptions::new()
            .write(true)
            .open(&spec.state_path)?
            .set_len(64)?;
        Ok(())
    }
    fn start(&mut self, _: &LaunchSpec) -> Result<BackendHandle, RuntimeError> {
        Ok(BackendHandle::new(()))
    }
    fn stop(&mut self, _: &BackendHandle) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn pause(&mut self, _: &BackendHandle) -> Result<(), RuntimeError> {
        unreachable!()
    }
    fn resume(&mut self, _: &BackendHandle) -> Result<(), RuntimeError> {
        unreachable!()
    }
}
struct Session(RuntimeWorker, Arc<std::sync::atomic::AtomicUsize>);
impl LocalSession for Session {
    fn execute(&mut self, _: EngineRequest) -> TransportFuture<'_> {
        panic!("upgrade must never send Connect/Test/Chat")
    }
    fn stop(&mut self) -> Result<(), RuntimeError> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.0.stop()
    }
}
struct DestinationFactory {
    root: PathBuf,
    data: PathBuf,
    destination: Mutex<Option<Journal>>,
    adoption: Option<(
        std::sync::mpsc::Sender<()>,
        Mutex<std::sync::mpsc::Receiver<()>>,
    )>,
    launched: Mutex<Vec<Uuid>>,
    stops: Arc<std::sync::atomic::AtomicUsize>,
    releases: std::sync::atomic::AtomicUsize,
}
impl LocalFactory for DestinationFactory {
    fn adopt_destination(&self, journal: Journal) -> Result<(), RuntimeError> {
        if let Some((entered, release)) = &self.adoption {
            entered.send(()).unwrap();
            release.lock().unwrap().recv().unwrap();
        }
        *self.destination.lock().unwrap() = Some(journal);
        Ok(())
    }
    fn release_stopped(&self) -> Result<(), RuntimeError> {
        self.releases
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.destination.lock().unwrap().take();
        Ok(())
    }
    fn launch(
        &self,
        _: watch::Receiver<bool>,
        _: Arc<dyn Fn(LocalProgress) + Send + Sync>,
    ) -> StartFuture<'_> {
        // Exercise the same live-journal admission used by NativeFactory. No
        // platform VM or guest messages (Connect/Test/Chat) are involved.
        Box::pin(async {
            let journal = match self.destination.lock().unwrap().take() {
                Some(journal) => {
                    admit_prepared(&self.data, &self.root, &journal)?;
                    journal.require_fresh_creation()?;
                    journal
                }
                None => {
                    admit(&self.data, &self.root)?;
                    Journal::open(&self.root)?
                }
            };
            let installation = journal.record().installation_id;
            let fresh = journal.record().phase != StatePhase::Consumed;
            let installed = Arc::new(crate::runtime::image_install::open_installed_image(
                &journal.image_target("new-state")?,
            )?);
            let mut worker = RuntimeWorker::new(Box::new(Backend), journal, installed)?;
            worker.start()?;
            let boot = worker.boot_request(
                &bridge_runtime_protocol::StageHello {
                    version: bridge_runtime_protocol::VERSION,
                    stage: bridge_runtime_protocol::Stage::Early,
                    guest_nonce: Uuid::new_v4(),
                    accepted_boot_nonce: None,
                    run_cleanup: false,
                },
                1,
            )?;
            let bridge_runtime_protocol::RequestBody::Boot(descriptor) = boot.body else {
                panic!("boot descriptor")
            };
            assert_eq!(
                descriptor.state_creation,
                if fresh {
                    bridge_runtime_protocol::StateCreation::New
                } else {
                    bridge_runtime_protocol::StateCreation::Existing
                }
            );
            self.launched.lock().unwrap().push(installation);
            Ok(Box::new(Session(worker, self.stops.clone())) as Box<dyn LocalSession>)
        })
    }
}
fn facade(
    scene: &Scene,
    adoption: Option<(
        std::sync::mpsc::Sender<()>,
        Mutex<std::sync::mpsc::Receiver<()>>,
    )>,
) -> (Arc<LocalVm>, Arc<DestinationFactory>) {
    let factory = Arc::new(DestinationFactory {
        root: scene.local.join("local-vm-v1"),
        data: scene.data.clone(),
        destination: Mutex::new(None),
        adoption,
        launched: Mutex::new(Vec::new()),
        stops: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        releases: std::sync::atomic::AtomicUsize::new(0),
    });
    (LocalVm::new(factory.clone()), factory)
}
fn target(scene: &Scene) -> UpgradeTarget {
    UpgradeTarget {
        source: scene.source.clone(),
        machine_id: vec![2],
    }
}
fn seed_cache(scene: &mut Scene) {
    // Seed exactly the verified cache which production acquisition reads; no
    // bypass of replace/retry, manifest policy or materialization.
    scene.target.take();
    fs::rename(
        scene._temp.path().join("new-cache/cache"),
        scene.local.join("image-cache-v1"),
    )
    .unwrap();
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn facade_replace_hands_live_creation_authority_to_launch_and_reports_fresh() {
    let mut scene = Scene::new(ImageTarget::WindowsAmd64);
    seed_cache(&mut scene);
    let (vm, factory) = facade(&scene, None);
    runtime().block_on(async {
        let dto = vm
            .replace_workspace(scene.data.clone(), scene.local.clone(), target(&scene))
            .await
            .unwrap();
        assert_eq!(dto.state, "fresh");
        assert!(!dto.busy);
        assert!(vm.workspace_replaced());
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while factory.launched.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(factory.launched.lock().unwrap().len(), 1);
        assert_ne!(factory.launched.lock().unwrap()[0], scene.source_id);
        assert_eq!(vm.status(), LocalVmStatus::Ready);
        let status = vm
            .upgrade_status(scene.data.clone(), scene.local.clone())
            .await;
        assert_eq!(status.state, "consumed");
        assert!(
            !status.busy,
            "our live VM journal is not mutation contention"
        );
        assert_eq!(
            read_record(&scene.local.join("local-vm-v1")).unwrap().phase,
            StatePhase::Consumed
        );
        assert!(!vm.model_connected());
        vm.shutdown().await.unwrap();
    });
}

#[test]
fn facade_dropped_forward_waiter_never_releases_lifecycle_before_work_finishes() {
    for retry in [false, true] {
        let mut scene = Scene::new(ImageTarget::WindowsAmd64);
        let transaction = if retry {
            scene.replace(Some("intent")).unwrap_err();
            Some(
                WorkspaceUpgrade::open(&scene.data, &scene.local)
                    .unwrap()
                    .read_intent()
                    .unwrap()
                    .unwrap()
                    .transaction,
            )
        } else {
            None
        };
        seed_cache(&mut scene);
        let (entered, entered_rx) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let (vm, factory) = facade(&scene, Some((entered, Mutex::new(release_rx))));
        let retired = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = retired.clone();
        vm.on_workspace_replaced(Arc::new(move || {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        runtime().block_on(async {
            let worker = {
                let (vm, data, local, target) = (
                    vm.clone(),
                    scene.data.clone(),
                    scene.local.clone(),
                    target(&scene),
                );
                tokio::spawn(async move {
                    match transaction {
                        Some(id) => vm.retry_workspace(data, local, target, id).await,
                        None => vm.replace_workspace(data, local, target).await,
                    }
                })
            };
            tokio::task::spawn_blocking(move || {
                entered_rx
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap()
            })
            .await
            .unwrap();
            worker.abort();
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            vm.start();
            let held_status = vm.status();
            let launched_early = !factory.launched.lock().unwrap().is_empty();
            release.send(()).unwrap();
            let _ = worker.await;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while retired.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await;
            assert_eq!(
                retired.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "native commit must retire ephemeral state even after the IPC waiter drops"
            );
            assert!(matches!(
                held_status,
                LocalVmStatus::Cancelled | LocalVmStatus::Preparing
            ));
            assert!(
                !launched_early,
                "gate released before worker finished; retry={retry}"
            );
            vm.shutdown().await.unwrap();
        });
    }
}

#[test]
fn facade_rejects_foreign_nil_and_replayed_transactions_without_mutating_identity() {
    for id in [Uuid::nil(), Uuid::from_u128(123)] {
        let mut scene = Scene::new(ImageTarget::WindowsAmd64);
        seed_cache(&mut scene);
        let (vm, factory) = facade(&scene, None);
        runtime().block_on(async {
            let dto = vm
                .replace_workspace(scene.data.clone(), scene.local.clone(), target(&scene))
                .await
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while vm.status() != LocalVmStatus::Ready {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let releases = factory.releases.load(std::sync::atomic::Ordering::SeqCst);
            let before = fs::read(scene.local.join("workspace-upgrades-v1/intent.json")).unwrap();
            assert!(vm
                .retry_workspace(scene.data.clone(), scene.local.clone(), target(&scene), id)
                .await
                .is_err());
            assert!(vm
                .restore_workspace(scene.data.clone(), scene.local.clone(), id)
                .await
                .is_err());
            assert!(vm
                .transfer_workspace_credential(scene.data.clone(), scene.local.clone(), id)
                .await
                .is_err());
            assert!(vm
                .remove_workspace_credential(scene.data.clone(), scene.local.clone(), id)
                .await
                .is_err());
            assert_eq!(
                fs::read(scene.local.join("workspace-upgrades-v1/intent.json")).unwrap(),
                before
            );
            assert_eq!(factory.stops.load(std::sync::atomic::Ordering::SeqCst), 0);
            assert_eq!(
                factory.releases.load(std::sync::atomic::Ordering::SeqCst),
                releases
            );
            assert_eq!(vm.status(), LocalVmStatus::Ready);
            assert!(vm
                .replace_workspace(scene.data.clone(), scene.local.clone(), target(&scene))
                .await
                .is_err());
            assert!(vm
                .retry_workspace(
                    scene.data.clone(),
                    scene.local.clone(),
                    target(&scene),
                    dto.transaction.unwrap()
                )
                .await
                .is_err());
            assert_eq!(
                factory.stops.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "direct initial-replace bypass must not stop current B"
            );
            assert_eq!(
                factory.releases.load(std::sync::atomic::Ordering::SeqCst),
                releases
            );
            assert_eq!(vm.status(), LocalVmStatus::Ready);
            assert!(dto.transaction.is_some());
            assert!(factory.launched.lock().unwrap().len() <= 1);
        });
    }
}

#[test]
fn facade_explicit_retry_from_blocked_startup_boots_only_new_b() {
    let mut scene = Scene::new(ImageTarget::WindowsAmd64);
    scene.replace(Some("intent")).unwrap_err();
    let transaction = WorkspaceUpgrade::open(&scene.data, &scene.local)
        .unwrap()
        .read_intent()
        .unwrap()
        .unwrap()
        .transaction;
    seed_cache(&mut scene);
    let (vm, factory) = facade(&scene, None);
    vm.block_for_recovery();
    runtime().block_on(async {
        vm.start();
        tokio::task::yield_now().await;
        assert!(factory.launched.lock().unwrap().is_empty());
        let dto = vm
            .retry_workspace(
                scene.data.clone(),
                scene.local.clone(),
                target(&scene),
                transaction,
            )
            .await
            .unwrap();
        assert_eq!(dto.state, "fresh");
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while vm.status() != LocalVmStatus::Ready {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_ne!(factory.launched.lock().unwrap()[0], scene.source_id);
        assert_eq!(
            vm.upgrade_status(scene.data.clone(), scene.local.clone())
                .await
                .state,
            "consumed"
        );
        assert!(!vm.model_connected());
        vm.transfer_workspace_credential(scene.data.clone(), scene.local.clone(), transaction)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while vm.status() != LocalVmStatus::Ready {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let launched = factory.launched.lock().unwrap().clone();
        assert_eq!(launched.len(), 2);
        assert_eq!(launched[0], launched[1]);
        assert!(!vm.model_connected());
        let stops = factory.stops.load(std::sync::atomic::Ordering::SeqCst);
        let releases = factory.releases.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            vm.transfer_workspace_credential(scene.data.clone(), scene.local.clone(), transaction)
                .await
                .is_err(),
            "completed transfer replay must not stop or restart B"
        );
        assert_eq!(
            factory.stops.load(std::sync::atomic::Ordering::SeqCst),
            stops
        );
        assert_eq!(
            factory.releases.load(std::sync::atomic::Ordering::SeqCst),
            releases
        );
        assert_eq!(vm.status(), LocalVmStatus::Ready);
        vm.shutdown().await.unwrap();
    });
}

#[test]
fn facade_cold_consumed_restart_seeds_transaction_before_ordinary_startup() {
    let mut scene = Scene::new(ImageTarget::WindowsAmd64);
    seed_cache(&mut scene);
    let (vm, _) = facade(&scene, None);
    runtime().block_on(async {
        let before = vm
            .replace_workspace(scene.data.clone(), scene.local.clone(), target(&scene))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while vm.status() != LocalVmStatus::Ready {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        vm.shutdown().await.unwrap();
        let (reopened, _) = facade(&scene, None);
        reopened.seed_upgrade_status(&scene.data, &scene.local);
        reopened.start();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while reopened.status() != LocalVmStatus::Ready {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let status = reopened
            .upgrade_status(scene.data.clone(), scene.local.clone())
            .await;
        assert_eq!(status.transaction, before.transaction);
        assert_eq!(status.state, "consumed");
        assert_eq!(status.credential, "eligible");
        assert!(status.can_restore);
        assert!(!status.busy);
        reopened.shutdown().await.unwrap();
    });
}

#[test]
fn facade_foreign_transaction_on_cold_recovery_cannot_release_any_native_custody() {
    let mut scene = Scene::new(ImageTarget::WindowsAmd64);
    scene.replace(Some("intent")).unwrap_err();
    let (vm, factory) = facade(&scene, None);
    vm.block_for_recovery();
    let foreign = Uuid::new_v4();
    runtime().block_on(async {
        assert!(vm
            .restore_workspace(scene.data.clone(), scene.local.clone(), foreign)
            .await
            .is_err());
        assert!(vm
            .transfer_workspace_credential(scene.data.clone(), scene.local.clone(), foreign)
            .await
            .is_err());
        assert!(vm
            .remove_workspace_credential(scene.data.clone(), scene.local.clone(), foreign)
            .await
            .is_err());
        assert!(vm
            .retry_workspace(
                scene.data.clone(),
                scene.local.clone(),
                target(&scene),
                foreign
            )
            .await
            .is_err());
    });
    assert_eq!(
        factory.releases.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "cache absence never authorizes lifecycle side effects"
    );
    assert_eq!(
        read_record(&scene.local.join("local-vm-v1"))
            .unwrap()
            .installation_id,
        scene.source_id
    );
}

#[test]
fn facade_restore_stops_current_b_and_restores_a_without_starting_it() {
    let mut scene = Scene::new(ImageTarget::WindowsAmd64);
    seed_cache(&mut scene);
    let (vm, factory) = facade(&scene, None);
    runtime().block_on(async {
        let dto = vm
            .replace_workspace(scene.data.clone(), scene.local.clone(), target(&scene))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while vm.status() != LocalVmStatus::Ready {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let restored = vm
            .restore_workspace(
                scene.data.clone(),
                scene.local.clone(),
                dto.transaction.unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(restored.state, "restored");
        assert_eq!(factory.stops.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            factory.releases.load(std::sync::atomic::Ordering::SeqCst),
            2
        );
        assert_eq!(
            read_record(&scene.local.join("local-vm-v1"))
                .unwrap()
                .installation_id,
            scene.source_id
        );
        vm.start();
        tokio::task::yield_now().await;
        assert_eq!(
            factory.launched.lock().unwrap().len(),
            1,
            "A is restored for its matching older app, never booted here"
        );
        assert!(!vm.model_connected());
    });
}

#[test]
fn facade_simultaneous_forward_requests_admit_exactly_one_without_stopping_its_b() {
    for retry in [false, true] {
        let mut scene = Scene::new(ImageTarget::WindowsAmd64);
        let transaction = if retry {
            scene.replace(Some("intent")).unwrap_err();
            Some(
                WorkspaceUpgrade::open(&scene.data, &scene.local)
                    .unwrap()
                    .read_intent()
                    .unwrap()
                    .unwrap()
                    .transaction,
            )
        } else {
            None
        };
        seed_cache(&mut scene);
        let (vm, factory) = facade(&scene, None);
        runtime().block_on(async {
            let forward = || async {
                match transaction {
                    Some(id) => vm.retry_workspace(scene.data.clone(), scene.local.clone(), target(&scene), id).await,
                    None => vm.replace_workspace(scene.data.clone(), scene.local.clone(), target(&scene)).await,
                }
            };
            // Both futures are polled before either owned worker can run.
            let (first, second) = tokio::join!(forward(), forward());
            assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
            let rejected = first.as_ref().err().or(second.as_ref().err());
            assert!(matches!(rejected, Some(RuntimeError::Busy)), "the second call must lose admission, not stop B and fail in storage; retry={retry}: {rejected:?}");
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while vm.status() != LocalVmStatus::Ready { tokio::task::yield_now().await; }
            }).await.unwrap();
            assert_eq!(factory.stops.load(std::sync::atomic::Ordering::SeqCst), 0);
            assert_eq!(factory.releases.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(factory.launched.lock().unwrap().len(), 1);
            vm.shutdown().await.unwrap();
        });
    }
}

#[test]
fn facade_completed_removal_ack_refuses_replay_before_stopping_b() {
    let mut scene = Scene::new(ImageTarget::WindowsAmd64);
    seed_cache(&mut scene);
    let (vm, factory) = facade(&scene, None);
    runtime().block_on(async {
        let dto = vm
            .replace_workspace(scene.data.clone(), scene.local.clone(), target(&scene))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while vm.status() != LocalVmStatus::Ready {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let owner = WorkspaceUpgrade::open(&scene.data, &scene.local).unwrap();
        let slot = owner
            .resolve_removal_slot(dto.transaction.unwrap())
            .unwrap();
        // Durable acknowledgement from an earlier exact-slot removal, without
        // consulting a real credential store or refreshing the cached DTO.
        owner.write_slot_outcome(slot, b"removed").unwrap();
        drop(owner);
        assert!(vm
            .remove_workspace_credential(
                scene.data.clone(),
                scene.local.clone(),
                dto.transaction.unwrap()
            )
            .await
            .is_err());
        assert_eq!(factory.stops.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(
            factory.releases.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(vm.status(), LocalVmStatus::Ready);
        vm.shutdown().await.unwrap();
    });
}

#[test]
fn facade_restore_after_restart_never_launches_the_old_installation() {
    let mut scene = Scene::new(ImageTarget::WindowsAmd64);
    scene.replace(Some("intent")).unwrap_err();
    let transaction = WorkspaceUpgrade::open(&scene.data, &scene.local)
        .unwrap()
        .read_intent()
        .unwrap()
        .unwrap()
        .transaction;
    let (vm, factory) = facade(&scene, None);
    vm.block_for_recovery();
    runtime().block_on(async {
        vm.start();
        tokio::task::yield_now().await;
        assert!(factory.launched.lock().unwrap().is_empty());
        let dto = vm
            .restore_workspace(scene.data.clone(), scene.local.clone(), transaction)
            .await
            .unwrap();
        assert_eq!(dto.state, "restored");
        vm.start();
        tokio::task::yield_now().await;
        assert!(factory.launched.lock().unwrap().is_empty());
        assert_eq!(
            read_record(&scene.local.join("local-vm-v1"))
                .unwrap()
                .installation_id,
            scene.source_id
        );
    });
}
