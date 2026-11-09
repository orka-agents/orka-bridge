use bridge_engine::runtime::{
    types::{BackendKind, LaunchSpec},
    windows::{inputs::*, launch::*},
};
use bridge_runtime_protocol::Uuid;
use serde_json::json;
use std::{fs, path::PathBuf};
use tempfile::TempDir;

use bridge_engine::runtime::windows::job::{create_verified, SpawnPolicy};
mod fixture_runtime {
    pub use bridge_engine::runtime::{image_cache, image_types, manifest};
}
#[path = "support/runtime_image_fixture.rs"]
mod runtime_image_fixture;
fn installed_fixture(
    root: &std::path::Path,
) -> (
    Uuid,
    std::sync::Arc<bridge_engine::runtime::image_install::InstalledImageSet>,
) {
    use bridge_engine::runtime::{
        image_install::materialize_image, image_types::ImageTarget, journal::Journal,
    };
    let journal = Journal::create(root, BackendKind::Qemu, b"pc-q35-11.0".to_vec()).unwrap();
    let source = root.join("inert-source");
    fs::create_dir(&source).unwrap();
    let verified = runtime_image_fixture::verified(&source, ImageTarget::WindowsAmd64);
    let target = journal
        .image_target(&verified.image().identity().state_compatibility_id)
        .unwrap();
    let (_sender, closed) = tokio::sync::watch::channel(false);
    let image = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(materialize_image(&verified, &target, closed, |_| {}))
        .unwrap();
    (journal.record().installation_id, std::sync::Arc::new(image))
}

