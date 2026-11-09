use super::*;
use sha2::{Digest, Sha256};
use std::fs;

mod infrastructure_tests {
    use super::*;
    use bridge_runtime_protocol::*;
    use std::{
        io::{self, Read, Write},
        sync::atomic::{AtomicU8, AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct Probe {
        state: AtomicU8,
        polls: AtomicUsize,
        stops: AtomicUsize,
        fail_stop: bool,
    }
    struct Channel {
        probe: Arc<Probe>,
        reply: io::Cursor<Vec<u8>>,
    }
    impl Read for Channel {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.reply.read(out)
        }
    }
    impl Write for Channel {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let Frame::Request(request) = serde_json::from_slice(bytes).unwrap() else {
                panic!("request")
            };
            assert!(
                matches!(request.body, RequestBody::Readiness {}),
                "startup may only observe readiness"
            );
            self.probe.polls.fetch_add(1, Ordering::SeqCst);
            let state = self.probe.state.load(Ordering::SeqCst);
            self.reply = io::Cursor::new(
                encode(&Frame::Result(WireResult {
                    version: VERSION,
                    boot_nonce: request.boot_nonce,
                    request_id: request.request_id,
                    body: ResultBody::Infrastructure(InfrastructureState {
                        phase: Some(if state == 0 {
                            BootstrapPhase::ClusterReady
                        } else {
                            BootstrapPhase::InfrastructureReady
                        }),
                        service: match state {
                            0 => InfrastructureService::Starting,
                            1 => InfrastructureService::Ready,
                            _ => InfrastructureService::Failed,
                        },
                    }),
                }))
                .unwrap(),
            );
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    struct TestSession(Arc<Probe>, VmClient);
    impl LocalSession for TestSession {
        fn with_client(
            &mut self,
            work: &mut dyn FnMut(Uuid, &mut VmClient) -> Result<(), RuntimeError>,
        ) -> Result<(), RuntimeError> {
            work(Uuid::from_u128(42), &mut self.1)
        }
        fn execute(&mut self, _: EngineRequest) -> TransportFuture<'_> {
            Box::pin(async { unreachable!() })
        }
        fn stop(&mut self) -> Result<(), RuntimeError> {
            self.0.stops.fetch_add(1, Ordering::SeqCst);
            if self.0.fail_stop {
                Err(io::Error::other("stop detail").into())
            } else {
                Ok(())
            }
        }
    }
    struct Factory(Arc<Probe>);
    impl LocalFactory for Factory {
        fn launch(
            &self,
            closed: watch::Receiver<bool>,
            progress: Arc<dyn Fn(LocalVmStatus) + Send + Sync>,
        ) -> StartFuture<'_> {
            let probe = self.0.clone();
            Box::pin(blocking_start(move |runtime| {
                progress(LocalVmStatus::Starting);
                let client = VmClient::established(
                    Box::new(Channel {
                        probe: probe.clone(),
                        reply: io::Cursor::new(Vec::new()),
                    }),
                    Uuid::new_v4(),
                )?;
                wait_for_infrastructure(Box::new(TestSession(probe, client)), closed, runtime)
            }))
        }
    }
    async fn phase(vm: &LocalVm, expected: LocalVmStatus) {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while vm.status() != expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    async fn polled(probe: &Probe) {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while probe.polls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn starting_infrastructure_locks_credentials_and_connect_until_ready() {
        let probe = Arc::new(Probe::default());
        let vm = LocalVm::new(Arc::new(Factory(probe.clone())));
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::setup::SetupStore::new(Some(temp.path().to_owned())));
        vm.start();
        phase(&vm, LocalVmStatus::Starting).await;
        polled(&probe).await;
        let snapshot =
            serde_json::to_value(setup_status(store.clone(), &vm).await.unwrap()).unwrap();
        assert_eq!(snapshot["localVm"], "starting");
        assert_eq!(snapshot["model"], "locked");
        assert!(vm
            .with_client(|_, _| panic!("credential capture must not be admitted"))
            .await
            .is_err());
        assert!(vm.connect_model(store).await.is_err());
        assert_eq!(vm.status(), LocalVmStatus::Starting);
        probe.state.store(1, Ordering::SeqCst);
        phase(&vm, LocalVmStatus::Ready).await;
        vm.shutdown().await.unwrap();
        assert_eq!(probe.stops.load(Ordering::SeqCst), 1);
        assert!(probe.polls.load(Ordering::SeqCst) >= 2);
    }
    #[tokio::test]
    async fn failed_bootstrap_never_becomes_running_and_preserves_stop_error() {
        for fail_stop in [false, true] {
            let probe = Arc::new(Probe {
                state: AtomicU8::new(2),
                fail_stop,
                ..Probe::default()
            });
            let vm = LocalVm::new(Arc::new(Factory(probe.clone())));
            vm.start();
            phase(&vm, LocalVmStatus::Failed).await;
            let temp = tempfile::tempdir().unwrap();
            let store = Arc::new(crate::setup::SetupStore::new(Some(temp.path().to_owned())));
            let snapshot = serde_json::to_value(setup_status(store, &vm).await.unwrap()).unwrap();
            assert_eq!(snapshot["model"], "locked");
            let error = snapshot["localVmError"].as_str().unwrap();
            assert!(error.contains("infrastructure"), "{error}");
            assert_eq!(error.contains("stop detail"), fail_stop, "{error}");
            assert_eq!(probe.stops.load(Ordering::SeqCst), 1);
        }
    }
    #[tokio::test]
    async fn close_wakes_the_infrastructure_wait_and_stops_once() {
        let probe = Arc::new(Probe::default());
        let vm = LocalVm::new(Arc::new(Factory(probe.clone())));
        vm.start();
        polled(&probe).await;
        tokio::time::timeout(std::time::Duration::from_secs(1), vm.shutdown())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(vm.status(), LocalVmStatus::Closed);
        assert_eq!(probe.stops.load(Ordering::SeqCst), 1);
    }
}

// Native startup must acquire the real Windows inheritance gate after returning
// from async image work, not while its blocking worker is inside block_on.
#[tokio::test]
async fn native_startup_from_tokio_can_enter_the_windows_spawn_window() {
    let result = blocking_start(|runtime| {
        runtime.block_on(async { tokio::task::yield_now().await });
        let _guard = crate::runtime::windows::job::spawn_guard()?;
        Err(std::io::Error::from_raw_os_error(193).into())
    })
    .await;
    assert!(matches!(result, Err(RuntimeError::Io(error)) if error.raw_os_error() == Some(193)));
}

#[cfg(all(debug_assertions, windows))]
use crate::runtime::windows::backend::boot_tests::fixture;

#[cfg(all(debug_assertions, windows))]
#[tokio::test]
async fn windows_session_startup_inside_tokio_reaches_child_creation_without_panicking() {
    use crate::runtime::{image_install::materialize_image, image_types::ImageTarget};
    let temp = tempfile::tempdir_in(std::env::var_os("USERPROFILE").unwrap()).unwrap();
    let root = temp.path().join("install");
    let journal = Journal::create(&root, BackendKind::Qemu, b"pc-q35-11.0".to_vec()).unwrap();
    let verified = fixture::verified(temp.path(), ImageTarget::WindowsAmd64);
    let target = journal
        .image_target(&verified.image().identity().state_compatibility_id)
        .unwrap();
    let (_sender, closed) = watch::channel(false);
    let image = materialize_image(&verified, &target, closed, |_| {})
        .await
        .unwrap();
    let runtime_root = temp.path().join("runtime");
    fs::create_dir_all(runtime_root.join("bin")).unwrap();
    let bytes = b"inert, not an executable";
    let pins: Vec<_> = ["bin/qemu-img.exe", "bin/qemu-system-x86_64.exe"]
        .into_iter()
        .map(|name| {
            fs::write(runtime_root.join(name), bytes).unwrap();
            FilePin {
                name: name.into(),
                bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(bytes)),
            }
        })
        .collect();
    let inputs = Arc::new(verify_windows_runtime(&pins, runtime_root).unwrap());
    let launch_root = root.clone();
    let result = blocking_start(move |_| {
        let native = start_windows_session(inputs, journal, Arc::new(image), &launch_root, 0)?;
        Ok(Box::new(native) as Box<dyn LocalSession>)
    })
    .await;
    // The real backend reaches CreateProcessW, which refuses the inert fixture.
    // A panic is mapped to Unavailable instead, so it cannot satisfy this check.
    match result {
        Err(RuntimeError::Io(error)) => {
            let reason = error.to_string();
            assert!(reason.contains("Run qemu-img"), "{reason}");
            assert!(
                reason.contains("(os error 193)") || reason.contains("(os error 216)"),
                "{reason}"
            );
            assert!(reason.contains("Details:"), "{reason}");
        }
        Err(error) => panic!("expected native executable refusal, got {error:?}"),
        Ok(_) => panic!("inert fixture must not start a session"),
    }
    assert_eq!(fs::metadata(root.join("state.disk")).unwrap().len(), 0);
    let evidence: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("startup-failure-")
                .then_some(path)
        })
        .collect();
    assert_eq!(evidence.len(), 1);
    assert!(evidence[0].join("failure.json").is_file());
    assert!(evidence[0].join("qemu-stderr.txt").is_file());
}

