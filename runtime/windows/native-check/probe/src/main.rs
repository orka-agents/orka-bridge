#[cfg(all(windows, target_env = "msvc", not(target_feature = "crt-static")))]
compile_error!("probe must use static CRT");
#[cfg(windows)]
include!(concat!(env!("OUT_DIR"), "/source.rs"));
#[cfg(any(windows, test))]
mod guard_probe;
#[cfg(any(windows, test))]
fn no_vm_prelaunch(reply: &serde_json::Value) -> bool {
    reply["return"]["status"] == "prelaunch" && reply["return"]["running"] == false
}
#[cfg(any(windows, test))]
fn capture_qmp_line(
    log: &mut impl std::io::Write,
    line: &[u8],
    total: &mut usize,
) -> std::io::Result<serde_json::Value> {
    if total.saturating_add(line.len()) > 2 * 1024 * 1024 {
        return Err(std::io::Error::other("QMP transcript exceeds 2 MiB"));
    }
    log.write_all(line)?;
    log.flush()?;
    *total += line.len();
    if line.is_empty() || line.len() > 131072 || line.last() != Some(&b'\n') {
        return Err(std::io::Error::other(
            "invalid QMP frame length or termination",
        ));
    }
    serde_json::from_slice(line).map_err(std::io::Error::other)
}
#[cfg(test)]
mod qmp_contract_tests {
    #[test]
    fn raw_reply_is_retained_even_when_parsing_fails() {
        let raw = b"not JSON\n";
        let mut log = Vec::new();
        assert!(super::capture_qmp_line(&mut log, raw, &mut 0).is_err());
        assert_eq!(log, raw);
    }
    #[test]
    fn fixed_no_vm_status_is_exact_prelaunch_not_any_nonrunning_state() {
        use serde_json::json;
        assert!(super::no_vm_prelaunch(
            &json!({"return":{"status":"prelaunch","running":false}})
        ));
        for state in ["preconfig", "paused", "shutdown", "internal-error"] {
            assert!(!super::no_vm_prelaunch(
                &json!({"return":{"status":state,"running":false}})
            ));
        }
        assert!(!super::no_vm_prelaunch(
            &json!({"return":{"status":"prelaunch","running":true}})
        ));
    }
}
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
    #[path = "guard_case.rs"]
    mod guard_case;
    #[path = "loader_trace.rs"]
    mod loader_trace;
    #[path = "owned_pipe_trial.rs"]
    mod owned_pipe_trial;
    #[path = "permission_diag.rs"]
    mod permission_diag;
    #[path = "unix_trial.rs"]
    mod unix_trial;
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
        endpoint_parent: PathBuf,
        #[serde(default)]
        ci_access_proof: bool,
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
        phase: &str,
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
                let kind = text(entry.TypeName, entry.TypeNameLength).map_err(|error| {
                    io::Error::other(format!(
                        "{}; invalid type text: {error}",
                        crate::guard_probe::missing_pss_type(
                            phase,
                            index,
                            entry.Handle as usize,
                            entry.Flags
                        )
                    ))
                })?;
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
                    eprintln!("PSS phase={phase} entry={index} handle={:#x} flags={:#x} objectType={} typeNameLength={} typeNamePtrNull={}",
                        entry.Handle as usize, entry.Flags, entry.ObjectType, entry.TypeNameLength, entry.TypeName.is_null());
                    return Err(crate::guard_probe::missing_pss_type(
                        phase,
                        index,
                        entry.Handle as usize,
                        entry.Flags,
                    ));
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
        let v = handle_snapshot("child-inherited-before-resume", process, &expected, &canary)?;
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
    fn reader(file: File, path: &PathBuf) -> io::Result<mpsc::Receiver<io::Result<Value>>> {
        let mut log = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let (tx, rx) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            let mut b = BufReader::new(file);
            let mut total = 0;
            loop {
                let mut line = Vec::new();
                let r = (&mut b)
                    .take(131073)
                    .read_until(b'\n', &mut line)
                    .and_then(|n| {
                        check(n > 0)?;
                        crate::capture_qmp_line(&mut log, &line, &mut total)
                    });
                let failed = r.is_err();
                if tx.send(r).is_err() || failed {
                    break;
                }
            }
        });
        Ok(rx)
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
        if args.get(1).map(String::as_str) == Some("unix-connect") {
            check(args.len() == 3)?;
            return unix_trial::client(std::path::Path::new(&args[2]));
        }
        if args.get(1).map(String::as_str) == Some("check") {
            return controller::run(&args[2..]);
        }
        let mode = args
            .get(1)
            .ok_or_else(|| io::Error::other("mode missing"))?;
        check(args.len() == if mode == "hold-guard-only" { 6 } else { 4 })?;
        let guard_close = if mode == "hold-guard-only" {
            let probe = crate::guard_probe::GuardProbe::parse(&args[4], &args[5], unsafe {
                windows_sys::Win32::System::SystemInformation::GetTickCount64()
            })?;
            let event = guard_case::open(&probe)?;
            Some((probe, event))
        } else {
            None
        };
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
        check(
            [
                "smoke",
                "hold-created",
                "hold-pipe",
                "hold-guard-only",
                "try-owned-pipe",
                "try-unix",
            ]
            .contains(&mode.as_str()),
        )?;
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
        // Reuse the source's verified synthetic factory and real materializer.
        // This issues a genuine installed binding; no guest bytes are booted.
        let source_root = root.join("inert-image-source");
        fs::create_dir(&source_root)?;
        let verified = crate::native_image_fixture::verified(
            &source_root,
            runtime::image_types::ImageTarget::WindowsAmd64,
        );
        let target = journal
            .image_target(&verified.image().identity().state_compatibility_id)
            .map_err(io::Error::other)?;
        let (_sender, closed) = tokio::sync::watch::channel(false);
        let installed = tokio::runtime::Builder::new_current_thread()
            .build()?
            .block_on(runtime::image_install::materialize_image(
                &verified,
                &target,
                closed,
                |_| {},
            ))
            .map_err(|error| io::Error::other(format!("inert image materialization: {error:?}")))?;
        let spec = runtime::probe_spec(&journal, Arc::new(installed));
        let name = format!("bridge-premise-{}", bridge_runtime_protocol::Uuid::new_v4());
        let endpoint = runtime::windows::control::ControlEndpoint::create(&cfg.endpoint_parent)?;
        let endpoint_path = endpoint.path().to_owned();
        let ci_control = if mode == "smoke" && cfg.ci_access_proof {
            Some(unix_trial::control_server()?)
        } else {
            None
        };
        let unix_trial = if mode == "try-unix" {
            Some(unix_trial::Trial::prepare(&root)?)
        } else {
            None
        };
        // Trial-only host server exists and is already listening BEFORE QEMU starts.
        let mut owned_pipe = if mode == "try-owned-pipe" {
            Some(owned_pipe_trial::Listener::create(&name)?)
        } else {
            None
        };
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
            match &unix_trial {
                Some(trial) => format!(
                    "socket,id=guest,path={},server=on,wait=on",
                    trial
                        .endpoint
                        .to_str()
                        .ok_or_else(|| io::Error::other("AF_UNIX UTF-8 path"))?
                )
                .into(),
                None if mode == "try-owned-pipe" => format!("pipe,id=guest,path={name}").into(),
                None => format!(
                    "socket,id=guest,path={},server=on,wait=off",
                    runtime::windows::launch::qemu_value(&endpoint_path)
                        .map_err(io::Error::other)?
                )
                .into(),
            },
        ]);
        // Reuse the production serial argument to exercise literal comma/Unicode paths.
        let production = runtime::windows::launch::launch_argv(
            &inputs,
            &spec,
            endpoint_path
                .to_str()
                .ok_or_else(|| io::Error::other("AF_UNIX UTF-8 path"))?,
        )
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
        let mut child = OwnedQemuProcess::spawn_suspended(
            inputs.clone(),
            spec.ownership.clone(),
            &root,
            &argv,
            endpoint,
        )?;
        let process = child.process_handle();
        crate::proof::check_token(process.raw())
            .map_err(|e| io::Error::other(format!("restricted primary token: {e}")))?;
        let inherited = inherited_guard(process.raw(), &spec.ownership, &canary)
            .map_err(|e| io::Error::other(format!("inherited handle snapshot: {e}")))?;
        if let Some((probe, event)) = guard_close {
            // No take_stdio/pumps: quiescent target has no pending pipe I/O.
            thread_state(process.raw(), "before-resume")?;
            child.resume()?;
            thread_state(process.raw(), "after-resume")?;
            let identity =
                runtime::identity::file_identity(&spec.ownership).map_err(io::Error::other)?;
            let canary_id = runtime::identity::file_identity(&canary).map_err(io::Error::other)?;
            let mut ready = json!({"sourceHead":crate::SOURCE_HEAD,"mode":mode,
                "stage":"guard-before-close","pid":unsafe{GetProcessId(process.raw())},
                "creationTime":creation_time(process.raw())?,"guardIdentity":identity,
                "canaryIdentity":canary_id,"root":root,"inherited":inherited,"restrictedPrimary":true,
                "knownGuardHandle":format!("{:#x}",spec.ownership.as_raw_handle() as usize)});
            let diagnostic = guard_case::describe_parent_handle(
                0x48usize as HANDLE,
                spec.ownership.as_raw_handle(),
            );
            eprintln!("parent handle 0x48 (diagnostic only): {diagnostic}");
            ready["parentHandle48Diagnostic"] = diagnostic;
            emit(ready.clone())?;
            guard_case::wait_for_signal(&probe, &event)?;
            let original = spec.ownership.as_raw_handle();
            // The deliberate raw close must never be followed by Rust dropping
            // this shared File: its numeric slot can subsequently be reused.
            let child = std::mem::ManuallyDrop::new(child);
            std::mem::forget(spec);
            std::mem::forget(journal);
            unsafe {
                win(CloseHandle(original))?;
            }
            ready["stage"] = json!("guard-after-close");
            ready["knownGuardClosed"] = json!(true);
            ready["explicitGuardInheritance"] = json!(
                inherited["guardCopies"] == 1
                    && inherited["inheritedJobs"] == 0
                    && inherited["unrelatedCanaries"] == 0
            );
            emit(ready.clone())?;
            // Controller checks Busy first, then requests owned-Job stop.
            guard_case::wait_for_signal(&probe, &event)?;
            child.abort()?;
            ready["stage"] = json!("guard-child-stopped");
            ready["ownedChildStopObserved"] = json!(true);
            emit(ready)?;
            // Remain alive while the controller proves the file is free.
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
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
        let log_root = if cfg.ci_access_proof && mode == "smoke" {
            std::env::current_exe()?
                .parent()
                .ok_or_else(|| io::Error::other("stage missing"))?
                .join("logs")
        } else {
            root.clone()
        };
        let qmp_rx_path = log_root.join("qmp-rx.jsonl");
        let qmp_tx_path = log_root.join("qmp-tx.jsonl");
        let mut qmp_tx = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&qmp_tx_path)?;
        let rx = reader(stdout, &qmp_rx_path)?;
        let traced = loader_trace::enabled()?;
        if traced {
            loader_trace::start(process.clone())?;
        }
        thread_state(process.raw(), "before-resume")?;
        if mode != "hold-created" {
            child.resume()?;
            thread_state(process.raw(), "after-resume")?;
        }
        let mut status = json!({"sourceHead":crate::SOURCE_HEAD,"pid":unsafe{GetProcessId(process.raw())},"creationTime":creation_time(process.raw())?,"mode":mode,"control":endpoint_path,"root":root,"inherited":inherited,"restrictedPrimary":true,"whpxUsability":"not-tested"});
        if let Some(trial) = unix_trial {
            let result = trial.run(&child, process.raw(), &mut stdin, &rx, &errors)?;
            return emit(
                json!({"sourceHead":crate::SOURCE_HEAD,"trialOnly":true,"afUnixTrial":result}),
            );
        }
        if let Some(listener) = owned_pipe.as_mut() {
            let until = Instant::now() + Duration::from_secs(30);
            let attached = loop {
                if listener.connected(process.raw())? {
                    break true;
                }
                if child.exited() {
                    // Do not classify a connect/exit race as failure to attach.
                    break listener.connected(process.raw())?;
                }
                check(Instant::now() < until)?;
                std::thread::sleep(Duration::from_millis(5));
            };
            if attached {
                child.abort()?;
            }
            child.wait_exit(Duration::from_secs(45))?;
            let mut exit = 0;
            unsafe {
                win(GetExitCodeProcess(process.raw(), &mut exit))?;
            }
            // Permit the bounded stderr pump to observe EOF, without consuming QMP.
            let until = Instant::now() + Duration::from_secs(1);
            while errors.lock().unwrap().is_empty() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            let stderr = String::from_utf8_lossy(&errors.lock().unwrap()).into_owned();
            drop(owned_pipe); // Cancel + observe pending ConnectNamedPipe completion.
            return emit(
                json!({"sourceHead":crate::SOURCE_HEAD,"trialOnly":true,"backendAttached":attached,"qemuExitCode":exit,"qemuExitObserved":true,"bridgePipeProtected":true,"bridgePipeUserSystemOnly":true,"rejectRemoteClientsFlag":true,"firstInstance":true,"overlappedCompletionObserved":true,"stderr":stderr}),
            );
        }
        if mode == "hold-pipe" {
            let connection = child.connect_control()?;
            status["qemuPeerPid"] = json!(connection.peer_pid());
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
        if traced {
            return Err(io::Error::other(
                "diagnostic debugger run cannot qualify as native PASS",
            ));
        }
        let until = Instant::now() + Duration::from_secs(30);
        while !endpoint_path.exists() {
            check(!child.exited() && Instant::now() < until)?;
            std::thread::sleep(Duration::from_millis(5));
        }
        let (_directory, directory_info) =
            unix_trial::checked_path(endpoint_path.parent().unwrap(), true)?;
        emit(
            json!({"stage":"native-control-ready","sourceHead":crate::SOURCE_HEAD,"endpoint":endpoint_path,
            "control":ci_control.as_ref().map(|c| &c.0),"workerPid":unsafe { GetCurrentProcessId() },"directory":directory_info}),
        )?;
        let proof = if cfg.ci_access_proof {
            Some(unix_trial::access_proof(&endpoint_path, &child)?)
        } else {
            None
        };
        let connection = child.connect_control().map_err(|e| {
            io::Error::other(format!(
                "AF_UNIX peer/metadata before QMP: {e}; stderr={}",
                String::from_utf8_lossy(&errors.lock().unwrap())
            ))
        })?;
        status["qemuPeerPid"] = json!(connection.peer_pid());
        let greeting = receive(&rx, None)?;
        let mut replies = Vec::new();
        for (id, command) in [(1, "qmp_capabilities"), (2, "query-status"), (3, "quit")] {
            if id == 3 {
                child.prepare_graceful_stop()?;
            }
            let mut bytes = serde_json::to_vec(&json!({"execute":command,"id":id}))
                .map_err(io::Error::other)?;
            bytes.push(b'\n');
            qmp_tx.write_all(&bytes)?;
            qmp_tx.flush()?;
            stdin.write_all(&bytes)?;
            stdin.flush()?;
            let reply = receive(&rx, Some(id))?;
            if id == 2 {
                check(crate::no_vm_prelaunch(&reply))?;
            }
            replies.push(reply);
        }
        child.wait_for_stop()?;
        let mut exit = 0;
        unsafe {
            win(GetExitCodeProcess(process.raw(), &mut exit))?;
        }
        if exit != 0 {
            return Err(io::Error::other(format!(
                "correlated QMP quit returned but process exit was {exit:#x}"
            )));
        }
        let mut byte = [0; 1];
        let terminal = connection
            .read(&mut byte, Instant::now() + Duration::from_secs(1))
            .unwrap_err();
        if terminal.kind() != io::ErrorKind::UnexpectedEof {
            return Err(io::Error::other(format!(
                "terminal owner-exit read/Job observation: {terminal}"
            )));
        }
        drop(connection);
        child.abort()?;
        drop(rx);
        drop(child);

        // A second fixed no-VM launch pins local Drop while QEMU is still alive.
        // Never exit preconfiguration or initialize a machine to manufacture EOF.
        let endpoint = runtime::windows::control::ControlEndpoint::create(&cfg.endpoint_parent)?;
        let disconnect_path = endpoint.path().to_owned();
        let mut disconnect_argv = argv.clone();
        let at = disconnect_argv
            .iter()
            .position(|a| a == "-chardev")
            .ok_or_else(|| io::Error::other("chardev missing"))?;
        disconnect_argv[at + 1] = format!(
            "socket,id=guest,path={},server=on,wait=off",
            runtime::windows::launch::qemu_value(&disconnect_path).map_err(io::Error::other)?
        )
        .into();
        let mut auxiliary = OwnedQemuProcess::spawn_suspended(
            inputs,
            spec.ownership.clone(),
            &root,
            &disconnect_argv,
            endpoint,
        )?;
        let (mut auxiliary_in, auxiliary_out, mut auxiliary_err) = auxiliary.take_stdio()?;
        std::thread::spawn(move || {
            let mut b = [0; 4096];
            while auxiliary_err.read(&mut b).is_ok_and(|n| n > 0) {}
        });
        let disconnect_rx = log_root.join("qmp-disconnect-rx.jsonl");
        let disconnect_tx = log_root.join("qmp-disconnect-tx.jsonl");
        let mut tx = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&disconnect_tx)?;
        let auxiliary_rx = reader(auxiliary_out, &disconnect_rx)?;
        auxiliary.resume()?;
        let live_connection = auxiliary.connect_control()?;
        receive(&auxiliary_rx, None)?;
        let command = b"{\"execute\":\"qmp_capabilities\",\"id\":1}\n";
        tx.write_all(command)?;
        tx.flush()?;
        auxiliary_in.write_all(command)?;
        auxiliary_in.flush()?;
        let auxiliary_reply = receive(&auxiliary_rx, Some(1))?;
        check(!auxiliary.exited())?;
        let peer = live_connection.peer_pid();
        drop(live_connection);
        auxiliary.wait_for_stop()?;
        auxiliary.abort()?;
        let mut terminated = 0;
        unsafe {
            win(GetExitCodeProcess(
                auxiliary.process_handle().raw(),
                &mut terminated,
            ))?;
        }
        check(terminated == 1)?;
        drop(auxiliary_rx);
        drop(auxiliary);
        drop(spec);
        drop(journal);
        let _j = runtime::journal::Journal::open(&root).map_err(io::Error::other)?;
        emit(
            json!({"sourceHead":crate::SOURCE_HEAD,"smoke":"passed","details":status,
            "controlSecurity":"protected-directory-user-system","processExitObserved":true,"lockReacquired":true,
            "otherAccount":proof,"qmp":{"greeting":greeting,"replies":replies,"exitCode":exit,"quitExitObserved":true,"terminalAfterOwnerExit":true,"rx":qmp_rx_path,"tx":qmp_tx_path},
            "terminalDisconnect":{"peerPid":peer,"exitCode":terminated,"exitObserved":true,"capabilityReply":auxiliary_reply,"rx":disconnect_rx,"tx":disconnect_tx}}),
        )
    }
}
