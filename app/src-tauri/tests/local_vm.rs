use bridge_engine::{
    engine::{types::*, EngineState, EngineTransport, TransportFuture},
    local_vm::{LocalFactory, LocalSession, LocalVm, LocalVmStatus, StartFuture},
    runtime::types::RuntimeError,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Condvar, Mutex,
};
use tokio::sync::{watch, Notify};

#[derive(Default)]
struct Probe {
    starts: AtomicUsize,
    stops: AtomicUsize,
    launch: Notify,
    entered: Notify,
    io: (Mutex<bool>, Condvar),
    order: Mutex<Vec<&'static str>>,
    fail_stop: bool,
    fail_launch: bool,
}
struct Factory(Arc<Probe>);
struct Session(Arc<Probe>);
impl LocalFactory for Factory {
    fn launch(
        &self,
        _: watch::Receiver<bool>,
        progress: Arc<dyn Fn(LocalVmStatus) + Send + Sync>,
    ) -> StartFuture<'_> {
        let p = self.0.clone();
        Box::pin(async move {
            p.starts.fetch_add(1, Ordering::SeqCst);
            progress(LocalVmStatus::Installing);
            p.launch.notified().await;
            if p.fail_launch {
                return Err(RuntimeError::Unavailable);
            }
            Ok(Box::new(Session(p)) as Box<dyn LocalSession>)
        })
    }
}
impl LocalSession for Session {
    fn execute(&mut self, _: EngineRequest) -> TransportFuture<'_> {
        let p = self.0.clone();
        Box::pin(async move {
            // Mac-shaped nested blocking work: it survives a dropped outer caller.
            tokio::task::spawn_blocking(move || {
                p.order.lock().unwrap().push("read");
                p.entered.notify_one();
                let (lock, wake) = &p.io;
                let ready = wake
                    .wait_timeout_while(
                        lock.lock().unwrap(),
                        std::time::Duration::from_secs(3),
                        |ready| !*ready,
                    )
                    .unwrap();
                assert!(*ready.0, "test must explicitly finish underlying I/O");
                p.order.lock().unwrap().push("read-done");
                Ok(EngineOutput {
                    exit: 0,
                    stdout: b"kmx v0.9.0\n".to_vec(),
                    stderr: Vec::new(),
                })
            })
            .await
            .unwrap()
        })
    }
    fn stop(&mut self) -> Result<(), RuntimeError> {
        self.0.stops.fetch_add(1, Ordering::SeqCst);
        self.0.order.lock().unwrap().push("stop");
        if self.0.fail_stop {
            Err(RuntimeError::Unavailable)
        } else {
            Ok(())
        }
    }
}
fn owner(p: Arc<Probe>) -> Arc<LocalVm> {
    LocalVm::new(Arc::new(Factory(p)))
}
async fn phase(vm: &LocalVm, expected: LocalVmStatus) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while vm.status() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn finish(p: &Probe) {
    *p.io.0.lock().unwrap() = true;
    p.io.1.notify_all();
}
fn request() -> EngineRequest {
    EngineRequest {
        action: EngineAction::Version {},
    }
}