#[test]
fn failed_start_retains_stage_stderr_and_existing_installation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("boot.log"), b"").unwrap();
    fs::write(root.join("record.json"), b"retained record").unwrap();
    let error = RuntimeError::Io(std::io::Error::other("control connection refused"));
    let reported = retain_start_failure(
        root,
        "Connect guest control",
        error,
        Ok(b"qemu: diagnostic\n".to_vec()),
    );
    let text = failure_reason(&reported);
    assert!(text.contains("Connect guest control"));
    assert!(text.contains("control connection refused"));
    let failures: Vec<_> = fs::read_dir(root)
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("startup-failure-")
                .then_some(path)
        })
        .collect();
    assert_eq!(failures.len(), 1);
    let details: serde_json::Value =
        serde_json::from_slice(&fs::read(failures[0].join("failure.json")).unwrap()).unwrap();
    assert_eq!(details["stage"], "Connect guest control");
    assert!(details["reason"]
        .as_str()
        .unwrap()
        .contains("control connection refused"));
    assert_eq!(
        fs::read(failures[0].join("qemu-stderr.txt")).unwrap(),
        b"qemu: diagnostic\n"
    );
    assert!(text.contains(failures[0].to_str().unwrap()));
    assert_eq!(
        fs::read(root.join("record.json")).unwrap(),
        b"retained record"
    );
    assert_eq!(fs::read(root.join("boot.log")).unwrap(), b"");
}

