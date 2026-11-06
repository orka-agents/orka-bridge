use crate::runtime::spawn_guard::{run_blocking, tests::HeldGate};

pub(crate) fn descriptor_window() -> (HeldGate, tempfile::TempDir, std::path::PathBuf) {
    use std::{os::fd::AsRawFd, os::unix::net::UnixStream, process::Command, sync::OnceLock};
    static PROBE: OnceLock<tempfile::TempDir> = OnceLock::new();
    let binary = PROBE
        .get_or_init(|| {
            let d = tempfile::tempdir().unwrap();
            let source = d.path().join("probe.rs");
            std::fs::write(
                &source,
                include_str!("../../tests/support/fd_probe_fixture.rs"),
            )
            .unwrap();
            let mut compiler = std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        run_blocking(|| {
                            Command::new("rustc")
                                .arg("--edition=2021")
                                .arg(source)
                                .arg("-o")
                                .arg(d.path().join("probe"))
                                .spawn()
                        })
                    })
                    .join()
                    .unwrap()
            })
            .unwrap();
            assert!(compiler.wait().unwrap().success());
            d
        })
        .path()
        .join("probe");
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_owned();
    let (ready, entered) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        run_blocking(|| {
            let (a, b) = UnixStream::pair().unwrap();
            let fd = b.as_raw_fd();
            // Deterministically recreate Darwin's non-atomic CLOEXEC setup window,
            // even on Linux where socketpair normally sets it atomically.
            assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFD, 0) }, 0);
            let path = root.join(format!("probe-{fd}"));
            std::fs::hard_link(binary, &path).unwrap();
            ready.send(path).unwrap();
            let _ = wait.recv();
            assert_eq!(
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
                0
            );
            drop((a, b));
        })
    });
    let executable = entered.recv().unwrap();
    (
        HeldGate {
            release: Some(release),
            thread: Some(thread),
        },
        directory,
        executable,
    )
}
