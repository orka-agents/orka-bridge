use super::*;
use sha2::{Digest, Sha256};
use std::fs;

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
mod fixture_runtime {
    pub use crate::runtime::{image_cache, image_types, manifest};
}
#[cfg(all(debug_assertions, windows))]
#[path = "../tests/support/runtime_image_fixture.rs"]
mod fixture;

#[cfg(all(debug_assertions, windows))]
#[tokio::test]
async fn windows_session_startup_inside_tokio_reaches_child_creation_without_panicking() {
    use crate::runtime::{
        image_install::materialize_image, image_types::ImageTarget, RuntimeWorker,
    };
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
    let result = blocking_start(move |_| {
        let worker = RuntimeWorker::new(
            Box::new(crate::runtime::windows::backend::WindowsBackend::new(
                inputs,
            )),
            journal,
            Arc::new(image),
        )?;
        let session = crate::runtime::windows::session::WindowsSession::start(worker, 0)?;
        Ok(Box::new(NativeSession(session)) as Box<dyn LocalSession>)
    })
    .await;
    // The real backend reaches CreateProcessW, which refuses the inert fixture.
    // A panic is mapped to Unavailable instead, so it cannot satisfy this check.
    match result {
        Err(RuntimeError::Io(error)) => assert!(
            matches!(error.raw_os_error(), Some(193 | 216)),
            "expected ERROR_BAD_EXE_FORMAT or ERROR_EXE_MACHINE_TYPE_MISMATCH: {error:?}"
        ),
        Err(error) => panic!("expected native executable refusal, got {error:?}"),
        Ok(_) => panic!("inert fixture must not start a session"),
    }
    assert_eq!(fs::metadata(root.join("state.disk")).unwrap().len(), 0);
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
