use bridge_engine::runtime::{
    types::{BackendKind, LaunchSpec},
    windows::{inputs::*, launch::*},
};
use bridge_runtime_protocol::Uuid;
use serde_json::json;
use std::{fs, path::PathBuf};
use tempfile::TempDir;

use bridge_engine::runtime::windows::job::{create_verified, SpawnPolicy};

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

// Hand-checked SHA256 of "abc"; fixtures never contain an executable or guest disk.
const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
fn fixture() -> (TempDir, ReviewedWindowsPin, WindowsInputPaths) {
    let t = tempfile::tempdir().unwrap();
    let runtime = t.path().join("runtime");
    fs::create_dir_all(runtime.join("bin")).unwrap();
    for name in ["qemu-system-x86_64.exe", "qemu-img.exe", "glib.dll"] {
        fs::write(runtime.join("bin").join(name), b"abc").unwrap();
    }
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
        "runtimeFiles": [asset("bin/qemu-system-x86_64.exe"),asset("bin/qemu-img.exe"),asset("bin/glib.dll")],
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
#[test]
fn os_root_is_not_installation_uuid_and_launch_vector_is_fixed() {
    let (t, pin, paths) = fixture();
    let inputs = verify_windows_inputs(&pin, paths).unwrap();
    let root = t.path().join("install");
    fs::create_dir(&root).unwrap();
    let spec = LaunchSpec {
        installation_id: Uuid::new_v4(),
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
    let args = launch_argv(&inputs, &spec, "bridge-test").unwrap();
    let strs: Vec<_> = args.iter().map(|x| x.to_str().unwrap()).collect();
    let value = |key| strs[strs.iter().position(|x| *x == key).unwrap() + 1];
    assert_eq!(
        value("-append"),
        "root=UUID=22222222-2222-4222-8222-222222222222 ro console=ttyS0,115200n8 noresume"
    );
    assert_eq!(value("-accel"), "whpx");
    assert_eq!(value("-qmp"), "stdio");
    assert_eq!(value("-machine"), "pc-q35-11.0,usb=off");
    assert_eq!(value("-display"), "none");
    assert_eq!(value("-monitor"), "none");
    assert!(strs.contains(&"-S") && strs.contains(&"-no-reboot") && strs.contains(&"-nodefaults"));
    assert!(strs.contains(&"virtio-blk-pci,drive=os,serial=bridge-os"));
    assert!(strs.contains(&"virtio-blk-pci,drive=state,serial=bridge-state"));
    assert!(
        strs.contains(&"virtserialport,bus=control.0,chardev=guest,name=ai.orka.bridge.control")
    );
    assert!(!strs
        .iter()
        .any(|a| a.contains("pflash") || a.contains("tcg") || a.contains("wsl")));
    let mut wrong = spec.clone();
    wrong.backend = BackendKind::Apple;
    assert!(launch_argv(&inputs, &wrong, "bridge-test").is_err());
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
fn qcow2_backing_chain_wrong_capacity_or_dirty_image_refuses() {
    let good = br#"{"format":"qcow2","virtual-size":17179869184,"dirty-flag":false}"#;
    assert!(validate_qcow2_info(good, 17179869184).is_ok());
    for bad in [
        br#"{"format":"raw","virtual-size":17179869184}"#.as_slice(),
        br#"{"format":"qcow2","virtual-size":34359738368}"#,
        br#"{"format":"qcow2","virtual-size":17179869184,"backing-filename":"other"}"#,
        br#"{"format":"qcow2","virtual-size":17179869184,"dirty-flag":true}"#,
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