#[test]
fn job_list_present_at_create_and_job_handle_does_not_inherit() {
    let p = SpawnPolicy::new(9, vec![1, 2, 3, 4]).unwrap();
    assert_eq!(p.jobs(), &[9]);
    assert_eq!(p.handles(), &[1, 2, 3, 4]);
    assert_eq!(p.flags() & 0x08080404, 0x08080404); // no-window, extended startup, Unicode, suspended
    assert!(SpawnPolicy::new(9, vec![1, 2, 3, 9]).is_err());
    assert!(SpawnPolicy::new(0, vec![1, 2, 3, 4]).is_err());
}
#[test]
fn sharing_and_lock_violations_are_busy_but_access_denied_is_not() {
    use bridge_engine::runtime::{types::RuntimeError, windows::job::owner_lock_error};
    for code in [32, 33] {
        assert!(matches!(
            owner_lock_error(std::io::Error::from_raw_os_error(code)),
            RuntimeError::Busy
        ));
    }
    assert!(matches!(
        owner_lock_error(std::io::Error::from_raw_os_error(5)),
        RuntimeError::Io(_)
    ));
}
#[test]
fn atomic_job_assignment_denial_has_no_child_or_fallback() {
    let p = SpawnPolicy::new(9, vec![1, 2, 3, 4]).unwrap();
    let calls = std::cell::Cell::new(0);
    let r = create_verified(
        &p,
        |_| {
            calls.set(calls.get() + 1);
            Err::<u32, _>(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        },
        |_| panic!("no child"),
        |_| panic!("no child"),
    );
    assert!(r.is_err());
    assert_eq!(calls.get(), 1);
}
#[test]
fn failed_membership_reaps_only_owned_child_before_any_resume() {
    let p = SpawnPolicy::new(9, vec![1, 2, 3, 4]).unwrap();
    let reaped = std::cell::Cell::new(0);
    let r = create_verified(
        &p,
        |_| Ok(42),
        |_| Ok(false),
        |c| {
            reaped.set(*c);
            Ok(())
        },
    );
    assert!(r.is_err());
    assert_eq!(reaped.get(), 42);
    assert_eq!(
        create_verified(
            &p,
            |_| Ok(42),
            |_| Ok(true),
            |_| panic!("successful child retained")
        )
        .unwrap(),
        42
    );
}

#[test]
fn unconfirmed_stop_quarantines_child_capability() {
    struct Child(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Child {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let released = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let p = SpawnPolicy::new(9, vec![1, 2, 3, 4]).unwrap();
    let r = create_verified(
        &p,
        |_| Ok(Child(released.clone())),
        |_| Ok(false),
        |_| Err(std::io::Error::other("stop not observed")),
    );
    assert!(r.is_err());
    assert!(!released.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn unconfirmed_teardown_retains_the_entire_capability_bundle() {
    use bridge_engine::runtime::windows::job::release_bundle;
    struct Capability(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Capability {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    for observed in [false, true] {
        let flags: Vec<_> = (0..4)
            .map(|_| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)))
            .collect();
        let bundle: Vec<_> = flags.iter().cloned().map(Capability).collect();
        release_bundle(bundle, observed);
        assert!(flags
            .iter()
            .all(|f| f.load(std::sync::atomic::Ordering::SeqCst) == observed));
    }
}
#[test]
fn concurrent_app_spawns_serialize_the_inheritable_handle_window() {
    use bridge_engine::runtime::windows::job::spawn_guard;
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let guard = spawn_guard().unwrap();
    let other = std::thread::spawn(move || {
        ready_tx.send(()).unwrap();
        let _guard = spawn_guard().unwrap();
        entered_tx.send(()).unwrap();
    });
    ready_rx.recv().unwrap();
    assert!(matches!(
        entered_rx.recv_timeout(std::time::Duration::from_millis(30)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    drop(guard);
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    other.join().unwrap();
}

// Hand-checked SHA256 of "abc"; fixtures never contain an executable or guest disk.
const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
fn fixture() -> (TempDir, ReviewedWindowsPin, WindowsInputPaths) {
    let t = tempfile::Builder::new()
        .prefix("wsl-fixture-")
        .tempdir()
        .unwrap();
    let runtime = t.path().join("runtime");
    fs::create_dir_all(runtime.join("bin")).unwrap();
    for name in ["qemu-system-x86_64.exe", "qemu-img.exe", "glib.dll"] {
        fs::write(runtime.join("bin").join(name), b"abc").unwrap();
    }
    fs::create_dir_all(runtime.join("share/qemu")).unwrap();
    fs::write(runtime.join("share/qemu/test-firmware.bin"), b"abc").unwrap();
    for name in ["os.kernel", "os.initrd", "factory.qcow2"] {
        fs::write(t.path().join(name), b"abc").unwrap();
    }
    let asset = |name| json!({"name":name,"bytes":3,"sha256":SHA});
    let mut provenance = serde_json::Map::new();
    for key in [
        "diskSha256",
        "bootReceiptSha256",
        "buildReceiptSha256",
        "inputSha256",
        "kernelConfigSha256",
        "modulesSha256",
        "endpointSha256",
        "earlyHookSha256",
        "buildHookSha256",
        "extractorSha256",
        "inspectorSha256",
    ] {
        provenance.insert(key.into(), json!(SHA));
    }
    let pin = serde_json::from_value(json!({
        "runtimeFiles": [asset("bin/qemu-system-x86_64.exe"),asset("bin/qemu-img.exe"),asset("bin/glib.dll"),asset("share/qemu/test-firmware.bin")],
        "image": asset("factory.qcow2"), "endpointSha256":SHA,
        "directBoot": { "kernel": {"name":"os.kernel","bytes":3,"sha256":SHA,"format":"linux-bzimage"},
            "initrd":{"name":"os.initrd","bytes":3,"sha256":SHA,"format":"initramfs","compression":"zstd"},
            "rootSelector":{"kind":"filesystem-uuid","value":"22222222-2222-4222-8222-222222222222"},
            "commandLine":"root=UUID=22222222-2222-4222-8222-222222222222 ro console=ttyS0,115200n8 noresume",
            "kernelRelease":"6.1.0-54-cloud-amd64", "provenance":provenance,
            "requiredDrivers":[{"name":"virtio_pci","builtIn":false},{"name":"virtio_blk","builtIn":false},{"name":"virtio_console","builtIn":false},{"name":"virtio_rng","builtIn":false},{"name":"virtio_net","builtIn":false},{"name":"ext4","builtIn":true}]
        }
    })).unwrap();
    let paths = WindowsInputPaths {
        runtime,
        kernel: t.path().join("os.kernel"),
        initrd: t.path().join("os.initrd"),
        factory: t.path().join("factory.qcow2"),
    };
    (t, pin, paths)
}
#[test]
fn runtime_reopen_does_not_require_the_original_source_image_files() {
    let (_temp, pin, paths) = fixture();
    for path in [&paths.kernel, &paths.initrd, &paths.factory] {
        fs::remove_file(path).unwrap();
    }
    let inputs = verify_windows_runtime(&pin.runtime_files, paths.runtime).unwrap();
    inputs.recheck_runtime().unwrap();
}
#[test]
fn initrd_compression_matches_the_publisher_contract() {
    let (_t, mut pin, paths) = fixture();
    for compression in ["gzip", "xz", "zstd", "cpio-prefix"] {
        pin.direct_boot.initrd.compression = Some(compression.into());
        assert!(
            verify_windows_inputs(&pin, paths.clone()).is_ok(),
            "{compression}"
        );
    }
    pin.direct_boot.initrd.compression = Some("lz4".into());
    assert!(verify_windows_inputs(&pin, paths).is_err());
}
#[test]
fn reject_mixed_boot_set() {
    let (_t, pin, paths) = fixture();
    assert!(verify_windows_inputs(&pin, paths.clone()).is_ok());
    for change in ["disk", "endpoint", "command", "driver", "format"] {
        let mut bad = pin.clone();
        match change {
            "disk" => {
                bad.direct_boot
                    .provenance
                    .insert("diskSha256".into(), "0".repeat(64));
            }
            "endpoint" => bad.endpoint_sha256 = "0".repeat(64),
            "command" => bad.direct_boot.command_line.push_str(" init=/bin/sh"),
            "driver" => {
                bad.direct_boot.required_drivers.pop();
            }
            _ => bad.direct_boot.kernel.format = "pe".into(),
        }
        assert!(
            verify_windows_inputs(&bad, paths.clone()).is_err(),
            "{change}"
        );
    }
    fs::write(&paths.initrd, b"xyz").unwrap();
    assert!(verify_windows_inputs(&pin, paths).is_err());
}
#[test]
fn runtime_exe_or_dll_changed_before_launch_refuses() {
    for name in [
        "qemu-system-x86_64.exe",
        "qemu-img.exe",
        "glib.dll",
        "extra.dll",
    ] {
        let (_t, pin, paths) = fixture();
        let verified = verify_windows_inputs(&pin, paths.clone()).unwrap();
        // Unix permits modifying an open file; native Windows deny-sharing is checked separately.
        #[cfg(not(windows))]
        fs::write(paths.runtime.join("bin").join(name), b"xyz").unwrap();
        #[cfg(windows)]
        {
            drop(verified);
            fs::write(paths.runtime.join("bin").join(name), b"xyz").unwrap();
            assert!(verify_windows_inputs(&pin, paths).is_err());
            continue;
        }
        #[cfg(not(windows))]
        assert!(verified.recheck_runtime().is_err(), "{name}");
    }
}
#[cfg(windows)]
#[test]
fn verbatim_dos_and_unc_values_keep_unicode_and_comma_escaping() {
    for (path, expected) in [
        (r"\\?\C:\runtime,é\share/qemu", r"C:\runtime,,é\share/qemu"),
        (
            r"\\?\UNC\host\share\runtime,é\share/qemu",
            r"\\host\share\runtime,,é\share/qemu",
        ),
        (r"C:\runtime,é\share/qemu", r"C:\runtime,,é\share/qemu"),
    ] {
        assert_eq!(qemu_value(std::path::Path::new(path)).unwrap(), expected);
    }
}

#[cfg(windows)]
#[test]
fn canonical_app_paths_reach_firmware_boot_files_and_disks_without_rust_normalization() {
    use std::{io::Read, os::windows::io::FromRawHandle, ptr};
    use windows_sys::Win32::{Foundation::INVALID_HANDLE_VALUE, Storage::FileSystem::*};
    unsafe extern "C" {
        fn _waccess(path: *const u16, mode: i32) -> i32;
    }
    // QEMU's firmware search uses CRT access(). Rust and CreateFileW alone
    // normalize paths more generously and would miss this failure.
    fn raw_open(path: &str) -> fs::File {
        let wide: Vec<_> = path.encode_utf16().chain(Some(0)).collect();
        assert_eq!(
            unsafe { _waccess(wide.as_ptr(), 4) },
            0,
            "QEMU cannot access {path:?}"
        );
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        assert_ne!(
            handle,
            INVALID_HANDLE_VALUE,
            "QEMU path {path:?}: {}",
            std::io::Error::last_os_error()
        );
        unsafe { fs::File::from_raw_handle(handle.cast()) }
    }
    let (t, pin, mut paths) = fixture();
    let canonical = fs::canonicalize(t.path()).unwrap();
    assert!(canonical.to_str().unwrap().starts_with(r"\\?\"));
    paths.runtime = canonical.join("runtime");
    let inputs = verify_windows_inputs(&pin, paths).unwrap();
    let root = canonical.join("install,é space");
    let (installation_id, image) = installed_fixture(&root);
    fs::write(root.join("boot.log"), b"").unwrap();
    let spec = LaunchSpec {
        installation_id,
        image,
        generation: 1,
        boot_nonce: Uuid::new_v4(),
        ownership: std::sync::Arc::new(fs::File::open(root.join("owner.lock")).unwrap()),
        state_path: root.join("state.disk"),
        root,
        state_bytes: 32 * 1024 * 1024 * 1024,
        backend: BackendKind::Qemu,
        machine_id: b"pc-q35-11.0".to_vec(),
        mac: [2, 0, 0, 0, 0, 1],
    };
    let socket = t.path().join("control.sock");
    let args = launch_argv(&inputs, &spec, socket.to_str().unwrap()).unwrap();
    let value = |key: &str| {
        args[args.iter().position(|a| a == key).unwrap() + 1]
            .to_str()
            .unwrap()
    };
    let mut firmware = Vec::new();
    raw_open(&format!("{}/test-firmware.bin", value("-L")))
        .read_to_end(&mut firmware)
        .unwrap();
    assert_eq!(firmware, b"abc");
    raw_open(value("-kernel"));
    raw_open(value("-initrd"));
    raw_open(value("-serial").strip_prefix("file:").unwrap());
    for pair in args.windows(2).filter(|pair| pair[0] == "-drive") {
        let path = pair[1]
            .to_str()
            .unwrap()
            .split_once("file=")
            .unwrap()
            .1
            .replace(",,", ",");
        raw_open(&path);
    }
}

#[test]
fn os_root_is_not_installation_uuid_and_launch_vector_is_fixed() {
    let (t, pin, paths) = fixture();
    let expected_data = paths.runtime.join("share/qemu");
    assert!(expected_data.is_dir());
    let inputs = verify_windows_inputs(&pin, paths).unwrap();
    let root = t.path().join("install,é space");
    let (installation_id, image) = installed_fixture(&root);
    let spec = LaunchSpec {
        installation_id,
        image,
        generation: 1,
        boot_nonce: Uuid::new_v4(),
        ownership: std::sync::Arc::new(fs::File::create(root.join("owner.lock")).unwrap()),
        state_path: root.join("state.disk"),
        root,
        state_bytes: 32 * 1024 * 1024 * 1024,
        backend: BackendKind::Qemu,
        machine_id: b"pc-q35-11.0".to_vec(),
        mac: [2, 0, 0, 0, 0, 1],
    };
    let socket = t.path().join("control.sock");
    let args = launch_argv(&inputs, &spec, socket.to_str().unwrap()).unwrap();
    let strs: Vec<_> = args.iter().map(|x| x.to_str().unwrap()).collect();
    let value = |key| strs[strs.iter().position(|x| *x == key).unwrap() + 1];
    assert_eq!(value("-append"), spec.image.command_line());
    assert_eq!(
        value("-serial"),
        format!("file:{}", spec.root.join("boot.log").display())
    );
    assert_eq!(value("-L"), expected_data.to_str().unwrap());
    let committed_os = spec.root.join(spec.image.disk_path());
    assert!(committed_os.is_file());
    assert_eq!(
        value("-drive"),
        format!(
            "if=none,id=os,format=qcow2,cache=writeback,file={}",
            qemu_value(&committed_os).unwrap()
        )
    );
    assert_eq!(
        value("-kernel"),
        spec.root.join(spec.image.kernel_path()).to_str().unwrap()
    );
    assert_eq!(
        value("-initrd"),
        spec.root.join(spec.image.initrd_path()).to_str().unwrap()
    );
    assert_eq!(value("-accel"), "whpx");
    assert_eq!(value("-qmp"), "stdio");
    assert_eq!(
        value("-chardev"),
        format!(
            "socket,id=guest,path={},server=on,wait=off",
            qemu_value(&socket).unwrap()
        )
    );
    assert!(!strs
        .iter()
        .any(|a| a.starts_with("pipe,id=guest") || a.contains("reconnect=")));
    assert_eq!(value("-machine"), "pc-q35-11.0,usb=off");
    assert_eq!(value("-display"), "none");
    assert_eq!(value("-monitor"), "none");
    assert!(strs.contains(&"-S") && strs.contains(&"-no-reboot") && strs.contains(&"-nodefaults"));
    assert!(strs.contains(&"virtio-blk-pci,drive=os,serial=bridge-os"));
    assert!(strs.contains(&"virtio-blk-pci,drive=state,serial=bridge-state"));
    assert!(
        strs.contains(&"virtserialport,bus=control.0,chardev=guest,name=ai.orka.bridge.control")
    );
    let raw_temp = t.path().to_str().unwrap();
    let option_text: Vec<_> = strs
        .iter()
        .map(|a| {
            a.replace(&qemu_value(t.path()).unwrap(), "")
                .replace(raw_temp, "")
        })
        .collect();
    assert!(!option_text
        .iter()
        .any(|a| a.contains("pflash") || a.contains("tcg") || a.contains("wsl")));
    let mut wrong = spec.clone();
    wrong.backend = BackendKind::Apple;
    assert!(launch_argv(&inputs, &wrong, socket.to_str().unwrap()).is_err());
}
#[test]
fn spaces_unicode_and_commas_are_literal() {
    assert_eq!(
        qemu_value(&PathBuf::from("C:/a,b/é space")).unwrap(),
        "C:/a,,b/é space"
    );
    assert_eq!(
        quote_windows_arg("a b\\\"c\\").unwrap(),
        "\"a b\\\\\\\"c\\\\\""
    );
    assert!(quote_windows_arg("x\0y").is_err());
}
#[test]
fn af_endpoint_paths_are_unique_utf8_and_bounded() {
    let root = tempfile::tempdir().unwrap();
    let first = control_path(root.path(), Uuid::new_v4()).unwrap();
    let second = control_path(root.path(), Uuid::new_v4()).unwrap();
    assert_ne!(first, second);
    assert_eq!(first.file_name().unwrap(), "s");
    assert_eq!(
        first
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .len(),
        34
    );
    let prefix = root.path().to_str().unwrap().len() + 1;
    let at_limit = root.path().join("x".repeat(107 - prefix));
    assert!(validate_control_path(&at_limit).is_ok());
    let too_long = root.path().join("x".repeat(108 - prefix));
    assert!(validate_control_path(&too_long)
        .unwrap_err()
        .to_string()
        .contains("107 bytes"));
    assert!(validate_control_path(&root.path().join("é")).is_ok());
    assert!(validate_control_path(&PathBuf::from("relative.sock")).is_err());
    assert!(validate_control_path(&root.path().join("bad\0socket")).is_err());
}
#[cfg(windows)]
#[test]
fn af_endpoint_rejects_remote_paths_and_normalizes_local_verbatim_prefix() {
    assert!(validate_control_path(&PathBuf::from(r"\\server\share\s")).is_err());
    assert_eq!(
        validate_control_path(&PathBuf::from(r"\\?\C:\private\s")).unwrap(),
        PathBuf::from(r"C:\private\s")
    );
}
#[test]
fn qcow2_backing_chain_wrong_capacity_or_dirty_image_refuses() {
    let good = br#"{"format":"qcow2","virtual-size":17179869184,"dirty-flag":false}"#;
    assert!(validate_qcow2_info(good, 17179869184).is_ok());
    for bad in [
        br#"{"format":"raw","virtual-size":17179869184}"#.as_slice(),
        br#"{"format":"qcow2","virtual-size":34359738368}"#,
        br#"{"format":"qcow2","virtual-size":17179869184,"backing-filename":"other"}"#,
        br#"{"format":"qcow2","virtual-size":17179869184,"dirty-flag":true}"#,
        br#"{"format":"qcow2","virtual-size":17179869184,"format-specific":{"data":{"data-file":"../external.raw"}}}"#,
        br#"{"format":"qcow2","virtual-size":17179869184,"format-specific":{"data":{"data-file-raw":true}}}"#,
    ] {
        assert!(validate_qcow2_info(bad, 17179869184).is_err());
    }
}
#[test]
fn feature_enabled_is_not_whpx_usable() {
    let d = Prerequisites {
        feature: Observation::Yes,
        reboot_pending: Observation::No,
        hypervisor: Observation::Yes,
        usability: Usability::NotTested,
    };
    assert!(!d.usable());
    assert!(!Prerequisites {
        usability: Usability::Unusable,
        ..d
    }
    .usable());
    assert!(!Prerequisites {
        feature: Observation::Unknown,
        usability: Usability::NotTested,
        ..d
    }
    .usable());
    assert!(!Prerequisites {
        reboot_pending: Observation::Yes,
        usability: Usability::Usable,
        ..d
    }
    .usable());
    assert!(Prerequisites {
        usability: Usability::Usable,
        ..d
    }
    .usable());
}
