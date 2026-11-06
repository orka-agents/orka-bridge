use super::*;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    sync::Mutex,
};
use windows_sys::Win32::Security::{Authorization::*, *};
use windows_sys::Win32::System::{JobObjects::*, SystemInformation::OSVERSIONINFOW};

#[link(name = "ntdll")]
extern "system" {
    fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
}
struct Watch(HANDLE);
impl Drop for Watch {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub(super) fn caller() -> io::Result<(Value, String)> {
    unsafe {
        let mut raw = std::ptr::null_mut();
        win(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw))?;
        let token = Watch(raw);
        let (mut elevated, mut n) = (0u32, 0);
        win(GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut elevated as *mut u32).cast(),
            4,
            &mut n,
        ))?;
        if elevated != 0 {
            return Err(io::Error::other(
                "run unelevated; elevated callers are not this premise check",
            ));
        }
        let mut version: OSVERSIONINFOW = zeroed();
        version.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
        check(RtlGetVersion(&mut version) == 0 && version.dwBuildNumber >= 22000)?;
        let (mut machine, mut native) = (0u16, 0u16);
        win(IsWow64Process2(
            GetCurrentProcess(),
            &mut machine,
            &mut native,
        ))?;
        check(machine == 0 && native == 0x8664)?;
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut n);
        check(n as usize >= size_of::<TOKEN_USER>())?;
        let mut data = vec![0usize; (n as usize).div_ceil(size_of::<usize>())];
        win(GetTokenInformation(
            token.0,
            TokenUser,
            data.as_mut_ptr().cast(),
            n,
            &mut n,
        ))?;
        let sid = (*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        let mut text = std::ptr::null_mut();
        win(ConvertSidToStringSidW(sid, &mut text))?;
        let mut len = 0;
        while *text.add(len) != 0 {
            check(len < 256)?;
            len += 1;
        }
        let user =
            String::from_utf16(std::slice::from_raw_parts(text, len)).map_err(io::Error::other)?;
        LocalFree(text.cast());
        Ok((
            json!({"elevated":false,"osBuild":version.dwBuildNumber,"architecture":"x64","userSid":user}),
            user,
        ))
    }
}
pub(super) fn private_root(root: &Path, sid: &str) -> io::Result<()> {
    unsafe {
        check(root.is_absolute())?;
        let sddl = wide(&format!("O:{sid}D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)"));
        let mut sd = std::ptr::null_mut();
        win(ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut sd,
            std::ptr::null_mut(),
        ))?;
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd,
            bInheritHandle: 0,
        };
        let path = root
            .to_str()
            .ok_or_else(|| io::Error::other("work path must be Unicode"))?;
        let result = win(CreateDirectoryW(wide(path).as_ptr(), &sa));
        LocalFree(sd);
        result
    }
}
fn contain_controller() -> io::Result<()> {
    unsafe {
        let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Watch(raw);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        win(SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ))?;
        let mut flags = 0;
        win(GetHandleInformation(job.0, &mut flags))?;
        check(flags & HANDLE_FLAG_INHERIT == 0)?;
        // Test controller self-assignment happens BEFORE creating any workers.
        // The production QEMU launch still uses its unchanged atomic JOB_LIST.
        win(AssignProcessToJobObject(job.0, GetCurrentProcess()))?;
        // Keep this noninherited handle until OS process exit. Ctrl+C, abort or
        // CI timeout then reaps the whole test tree. It stays open during the
        // inner worker-death tests, so it cannot make a broken inner Job pass.
        std::mem::forget(job);
        Ok(())
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn packet(runtime: &Path, pin: &Path, work: &Path) -> io::Result<(PathBuf, Value)> {
    let trusted: Value = serde_json::from_slice(&fs::read(pin)?).map_err(io::Error::other)?;
    check(
        trusted["head"] == crate::SOURCE_HEAD
            && trusted["producerRun"] == 42120608528u64
            && trusted["producerHead"] == "006b30f02a3c9cec61f8f3cc32fde6b0d248fc34",
    )?;
    check(
        trusted["runtimeZipSha256"]
            .as_str()
            .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())),
    )?;
    let fixture = work.join("inert-fixtures");
    fs::create_dir(&fixture)?;
    let names = ["fixture.kernel", "fixture.initrd", "fixture.disk"];
    let bytes = b"INERT native no-VM fixture. Never supplied to QEMU.\n";
    let hash = digest(bytes);
    for name in names {
        fs::write(fixture.join(name), bytes)?;
    }
    let provenance: serde_json::Map<String, Value> = [
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
    ]
    .into_iter()
    .map(|key| (key.into(), json!(hash)))
    .collect();
    let uuid = "00000000-0000-4000-8000-000000000001";
    let config = json!({"head":crate::SOURCE_HEAD,"pin":{"runtimeFiles":trusted["runtimeFiles"],"image":{"name":names[2],"bytes":bytes.len(),"sha256":hash},"endpointSha256":hash,"directBoot":{"kernel":{"name":names[0],"bytes":bytes.len(),"sha256":hash,"format":"linux-bzimage"},"initrd":{"name":names[1],"bytes":bytes.len(),"sha256":hash,"format":"initramfs","compression":"cpio-prefix"},"rootSelector":{"kind":"filesystem-uuid","value":uuid},"commandLine":format!("root=UUID={uuid} ro console=ttyS0,115200n8 noresume"),"kernelRelease":"inert-fixture-amd64","requiredDrivers":(["virtio_pci","virtio_blk","virtio_console","virtio_rng","virtio_net","ext4"].map(|name|json!({"name":name,"builtIn":true}))),"provenance":provenance}},"paths":{"runtime":runtime,"kernel":fixture.join(names[0]),"initrd":fixture.join(names[1]),"factory":fixture.join(names[2])}});
    // Validate the test fixture against the EXACT imported source schema before
    // any worker or QEMU can run. This caught a built_in/builtIn fixture mismatch.
    let _: Config = serde_json::from_value(config.clone()).map_err(io::Error::other)?;
    let path = work.join("packet.json");
    fs::write(
        &path,
        serde_json::to_vec(&config).map_err(io::Error::other)?,
    )?;
    Ok((path, trusted))
}
struct Worker {
    child: Option<Child>,
    lines: mpsc::Receiver<io::Result<Value>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}