#[test]
fn authenticated_nested_manifest_is_part_of_the_complete_runtime_inventory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir(root.join("bin")).unwrap();
    fs::create_dir(root.join("provenance")).unwrap();
    let bytes = b"inert runtime fixture";
    let sha = format!("{:x}", Sha256::digest(bytes));
    for name in ["qemu-img.exe", "qemu-system-x86_64.exe"] {
        fs::write(root.join("bin").join(name), bytes).unwrap();
    }
    let manifest = serde_json::to_vec(&serde_json::json!({"files":[
        {"path":"bin/qemu-img.exe","bytes":bytes.len(),"sha256":sha},
        {"path":"bin/qemu-system-x86_64.exe","bytes":bytes.len(),"sha256":sha}
    ]}))
    .unwrap();
    // Test authority is independently retained here, not read from that file.
    let pin = FilePin {
        name: "provenance/runtime-manifest.json".into(),
        bytes: manifest.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&manifest)),
    };
    fs::write(root.join(&pin.name), &manifest).unwrap();
    let pins = read_runtime_manifest(root, &pin).unwrap();
    assert_eq!(pins.len(), 3);
    assert_eq!(pins.last().unwrap().name, pin.name);
    let inputs = verify_windows_runtime(&pins, root.to_owned()).unwrap();
    inputs.recheck_runtime().unwrap();
    fs::write(root.join("extra"), b"not authorized").unwrap();
    assert!(inputs.recheck_runtime().is_err());
    fs::remove_file(root.join("extra")).unwrap();
    drop(inputs); // Windows retains no-write handles until the capability ends.
    let mut changed = manifest.clone();
    changed[0] ^= 1;
    fs::write(root.join(&pin.name), changed).unwrap();
    assert!(read_runtime_manifest(root, &pin).is_err());
    assert!(verify_windows_runtime(&pins, root.to_owned()).is_err());
}
#[test]
fn missing_manifest_and_untrusted_self_description_cannot_supply_runtime_pins() {
    let temp = tempfile::tempdir().unwrap();
    assert!(read_runtime_manifest(temp.path(), &runtime_manifest_pin()).is_err());
    fs::create_dir(temp.path().join("provenance")).unwrap();
    fs::write(
        temp.path().join("provenance/runtime-manifest.json"),
        br#"{"files":[],"sha256":"pretend"}"#,
    )
    .unwrap();
    assert!(read_runtime_manifest(temp.path(), &runtime_manifest_pin()).is_err());
}
#[test]
fn reopen_never_creates_or_repairs_missing_or_incomplete_installations() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("local-vm-v1");
    assert!(reopen(&root, "engineering-afc68ae1").unwrap().is_none());
    assert!(!root.exists());
    let journal = Journal::create(&root, BackendKind::Qemu, b"pc-q35-11.0".to_vec()).unwrap();
    drop(journal);
    let before = fs::read(root.join("record.json")).unwrap();
    assert!(reopen(&root, "engineering-afc68ae1").is_err());
    assert_eq!(fs::read(root.join("record.json")).unwrap(), before);
    assert_eq!(fs::metadata(root.join("state.disk")).unwrap().len(), 0);
    assert!(!root.join("image-binding.json").exists());
}
#[test]
fn factory_configuration_without_native_paths_cannot_activate_a_session() {
    assert_eq!(
        configured(None, None).status(),
        LocalVmStatus::NotIntegrated
    );
}