#[tokio::test]
async fn selection_schedules_only_one_session_and_status_is_observational() {
    let p = Arc::new(Probe::default());
    let vm = owner(p.clone());
    assert_eq!(vm.status(), LocalVmStatus::Idle);
    assert_eq!(p.starts.load(Ordering::SeqCst), 0);
    vm.start();
    vm.start();
    phase(&vm, LocalVmStatus::Installing).await;
    assert_eq!(p.starts.load(Ordering::SeqCst), 1);
    let engine = EngineState::with_transport(vm.clone());
    assert!(
        engine.call(request()).await.is_err(),
        "selection is not readiness"
    );
    p.launch.notify_one();
    phase(&vm, LocalVmStatus::Ready).await;
    finish(&p);
    assert!(matches!(
        engine.call(request()).await.unwrap(),
        EngineReply::Version { .. }
    ));
    vm.shutdown().await.unwrap();
    assert_eq!(&*p.order.lock().unwrap(), &["read", "read-done", "stop"]);
    assert_eq!(p.stops.load(Ordering::SeqCst), 1);
    vm.start();
    assert_eq!(p.starts.load(Ordering::SeqCst), 1);
    assert!(engine.call(request()).await.is_err());
}
#[tokio::test]
async fn close_during_start_waits_and_stops_late_session_without_ready() {
    let p = Arc::new(Probe::default());
    let vm = owner(p.clone());
    vm.start();
    phase(&vm, LocalVmStatus::Installing).await;
    let closing = {
        let vm = vm.clone();
        tokio::spawn(async move { vm.shutdown().await })
    };
    phase(&vm, LocalVmStatus::Closing).await;
    assert!(!closing.is_finished());
    p.launch.notify_one();
    closing.await.unwrap().unwrap();
    assert_eq!(vm.status(), LocalVmStatus::Closed);
    assert_eq!(p.stops.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn cancelled_request_keeps_session_until_inner_io_finishes_and_refuses_queueing() {
    let p = Arc::new(Probe::default());
    let vm = owner(p.clone());
    vm.start();
    p.launch.notify_one();
    phase(&vm, LocalVmStatus::Ready).await;
    let engine = Arc::new(EngineState::with_transport(vm.clone()));
    let running = {
        let e = engine.clone();
        tokio::spawn(async move { e.call(request()).await })
    };
    p.entered.notified().await;
    assert_eq!(
        engine.call(request()).await.unwrap_err().kind,
        ErrorKind::Execution
    );
    running.abort();
    let _ = running.await;
    let closing = {
        let vm = vm.clone();
        tokio::spawn(async move { vm.shutdown().await })
    };
    phase(&vm, LocalVmStatus::Closing).await;
    assert!(!closing.is_finished());
    assert_eq!(p.stops.load(Ordering::SeqCst), 0);
    finish(&p);
    closing.await.unwrap().unwrap();
    assert_eq!(&*p.order.lock().unwrap(), &["read", "read-done", "stop"]);
}
#[tokio::test]
async fn cancelled_close_waiter_does_not_abandon_startup_or_custody() {
    let p = Arc::new(Probe::default());
    let vm = owner(p.clone());
    vm.start();
    phase(&vm, LocalVmStatus::Installing).await;
    let closing = {
        let vm = vm.clone();
        tokio::spawn(async move { vm.shutdown().await })
    };
    phase(&vm, LocalVmStatus::Closing).await;
    closing.abort();
    let _ = closing.await;
    p.launch.notify_one();
    vm.shutdown().await.unwrap();
    assert_eq!(vm.status(), LocalVmStatus::Closed);
    assert_eq!(p.stops.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn failed_stop_cannot_claim_closed_clean_or_restart() {
    let p = Arc::new(Probe {
        fail_stop: true,
        ..Probe::default()
    });
    let vm = owner(p.clone());
    vm.start();
    p.launch.notify_one();
    phase(&vm, LocalVmStatus::Ready).await;
    assert!(vm.shutdown().await.is_err());
    assert_eq!(vm.status(), LocalVmStatus::Failed);
    vm.start();
    assert_eq!(p.starts.load(Ordering::SeqCst), 1);
    assert!(vm.shutdown().await.is_err());
    assert_eq!(p.stops.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn native_unconfigured_owner_never_starts_from_selection() {
    let vm = LocalVm::unavailable();
    vm.start();
    assert_eq!(vm.status(), LocalVmStatus::NotIntegrated);
    vm.shutdown().await.unwrap();
    assert!(vm.execute(request()).await.is_err());
}
#[tokio::test]
async fn startup_error_cannot_become_clean_close_with_an_empty_session_slot() {
    for during_close in [false, true] {
        let p = Arc::new(Probe {
            fail_launch: true,
            ..Probe::default()
        });
        let vm = owner(p.clone());
        vm.start();
        phase(&vm, LocalVmStatus::Installing).await;
        if during_close {
            vm.request_close();
        }
        p.launch.notify_one();
        if !during_close {
            phase(&vm, LocalVmStatus::Failed).await;
        }
        assert!(
            vm.shutdown().await.is_err(),
            "joining a returned task is not proof of successful native cleanup"
        );
        assert_eq!(vm.status(), LocalVmStatus::Failed);
        assert_eq!(p.stops.load(Ordering::SeqCst), 0);
        assert!(vm.shutdown().await.is_err());
    }
}
#[test]
fn native_close_signal_bars_ready_without_an_async_runtime() {
    let vm = LocalVm::unavailable();
    vm.request_close();
    assert_eq!(vm.status(), LocalVmStatus::Closing);
}
#[tokio::test]
async fn setup_selection_persists_then_starts_but_status_never_starts() {
    use bridge_engine::{local_vm, setup::SetupStore};
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SetupStore::new(Some(temp.path().to_owned())));
    let p = Arc::new(Probe::default());
    let vm = owner(p.clone());
    let observed = local_vm::setup_status(store.clone(), &vm).await.unwrap();
    assert!(observed.selected.is_none());
    assert_eq!(p.starts.load(Ordering::SeqCst), 0);
    let selected = local_vm::select_local(store.clone(), &vm).await.unwrap();
    assert!(selected.selected.is_some());
    assert!(temp.path().join("setup-v1/local-vm").is_file());
    phase(&vm, LocalVmStatus::Installing).await;
    assert_eq!(p.starts.load(Ordering::SeqCst), 1);
    local_vm::select_local(store.clone(), &vm).await.unwrap();
    assert_eq!(p.starts.load(Ordering::SeqCst), 1);
    p.launch.notify_one();
    phase(&vm, LocalVmStatus::Ready).await;
    let value = serde_json::to_value(local_vm::setup_status(store, &vm).await.unwrap()).unwrap();
    assert_eq!(value["localVm"], "ready");
    assert_eq!(value["model"], "locked");
    vm.shutdown().await.unwrap();
}
#[tokio::test]
async fn native_resume_starts_only_a_saved_choice_without_resaving_it() {
    use bridge_engine::{local_vm, setup::SetupStore};
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(SetupStore::new(Some(temp.path().to_owned())));
    let p = Arc::new(Probe::default());
    let vm = owner(p.clone());
    local_vm::resume_selected(store.clone(), &vm).await.unwrap();
    assert_eq!(p.starts.load(Ordering::SeqCst), 0);
    store.select_local().unwrap();
    local_vm::resume_selected(store, &vm).await.unwrap();
    phase(&vm, LocalVmStatus::Installing).await;
    p.launch.notify_one();
    phase(&vm, LocalVmStatus::Ready).await;
    vm.shutdown().await.unwrap();
}
#[test]
fn fixed_example_is_the_exact_public_pair_not_an_edit_or_run_operation() {
    use sha2::{Digest, Sha256};
    let pair = bridge_engine::local_vm::example();
    assert_eq!(
        format!("{:x}", Sha256::digest(pair.agent.as_bytes())),
        "5c92e1867edf076e62723120a12264a0c5106e3047bfc8901973803f440c39af"
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(pair.tests.as_bytes())),
        "710f4309a3b482ccc41049b204a9d464460ed4285285d73f5e58e7487c62c3ba"
    );
    assert!(pair.agent.len() < 65536 && pair.tests.len() < 65536);
}
#[tokio::test]
async fn engine_shutdown_bars_transport_calls() {
    let p = Arc::new(Probe::default());
    let vm = owner(p.clone());
    vm.start();
    p.launch.notify_one();
    phase(&vm, LocalVmStatus::Ready).await;
    finish(&p);
    let engine = EngineState::with_transport(vm.clone());
    engine.shutdown();
    assert!(engine.call(request()).await.is_err());
    assert!(p.order.lock().unwrap().is_empty());
    vm.shutdown().await.unwrap();
}
