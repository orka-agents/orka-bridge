use crate::runtime as fixture_runtime;
use crate::runtime::{apple_input::*, image_install::*, image_types::*, journal::Journal};
use std::{
    fs,
    io::{Read, Seek},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    sync::Arc,
};
mod fixture {
    include!("../../tests/support/runtime_image_fixture.rs");
}

pub(crate) fn installation() -> (tempfile::TempDir, Journal, Arc<InstalledImageSet>) {
    let temp = tempfile::tempdir().unwrap();
    let verified = fixture::verified(temp.path(), ImageTarget::MacosArm64);
    let journal = Journal::create(
        &temp.path().join("install"),
        BackendKind::Apple,
        vec![1, 2, 255],
    )
    .unwrap();
    let target = journal
        .image_target(&verified.image().identity().state_compatibility_id)
        .unwrap();
    let (_sender, closed) = tokio::sync::watch::channel(false);
    let image = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(materialize_image(&verified, &target, closed, |_| {}))
        .unwrap();
    (temp, journal, Arc::new(image))
}
#[test]
fn config_is_unlinked_private_rewound_and_uses_numeric_sdk_arrays() {
    let (temp, journal, image) = installation();
    let spec = journal.spec(bridge_runtime_protocol::Uuid::new_v4(), image);
    let before = fs::read_dir(temp.path().join("install")).unwrap().count();
    let mut file = configuration_file(&spec, true).unwrap();
    let metadata = file.metadata().unwrap();
    assert_eq!(metadata.mode() & 0o777, 0o600);
    assert_eq!(metadata.nlink(), 0);
    assert!(metadata.is_file());
    assert_eq!(file.stream_position().unwrap(), 0);
    assert_eq!(
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
        libc::FD_CLOEXEC
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert!(bytes.len() <= 65536);
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["machineId"], serde_json::json!([1, 2, 255]));
    assert_eq!(json["mac"], serde_json::json!(spec.mac));
    assert_eq!(
        json["osDiskPath"],
        spec.root.join(spec.image.disk_path()).to_str().unwrap()
    );
    assert_eq!(json["stateDiskPath"], spec.state_path.to_str().unwrap());
    assert_eq!(json["createEFI"], true);
    assert_eq!(json.as_object().unwrap().len(), 7);
    assert_eq!(
        fs::read_dir(temp.path().join("install")).unwrap().count(),
        before
    );
}
#[test]
fn expired_cleanup_retains_a_live_child_until_its_later_exit_is_reaped() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let reaper = identifier_waiter().unwrap();
    let mut child = Command::new("/bin/sh")
        .args(["-c", "read line"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let pid = child.id();
    assert!(matches!(
        reap_identifier(child, Instant::now(), reaper),
        Err(RuntimeError::RetainedState)
    ));
    // The direct child cannot exit until the test releases this input endpoint.
    drop(input);
    let end = Instant::now() + Duration::from_secs(1);
    let reaped = loop {
        let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) {
            break true;
        }
        if Instant::now() >= end {
            break false;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    if !reaped {
        // Reap only the test-owned, still-unreaped PID after the RED observation.
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid as i32, &mut status, 0) },
            pid as i32
        );
    }
    assert!(
        reaped,
        "bounded return discarded the only owner of a later-exiting child"
    );
}
#[test]
fn initializer_accepts_only_bounded_raw_success_and_reaps_exact_child() {
    use std::os::unix::fs::PermissionsExt;
    for (body, succeeds) in [
        ("printf '\\001\\002\\377'", true),
        ("exit 0", false),
        ("printf x; exit 7", false),
        ("head -c 4097 /dev/zero", false),
        ("sleep 2", false),
    ] {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("native")).unwrap();
        let executable = root.path().join("native/BridgeVM");
        fs::write(&executable, format!("#!/bin/sh\necho $$ > '{}'/pid\n[ \"$#\" = 1 ] && [ \"$1\" = --machine-identifier ] || exit 8\n[ -z \"$GH_TOKEN\" ] || exit 9\n{body}\n", root.path().display())).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let result = identifier_with_deadline(
            root.path(),
            std::time::Instant::now() + std::time::Duration::from_millis(100),
        );
        assert_eq!(result.is_ok(), succeeds, "{body}: {result:?}");
        if succeeds {
            assert_eq!(result.unwrap(), vec![1, 2, 255]);
        }
        let pid: i32 = fs::read_to_string(root.path().join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}
