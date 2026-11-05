use std::process::Command;
#[test]
fn help_and_invalid_modes_do_no_privileged_io() {
    let exe = env!("CARGO_BIN_EXE_bridge-guest");
    let help = Command::new(exe).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout).unwrap().contains("--early"));
    for args in [
        vec![],
        vec!["--serve", "/chosen-root"],
        vec!["--exec", "sh"],
    ] {
        let out = Command::new(exe).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.is_empty());
    }
}
#[test]
fn early_and_serve_fail_safely_before_privileged_io_for_nonroot() {
    // A root test runner must not execute either real privileged path.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    for (mode, stage) in [("--early", "early"), ("--serve", "serve")] {
        let out = Command::new(env!("CARGO_BIN_EXE_bridge-guest"))
            .arg(mode)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert_eq!(String::from_utf8(out.stderr).unwrap().lines().collect::<Vec<_>>(),[
            format!("bridge-guest: stage={stage} phase=euid-precheck kind=PermissionDenied errno=none"),
            format!("bridge-guest: stage={stage} phase=run kind=Other errno=none"),
            "bridge-guest: incompatible state or unavailable guest operation".to_owned(),
        ]);
    }
}
#[test]
fn invalid_arguments_are_not_echoed_in_diagnostics() {
    let secret = "synthetic-credential-do-not-log";
    let out = Command::new(env!("CARGO_BIN_EXE_bridge-guest"))
        .arg(secret)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains(secret));
    assert!(stderr.contains("stage=cli phase=arguments kind=InvalidInput errno=none"));
}
#[test]
fn actual_socket_peer_is_checked() {
    let (a, _) = std::os::unix::net::UnixStream::pair().unwrap();
    let uid = unsafe { libc::geteuid() };
    assert_eq!(bridge_guest::peer::check(&a).is_ok(), uid == 0);
}
