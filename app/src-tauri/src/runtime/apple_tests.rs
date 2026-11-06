use super::*;
use std::{io::Read, os::fd::AsRawFd, sync::OnceLock};

fn fixture() -> &'static std::path::Path {
    static FIXTURE: OnceLock<tempfile::TempDir> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let d = tempfile::tempdir().unwrap();
            let source = d.path().join("helper.rs");
            std::fs::write(
                &source,
                include_str!("../../tests/support/apple_helper_fixture.rs"),
            )
            .unwrap();
            let mut compiler = super::super::spawn_guard::run_blocking(|| {
                Command::new("rustc")
                    .arg("--edition=2021")
                    .arg(source)
                    .arg("-o")
                    .arg(d.path().join("helper"))
                    .spawn()
            })
            .unwrap();
            assert!(compiler.wait().unwrap().success());
            d
        })
        .path()
}
fn setup(mode: &str) -> (tempfile::TempDir, AppleHelperOwner, Command) {
    let d = tempfile::tempdir().unwrap();
    let file = File::create(d.path().join("lock")).unwrap();
    file.try_lock().unwrap();
    let mut owner = AppleHelperOwner::new(Arc::new(file));
    owner.budgets = Budgets {
        read: Duration::from_secs(2),
        write: Duration::from_secs(1),
        exit: Duration::from_millis(40),
        cleanup: Duration::from_millis(40),
    };
    let mut cmd = Command::new(fixture().join("helper"));
    cmd.arg(mode)
        .arg(d.path().join("pid"))
        .env("GH_TOKEN", "synthetic-not-an-account");
    (d, owner, cmd)
}
fn assert_reaped(d: &tempfile::TempDir) {
    let pid: i32 = std::fs::read_to_string(d.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
        -1,
        "owned child was not reaped"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

#[test]
fn helper_owns_only_selected_channels_and_cannot_inherit_ambient_tokens() {
    let (d, mut owner, cmd) = setup("normal");
    let parent_flags = unsafe { libc::fcntl(owner.ownership.as_raw_fd(), libc::F_GETFD) };
    let mut guest = owner.start(cmd).unwrap();
    let mut bytes = [0; 11];
    guest.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"guest-only\n");
    owner.pause().unwrap();
    owner.resume().unwrap();
    assert_eq!(
        unsafe { libc::fcntl(owner.ownership.as_raw_fd(), libc::F_GETFD) },
        parent_flags
    );
    owner.stop().unwrap();
    assert_reaped(&d);
}
#[test]
fn supervisor_eof_reaches_helper_without_any_termination_signal() {
    let (d, mut owner, cmd) = setup("normal");
    let _guest = owner.start(cmd).unwrap();
    let session = owner.session.as_mut().unwrap();
    session.control = None;
    assert!(
        wait_until(&mut session.child, Instant::now() + Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .success()
    );
    owner.session = None;
    assert_reaped(&d);
}

#[test]
fn idle_pause_and_resume_do_not_poison_a_safe_owner() {
    let (d, mut owner, cmd) = setup("normal");
    let disconnected = HelperError::Io(std::io::ErrorKind::NotConnected);
    assert_eq!(owner.pause().unwrap_err(), disconnected);
    assert_eq!(owner.resume().unwrap_err(), disconnected);
    let _guest = owner.start(cmd).unwrap();
    owner.stop().unwrap();
    assert_eq!(owner.pause().unwrap_err(), disconnected);
    assert_eq!(owner.resume().unwrap_err(), disconnected);
    let mut again = Command::new(fixture().join("helper"));
    again.arg("normal").arg(d.path().join("pid"));
    let _next_guest = owner.start(again).unwrap();
    owner.stop().unwrap();
    assert_reaped(&d);
}

#[test]
fn guest_parent_endpoint_is_not_inherited_by_the_helper() {
    let (d, mut owner, cmd) = setup("guest-eof");
    let mut guest = owner.start(cmd).unwrap();
    let mut bytes = [0; 11];
    guest.read_exact(&mut bytes).unwrap();
    drop(guest);
    // Fixture cannot acknowledge until it observes guest-peer EOF; unlike Drop,
    // no TERM/KILL can make a hidden inherited parent endpoint pass this test.
    owner.pause().unwrap();
    owner.stop().unwrap();
    assert_reaped(&d);
}

#[test]
fn busy_owner_never_starts_another_process() {
    let (d, mut owner, cmd) = setup("normal");
    let _guest = owner.start(cmd).unwrap();
    assert_eq!(
        owner.start(Command::new("/must-not-execute")).unwrap_err(),
        HelperError::Busy
    );
    owner.stop().unwrap();
    assert_reaped(&d);
}
#[test]
fn bad_startup_permanently_poisons_owner_before_retry() {
    let (d, mut owner, cmd) = setup("bad-ready");
    assert_eq!(owner.start(cmd).unwrap_err(), HelperError::Protocol);
    assert_eq!(
        owner.start(Command::new("/must-not-execute")).unwrap_err(),
        HelperError::Poisoned
    );
    assert_reaped(&d);
}
#[test]
fn mismatched_reply_and_helper_failure_poison_without_replay() {
    for mode in ["stale", "error"] {
        let (d, mut owner, cmd) = setup(mode);
        let _guest = owner.start(cmd).unwrap();
        assert_eq!(owner.pause().unwrap_err(), HelperError::Protocol);
        assert_eq!(owner.resume().unwrap_err(), HelperError::Poisoned);
        assert_eq!(
            owner.start(Command::new("/must-not-execute")).unwrap_err(),
            HelperError::Poisoned
        );
        assert_reaped(&d);
    }
}
#[test]
fn response_fragments_do_not_restart_the_absolute_deadline() {
    let (d, mut owner, cmd) = setup("fragment");
    let _guest = owner.start(cmd).unwrap();
    owner.budgets.read = Duration::from_millis(35);
    assert_eq!(
        owner.pause().unwrap_err(),
        HelperError::Io(std::io::ErrorKind::TimedOut)
    );
    assert_eq!(owner.resume().unwrap_err(), HelperError::Poisoned);
    assert_reaped(&d);
}
#[test]
fn stop_requires_observed_successful_exit_not_only_an_ack() {
    for mode in ["exit-bad", "hang-stop"] {
        let (d, mut owner, cmd) = setup(mode);
        let _guest = owner.start(cmd).unwrap();
        assert!(owner.stop().is_err());
        assert_eq!(
            owner.start(Command::new("/must-not-execute")).unwrap_err(),
            HelperError::Poisoned
        );
        assert_reaped(&d);
    }
}
#[test]
fn dropping_live_owner_reaps_before_releasing_its_lock() {
    let (d, mut owner, cmd) = setup("normal");
    let _guest = owner.start(cmd).unwrap();
    let probe = File::open(d.path().join("lock")).unwrap();
    assert!(probe.try_lock().is_err());
    drop(owner);
    assert_reaped(&d);
    // Another app spawn may transiently inherit even CLOEXEC files between fork
    // and exec. Check release after in-flight guarded spawns cross that boundary.
    super::super::spawn_guard::run_blocking(|| probe.try_lock().unwrap());
}
#[test]
fn unobserved_start_exit_is_distinct_and_drop_retains_lock_without_retry() {
    let (d, mut owner, cmd) = setup("unobserved");
    // No observation budget: still an actual child, not a mocked wait result.
    // The fixture sends a bad header only after installing TERM-ignore and its PID receipt.
    owner.budgets.cleanup = Duration::ZERO;
    assert_eq!(owner.start(cmd).unwrap_err(), HelperError::ExitUnobserved);
    assert_eq!(
        owner.start(Command::new("/must-not-execute")).unwrap_err(),
        HelperError::ExitUnobserved
    );
    drop(owner);
    let probe = File::open(d.path().join("lock")).unwrap();
    assert!(
        probe.try_lock().is_err(),
        "Drop released retained installation ownership"
    );
    // Reap the exact test-owned child which the zero-budget observer retained.
    // A real unobserved exit keeps its capability quarantined until app exit.
    let pid: i32 = std::fs::read_to_string(d.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    unsafe {
        libc::kill(pid, libc::SIGKILL);
        let mut status = 0;
        assert_eq!(libc::waitpid(pid, &mut status, 0), pid);
    }
    assert!(
        probe.try_lock().is_err(),
        "quarantined parent capability was not retained"
    );
}