impl Worker {
    fn start(mode: &str, packet: &Path, root: &Path) -> io::Result<Self> {
        use std::os::windows::process::CommandExt;
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                OsString::from(mode),
                packet.as_os_str().to_owned(),
                root.as_os_str().to_owned(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("stdout missing"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("stderr missing"))?;
        let (tx, rx) = mpsc::sync_channel(2);
        let messages = if mode == "try-unix" || mode == "smoke" {
            2
        } else {
            1
        };
        std::thread::spawn(move || {
            let mut b = BufReader::new(stdout);
            for _ in 0..messages {
                let mut line = Vec::new();
                let r = (&mut b)
                    .take(131073)
                    .read_until(b'\n', &mut line)
                    .and_then(|n| {
                        check(n > 0 && n <= 131072 && line.last() == Some(&b'\n'))?;
                        serde_json::from_slice(&line).map_err(io::Error::other)
                    });
                if tx.send(r).is_err() {
                    break;
                }
            }
        });
        let errors = Arc::new(Mutex::new(Vec::new()));
        let out = errors.clone();
        std::thread::spawn(move || {
            let mut buf = [0; 4096];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if let Ok(mut v) = out.lock() {
                    let take = n.min(65536usize.saturating_sub(v.len()));
                    v.extend_from_slice(&buf[..take]);
                }
            }
        });
        Ok(Self {
            child: Some(child),
            lines: rx,
            stderr: errors,
        })
    }
    fn line(&self) -> io::Result<Value> {
        self.lines
            .recv_timeout(Duration::from_secs(180))
            .map_err(io::Error::other)?
            .map_err(|e| {
                io::Error::other(format!(
                    "worker: {e}; {}",
                    String::from_utf8_lossy(&self.stderr.lock().unwrap())
                ))
            })
    }
    fn wait(&mut self, timeout: Duration) -> io::Result<()> {
        let until = Instant::now() + timeout;
        loop {
            let status = self.child.as_mut().unwrap().try_wait()?;
            if let Some(status) = status {
                return check(status.success());
            }
            check(Instant::now() < until)?;
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn kill_observe(&mut self) -> io::Result<()> {
        let c = self.child.as_mut().unwrap();
        if c.try_wait()?.is_none() {
            c.kill()?;
        }
        let until = Instant::now() + Duration::from_secs(45);
        loop {
            if c.try_wait()?.is_some() {
                return Ok(());
            }
            check(Instant::now() < until)?;
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if self.kill_observe().is_err() {
            eprintln!("FAIL/BLOCKED: worker stop unobserved; do not claim clean teardown");
            std::mem::forget(self.child.take());
        }
    }
}
fn one(mode: &str, packet: &Path, root: &Path) -> io::Result<Value> {
    let mut worker = Worker::start(mode, packet, root)?;
    let v = worker.line()?;
    worker.wait(Duration::from_secs(45))?;
    check(v["sourceHead"] == crate::SOURCE_HEAD)?;
    Ok(v)
}
pub(super) fn run(args: &[String]) -> io::Result<()> {
    let trial = args.len() == 4 && args[3] == "--owned-pipe-trial";
    let unix_trial = args.len() == 4 && args[3] == "--af-unix-trial";
    let ci_access = args.len() == 4 && args[3] == "--ci-access-proof";
    check(args.len() == 3 || trial || unix_trial || ci_access)?;
    let runtime = PathBuf::from(&args[0]);
    let pin = PathBuf::from(&args[1]);
    let work = PathBuf::from(&args[2]);
    check(runtime.is_absolute() && pin.is_absolute())?;
    let (caller, sid) = caller()?;
    contain_controller()?;
    private_root(&work, &sid)?;
    let (packet, trusted) = packet(&runtime, &pin, &work)?;
    let endpoint_parent = work.join("i-é,");
    private_root(&endpoint_parent, &sid)?;
    let mut config: Value =
        serde_json::from_slice(&fs::read(&packet)?).map_err(io::Error::other)?;
    config["endpoint_parent"] = json!(endpoint_parent);
    config["ci_access_proof"] = json!(ci_access);
    fs::write(
        &packet,
        serde_json::to_vec(&config).map_err(io::Error::other)?,
    )?;
    if unix_trial {
        let mut worker = Worker::start("try-unix", &packet, &work.join("unix"))?;
        let first = worker.line()?;
        let result = if first["stage"] == "af-unix-ready" {
            check(first["sourceHead"] == crate::SOURCE_HEAD)?;
            emit(first)?;
            worker.line()?
        } else {
            first
        };
        worker.wait(Duration::from_secs(45))?;
        check(result["sourceHead"] == crate::SOURCE_HEAD && result["trialOnly"] == true)?;
        let receipt = json!({"status":"af-unix-trial-only","sourceHead":crate::SOURCE_HEAD,"caller":caller,"probeSha256":digest(&fs::read(std::env::current_exe()?)?),"runtimeZipSha256":trusted["runtimeZipSha256"],"producerRun":trusted["producerRun"],"guestBoot":false,"fullContainmentQualified":false,"afUnixTrial":result["afUnixTrial"]});
        fs::write(
            work.join("receipt.json"),
            serde_json::to_vec_pretty(&receipt).map_err(io::Error::other)?,
        )?;
        emit(receipt)?;
        eprintln!("DIAGNOSTIC AF_UNIX EXPERIMENT: not full native containment qualification");
        return Ok(());
    }
    if trial {
        let result = one("try-owned-pipe", &packet, &work.join("owned-pipe"))?;
        let receipt = json!({"status":"trial-only","sourceHead":crate::SOURCE_HEAD,"caller":caller,"probeSha256":digest(&fs::read(std::env::current_exe()?)?),"runtimeZipSha256":trusted["runtimeZipSha256"],"producerRun":trusted["producerRun"],"guestBoot":false,"whpxUsability":"not-tested","ownedPipeTrial":result});
        fs::write(
            work.join("receipt.json"),
            serde_json::to_vec_pretty(&receipt).map_err(io::Error::other)?,
        )?;
        emit(receipt)?;
        eprintln!("DIAGNOSTIC ONLY: host-owned pipe backend trial, not native qualification");
        return Ok(());
    }
    let mut cases = Vec::new();
    eprintln!("Native no-VM smoke at {}", crate::SOURCE_HEAD);
    let mut worker = Worker::start("smoke", &packet, &work.join("smoke,é space"))?;
    let ready = worker.line()?;
    check(ready["sourceHead"] == crate::SOURCE_HEAD && ready["stage"] == "native-control-ready")?;
    if ci_access {
        emit(ready)?;
    }
    let smoke = worker.line()?;
    worker.wait(Duration::from_secs(45))?;
    check(
        smoke["sourceHead"] == crate::SOURCE_HEAD
            && smoke["smoke"] == "passed"
            && smoke["qmp"]["quitExitObserved"] == true
            && smoke["qmp"]["exitCode"] == 0
            && smoke["terminalDisconnect"]["exitObserved"] == true
            && smoke["terminalDisconnect"]["exitCode"] == 1,
    )?;
    if ci_access {
        check(
            smoke["otherAccount"]["privateError"] == 10013
                && smoke["otherAccount"]["controlConnected"] == true,
        )?;
    }
    cases.push(json!({"case":"smoke","result":smoke}));
    for mode in ["hold-created", "hold-pipe", "hold-guard-only"] {
        eprintln!("Native no-VM case {mode}");
        let root = work.join(mode);
        let mut worker = Worker::start(mode, &packet, &root)?;
        let ready = worker.line()?;
        check(ready["mode"] == mode && ready["sourceHead"] == crate::SOURCE_HEAD)?;
        let pid = ready["pid"]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| io::Error::other("worker PID missing"))?;
        // Observation only, never termination by a PID lookup. Bind PID to creation time
        // while the actual owned worker is alive, before deliberately killing that worker.
        let watch =
            Watch(unsafe { OpenProcess(0x00100000 | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) });
        check(!watch.0.is_null())?;
        check(
            creation_time(watch.0)?
                == ready["creationTime"]
                    .as_u64()
                    .ok_or_else(|| io::Error::other("creation time missing"))?,
        )?;
        check(unsafe { WaitForSingleObject(watch.0, 0) } == WAIT_TIMEOUT)?;
        let busy = one("check-busy", &packet, &root)?;
        check(busy["lock"] == "busy")?;
        if mode == "hold-guard-only" {
            check(ready["parentGuardHandles"] == 0 && ready["childGuardHandles"] == 1)?;
        }
        worker.kill_observe()?;
        check(unsafe { WaitForSingleObject(watch.0, 45000) } == WAIT_OBJECT_0)?;
        let free = one("check-free", &packet, &root)?;
        check(free["lock"] == "free")?;
        cases.push(
            json!({"case":mode,"ready":ready,"busy":busy,"childExitObserved":true,"free":free}),
        );
    }
    let receipt = json!({"status":"passed","sourceHead":crate::SOURCE_HEAD,"caller":caller,"probeSha256":digest(&fs::read(std::env::current_exe()?)?),"runtimeZipSha256":trusted["runtimeZipSha256"],"producerRun":trusted["producerRun"],"guestAssets":"inert-test-fixtures-not-qualified","rootAcl":"test-setup-user-system","guestBoot":false,"whpxUsability":"not-tested","cases":cases});
    fs::write(
        work.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).map_err(io::Error::other)?,
    )?;
    emit(receipt)?;
    println!(
        "PASS: native no-VM premises at {}; WHPX usability NOT TESTED",
        crate::SOURCE_HEAD
    );
    Ok(())
}
