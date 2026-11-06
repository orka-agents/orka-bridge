#[cfg(all(windows, target_env = "msvc", not(target_feature = "crt-static")))]
compile_error!("probe must use static CRT");
#[cfg(windows)]
include!(concat!(env!("OUT_DIR"), "/source.rs"));
#[cfg(not(windows))]
fn main() {
    eprintln!("BLOCKED: native Windows required");
    std::process::exit(2)
}
#[cfg(windows)]
fn main() {
    if let Err(e) = native::run() {
        eprintln!("FAIL/BLOCKED: {e}");
        std::process::exit(1)
    }
}
#[cfg(windows)]
mod native {
    #[path = "controller.rs"]
    mod controller;
    #[path = "loader_trace.rs"]
    mod loader_trace;
    #[path = "permission_diag.rs"]
    mod permission_diag;
    use crate::runtime::{
        self,
        windows::{inputs::*, job::OwnedQemuProcess},
    };
    use serde::Deserialize;
    use serde_json::{json, Value};
    use std::{
        ffi::OsString,
        fs::{self, File},
        io::{self, BufRead, BufReader, Read, Write},
        mem::{size_of, zeroed},
        os::windows::io::{AsRawHandle, FromRawHandle},
        path::PathBuf,
        sync::{mpsc, Arc},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::*,
        Storage::FileSystem::*,
        System::{Diagnostics::ProcessSnapshotting::*, Pipes::*, Threading::*},
    };
    fn check(b: bool) -> io::Result<()> {
        if b {
            Ok(())
        } else {
            Err(io::Error::other("native premise failed"))
        }
    }
    fn win(b: i32) -> io::Result<()> {
        if b == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    #[derive(Deserialize)]
    struct Paths {
        runtime: PathBuf,
        kernel: PathBuf,
        initrd: PathBuf,
        factory: PathBuf,
    }
    #[derive(Deserialize)]
    struct Config {
        head: String,
        pin: ReviewedWindowsPin,
        paths: Paths,
    }
    fn emit(v: Value) -> io::Result<()> {
        println!("{v}");
        io::stdout().flush()
    }
    struct Snapshot(HPSS, HPSSWALK);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            unsafe {
                let marker = if self.1.is_null() {
                    0
                } else {
                    PssWalkMarkerFree(self.1)
                };
                let snapshot = PssFreeSnapshot(GetCurrentProcess(), self.0);
                if marker != 0 || snapshot != 0 {
                    eprintln!(
                        "FAIL/BLOCKED: snapshot release marker={marker}, snapshot={snapshot}"
                    );
                    std::process::abort();
                }
            }
        }
    }
    unsafe fn text(p: *const u16, n: u16) -> io::Result<String> {
        check(n.is_multiple_of(2) && (n == 0 || !p.is_null()))?;
        if n == 0 {
            Ok(String::new())
        } else {
            String::from_utf16(std::slice::from_raw_parts(p, n as usize / 2))
                .map(|s| s.trim_end_matches('\0').to_owned())
                .map_err(io::Error::other)
        }
    }
    fn handle_snapshot(
        process: HANDLE,
        expected: &runtime::identity::FileIdentity,
        canary: &runtime::identity::FileIdentity,
    ) -> io::Result<Value> {
        unsafe {
            let mut snap = std::ptr::null_mut();
            let e = PssCaptureSnapshot(
                process,
                PSS_CAPTURE_HANDLES
                    | PSS_CAPTURE_HANDLE_BASIC_INFORMATION
                    | PSS_CAPTURE_HANDLE_NAME_INFORMATION,
                0,
                &mut snap,
            );
            if e != 0 {
                return Err(io::Error::from_raw_os_error(e as i32));
            }
            let mut s = Snapshot(snap, std::ptr::null_mut());
            let e = PssWalkMarkerCreate(std::ptr::null(), &mut s.1);
            if e != 0 {
                return Err(io::Error::from_raw_os_error(e as i32));
            }
            let mut found = 0;
            let mut jobs = 0;
            let mut canaries = 0;
            for index in 0..8192 {
                let mut entry: PSS_HANDLE_ENTRY = zeroed();
                let e = PssWalkSnapshot(
                    s.0,
                    PSS_WALK_HANDLES,
                    s.1,
                    (&mut entry as *mut PSS_HANDLE_ENTRY).cast(),
                    size_of::<PSS_HANDLE_ENTRY>() as u32,
                );
                if e == ERROR_NO_MORE_ITEMS {
                    return Ok(
                        json!({"guardIdentity":expected,"guardCopies":found,"inheritedJobs":jobs,"unrelatedCanaries":canaries}),
                    );
                }
                if e != 0 {
                    return Err(io::Error::from_raw_os_error(e as i32));
                }
                // HAVE_TYPE validates ObjectType's small enum, not TypeName.
                // Files and Jobs are not in that enum. PSS can omit File
                // names entirely; classify the type, then query actual file IDs.
                let kind = text(entry.TypeName, entry.TypeNameLength)?;
                let name = if entry.Flags & PSS_HANDLE_HAVE_NAME != 0 {
                    text(entry.ObjectName, entry.ObjectNameLength)?
                } else {
                    String::new()
                };
                if index < 12
                    || name.to_lowercase().ends_with("\\owner.lock")
                    || name.to_lowercase().ends_with("\\unrelated-canary")
                {
                    eprintln!(
                        "PSS entry flags={:#x} enum={} kind={kind:?} name={name:?}",
                        entry.Flags, entry.ObjectType
                    );
                }
                if kind.is_empty() {
                    return Err(io::Error::other("snapshot lacks object type name"));
                }
                if kind == "Job" {
                    jobs += 1
                }
                if kind == "File" {
                    let mut dup = std::ptr::null_mut();
                    win(DuplicateHandle(
                        process,
                        entry.Handle,
                        GetCurrentProcess(),
                        &mut dup,
                        0,
                        0,
                        DUPLICATE_SAME_ACCESS,
                    ))?;
                    let file = File::from_raw_handle(dup);
                    match GetFileType(file.as_raw_handle()) {
                        FILE_TYPE_DISK => {
                            let id = runtime::identity::file_identity(&file)
                                .map_err(io::Error::other)?;
                            if id == *expected {
                                found += 1;
                            }
                            if id == *canary {
                                canaries += 1;
                            }
                        }
                        FILE_TYPE_PIPE | FILE_TYPE_CHAR => {}
                        _ => return Err(io::Error::other("snapshot File kind unavailable")),
                    }
                }
            }
            Err(io::Error::other("snapshot bound reached"))
        }
    }
    fn inherited_guard(process: HANDLE, original: &File, canary: &File) -> io::Result<Value> {
        let expected = runtime::identity::file_identity(original).map_err(io::Error::other)?;
        let canary = runtime::identity::file_identity(canary).map_err(io::Error::other)?;
        let v = handle_snapshot(process, &expected, &canary)?;
        if v["guardCopies"] != 1 || v["inheritedJobs"] != 0 || v["unrelatedCanaries"] != 0 {
            return Err(io::Error::other(format!(
                "inherited inventory differs: {v}"
            )));
        }
        Ok(v)
    }
    fn thread_state(process: HANDLE, label: &str) -> io::Result<()> {
        unsafe {
            let mut raw = std::ptr::null_mut();
            let status = PssCaptureSnapshot(process, PSS_CAPTURE_THREADS, 0, &mut raw);
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            let mut snapshot = Snapshot(raw, std::ptr::null_mut());
            let status = PssWalkMarkerCreate(std::ptr::null(), &mut snapshot.1);
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            for _ in 0..256 {
                let mut thread: PSS_THREAD_ENTRY = zeroed();
                let status = PssWalkSnapshot(
                    snapshot.0,
                    PSS_WALK_THREADS,
                    snapshot.1,
                    (&mut thread as *mut PSS_THREAD_ENTRY).cast(),
                    size_of::<PSS_THREAD_ENTRY>() as u32,
                );
                if status == ERROR_NO_MORE_ITEMS {
                    return Ok(());
                }
                if status != 0 {
                    return Err(io::Error::from_raw_os_error(status as i32));
                }
                eprintln!(
                    "QEMU thread {label}: id={}, suspended={}, exit={:#x}",
                    thread.ThreadId, thread.SuspendCount, thread.ExitStatus
                );
            }
            Err(io::Error::other("thread snapshot bound"))
        }
    }
    fn creation_time(process: HANDLE) -> io::Result<u64> {
        unsafe {
            let (mut c, mut e, mut k, mut u) = (zeroed(), zeroed(), zeroed(), zeroed());
            win(GetProcessTimes(process, &mut c, &mut e, &mut k, &mut u))?;
            Ok(((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64)
        }
    }
    fn connect(name: &str, process: HANDLE) -> io::Result<File> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let path = wide(&format!("\\\\.\\pipe\\{name}"));
        let mut last_retry = 0;
        loop {
            unsafe {
                check(WaitForSingleObject(process, 0) == WAIT_TIMEOUT)?;
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("pipe connect deadline; last retry Win32={last_retry}"),
                    ));
                }
                let h = CreateFileW(
                    path.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                );
                if h != INVALID_HANDLE_VALUE {
                    let f = File::from_raw_handle(h);
                    let (mut pid, mut instances) = (0, 0);
                    win(GetNamedPipeServerProcessId(h, &mut pid)).map_err(|e| {
                        io::Error::other(format!("GetNamedPipeServerProcessId: {e}"))
                    })?;
                    win(GetNamedPipeInfo(
                        h,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        &mut instances,
                    ))
                    .map_err(|e| io::Error::other(format!("GetNamedPipeInfo: {e}")))?;
                    check(pid == GetProcessId(process) && instances == 1)?;
                    permission_diag::pipe_security(h);
                    runtime::windows::check_pipe(h).map_err(|e| {
                        io::Error::other(format!("source private_object check: {e}"))
                    })?;
                    check(WaitForSingleObject(process, 0) == WAIT_TIMEOUT)?;
                    return Ok(f);
                }
                let e = io::Error::last_os_error();
                if !matches!(e.raw_os_error(), Some(2 | 231)) {
                    return Err(e);
                }
                last_retry = e.raw_os_error().unwrap();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn reader(file: File) -> mpsc::Receiver<io::Result<Value>> {
        let (tx, rx) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            let mut b = BufReader::new(file);
            loop {
                let mut line = Vec::new();
                let r = (&mut b)
                    .take(131073)
                    .read_until(b'\n', &mut line)
                    .and_then(|n| {
                        check(n > 0 && n <= 131072 && line.last() == Some(&b'\n'))?;
                        serde_json::from_slice(&line).map_err(io::Error::other)
                    });
                let failed = r.is_err();
                if tx.send(r).is_err() || failed {
                    break;
                }
            }
        });
        rx
    }
    fn receive(rx: &mpsc::Receiver<io::Result<Value>>, id: Option<u64>) -> io::Result<Value> {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            let v = rx
                .recv_timeout(until.saturating_duration_since(Instant::now()))
                .map_err(io::Error::other)??;
            if let Some(id) = id {
                if v.get("event").is_some() {
                    continue;
                }
                check(v["id"] == id && v.get("return").is_some() && v.get("error").is_none())?;
            } else {
                check(v["QMP"].is_object())?;
            }
            return Ok(v);
        }
    }
    pub fn run() -> io::Result<()> {
        let args: Vec<_> = std::env::args().collect();
        if args.get(1).map(String::as_str) == Some("check") {
            return controller::run(&args[2..]);
        }
        check(args.len() == 4)?;
        let mode = &args[1];
        let cfg: Config = serde_json::from_slice(&fs::read(&args[2])?).map_err(io::Error::other)?;
        check(cfg.head == crate::SOURCE_HEAD)?;
        let root = PathBuf::from(&args[3]);
        if mode == "check-free" {
            let _j = runtime::journal::Journal::open(&root).map_err(io::Error::other)?;
            return emit(json!({"sourceHead":crate::SOURCE_HEAD,"lock":"free"}));
        }
        if mode == "check-busy" {
            check(matches!(
                runtime::journal::Journal::open(&root),
                Err(runtime::types::RuntimeError::Busy)
            ))?;
            return emit(json!({"sourceHead":crate::SOURCE_HEAD,"lock":"busy"}));
        }
        check(["smoke", "hold-created", "hold-pipe", "hold-guard-only"].contains(&mode.as_str()))?;
        let diagnostic_runtime = cfg.paths.runtime.clone();
        let paths = WindowsInputPaths {
            runtime: cfg.paths.runtime,
            kernel: cfg.paths.kernel,
            initrd: cfg.paths.initrd,
            factory: cfg.paths.factory,
        };
        let inputs = Arc::new(verify_windows_inputs(&cfg.pin, paths).map_err(io::Error::other)?);
        let journal = runtime::journal::Journal::create(
            &root,
            runtime::types::BackendKind::Qemu,
            b"pc-q35-11.0".to_vec(),
        )
        .map_err(io::Error::other)?;
        let spec = runtime::probe_spec(&journal);
        let name = format!("bridge-premise-{}", bridge_runtime_protocol::Uuid::new_v4());
        let mut argv: Vec<OsString> = [
            "--preconfig",
            "-machine",
            "none",
            "-accel",
            "qtest",
            "-S",
            "-nodefaults",
            "-no-user-config",
            "-display",
            "none",
            "-vga",
            "none",
            "-monitor",
            "none",
            "-qmp",
            "stdio",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        argv.extend([
            OsString::from("-chardev"),
            format!("pipe,id=guest,path={name}").into(),
        ]);
        // Reuse the production serial argument to exercise literal comma/Unicode paths.
        let production = runtime::windows::launch::launch_argv(&inputs, &spec, &name)
            .map_err(io::Error::other)?;
        let at = production
            .iter()
            .position(|a| a == "-serial")
            .ok_or_else(|| io::Error::other("serial missing"))?;
        argv.extend([OsString::from("-serial"), production[at + 1].clone()]);
        // Deliberately inheritable unrelated file: HANDLE_LIST must exclude it.
        let canary = fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(root.join("unrelated-canary"))?;
        unsafe {
            win(SetHandleInformation(
                canary.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            ))?;
        }
        // Test diagnostics: loader faults must report an exit, not hang on a
        // modal dialog on an unattended desktop. This cannot turn a fault into PASS.
        unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::SetErrorMode(0x8003);
        }
        eprintln!("Fixed no-VM QEMU argv: {argv:?}");
        let mut child =
            OwnedQemuProcess::spawn_suspended(inputs, spec.ownership.clone(), &root, &argv)?;
        let process = child.process_handle();
        runtime::windows::check_token(process.raw())
            .map_err(|e| io::Error::other(format!("restricted primary token: {e}")))?;
        let inherited = inherited_guard(process.raw(), &spec.ownership, &canary)
            .map_err(|e| io::Error::other(format!("inherited handle snapshot: {e}")))?;
        if mode == "smoke" {
            if let Err(e) = permission_diag::run(process.raw(), &diagnostic_runtime) {
                eprintln!("[LOADER-DIAG] measurement unavailable: {e}");
            }
        }
        // Drain diagnostics and pump QMP immediately, including during pipe wait.
        // The greeting is still consumed only AFTER pipe peer/DACL validation.
        let (mut stdin, stdout, mut stderr) = child.take_stdio()?;
        let errors = Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = errors.clone();
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(n) = stderr.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                let mut out = capture.lock().unwrap();
                let take = n.min(65536usize.saturating_sub(out.len()));
                out.extend_from_slice(&buffer[..take]);
            }
        });
        let rx = reader(stdout);
        let traced = loader_trace::enabled()?;
        if traced {
            loader_trace::start(process.clone())?;
        }
        thread_state(process.raw(), "before-resume")?;
        if mode != "hold-created" {
            child.resume()?;
            thread_state(process.raw(), "after-resume")?;
        }
        if mode == "hold-pipe" {
            let until = Instant::now() + Duration::from_secs(30);
            let path = wide(&format!("\\\\.\\pipe\\{name}"));
            loop {
                if unsafe { WaitNamedPipeW(path.as_ptr(), 1) } != 0 {
                    break;
                }
                check(!child.exited() && Instant::now() < until)?;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let mut status = json!({"sourceHead":crate::SOURCE_HEAD,"pid":unsafe{GetProcessId(process.raw())},"creationTime":creation_time(process.raw())?,"mode":mode,"pipe":name,"root":root,"inherited":inherited,"restrictedPrimary":true,"whpxUsability":"not-tested"});
        if mode == "hold-guard-only" {
            let identity =
                runtime::identity::file_identity(&spec.ownership).map_err(io::Error::other)?;
            let canary_id = runtime::identity::file_identity(&canary).map_err(io::Error::other)?;
            let positive = handle_snapshot(unsafe { GetCurrentProcess() }, &identity, &canary_id)?;
            check(
                positive["guardCopies"] == 1
                    && positive["unrelatedCanaries"] == 1
                    && positive["inheritedJobs"].as_u64().is_some_and(|n| n >= 1),
            )?;
            let original = spec.ownership.as_raw_handle();
            // Fault injection in this disposable worker only. Every Rust owner is leaked
            // before closing its ONE known shared HANDLE; none may be touched/dropped later.
            // The leaked Job deliberately stays alive until this worker is killed.
            std::mem::forget(child);
            std::mem::forget(spec);
            std::mem::forget(journal);
            unsafe {
                win(CloseHandle(original))?;
            }
            let after = handle_snapshot(unsafe { GetCurrentProcess() }, &identity, &canary_id)?;
            check(
                after["guardCopies"] == 0
                    && after["unrelatedCanaries"] == 1
                    && after["inheritedJobs"].as_u64().is_some_and(|n| n >= 1),
            )?;
            check(handle_snapshot(process.raw(), &identity, &canary_id)?["guardCopies"] == 1)?;
            status["parentSnapshotBeforeClose"] = positive;
            status["parentSnapshotAfterClose"] = after;
            status["parentGuardHandles"] = json!(0);
            status["childGuardHandles"] = json!(1);
            emit(status)?;
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        if mode != "smoke" {
            emit(status)?;
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        let pipe = connect(&name, process.raw()).map_err(|e| {
            let mut code = 0;
            unsafe {
                GetExitCodeProcess(process.raw(), &mut code);
            }
            let _ = thread_state(process.raw(), "pipe-failure");
            io::Error::other(format!(
                "pipe peer/instance/DACL before QMP: {e}; QEMU status={code:#x}; stderr={}",
                String::from_utf8_lossy(&errors.lock().unwrap())
            ))
        })?;
        if traced {
            return Err(io::Error::other(
                "diagnostic debugger run cannot qualify as native PASS",
            ));
        }
        receive(&rx, None)?;
        for (id, command) in [(1, "qmp_capabilities"), (2, "query-status"), (3, "quit")] {
            serde_json::to_writer(&mut stdin, &json!({"execute":command,"id":id}))
                .map_err(io::Error::other)?;
            stdin.write_all(b"\n")?;
            stdin.flush()?;
            let result = receive(&rx, Some(id))?;
            if id == 2 {
                check(
                    result["return"]["status"] == "preconfig"
                        && result["return"]["running"] == false,
                )?;
            }
        }
        child.wait_exit(Duration::from_secs(45))?;
        drop(pipe);
        drop(child);
        drop(spec);
        drop(journal);
        let _j = runtime::journal::Journal::open(&root).map_err(io::Error::other)?;
        emit(
            json!({"sourceHead":crate::SOURCE_HEAD,"smoke":"passed","details":status,"pipeSecurity":"protected-user-system","processExitObserved":true,"lockReacquired":true}),
        )
    }
}
