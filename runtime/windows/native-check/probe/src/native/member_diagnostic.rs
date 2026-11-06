// DIAGNOSTIC-ONLY (feature `member-diagnostic`): identify the actual live members
// of the owned Job for ONE fixed no-VM QEMU. Never a qualification: this module
// cannot emit `passed`, and its imported job.rs is a derived, hashed copy.
use super::*;
use crate::runtime::{identity::FileIdentity, windows::security::Handle};

pub(super) struct Launch {
    pub child: OwnedQemuProcess,
    pub stdin: File,
    pub stdout: File,
    pub errors: Arc<std::sync::Mutex<Vec<u8>>>,
    pub guard: FileIdentity,
    pub canary: FileIdentity,
    pub endpoint: PathBuf,
    pub inherited: Value,
}
/// Fixed identity-sampling bound. It is observation only, never a qualifier retry.
const SAMPLING: Duration = Duration::from_secs(5);
/// The original shared stop budget; held member handles must signal within it.
const CLEANUP: Duration = Duration::from_secs(45);
struct Member {
    handle: Handle,
    pid: u32,
    created: Option<u64>,
    leader: bool,
    record: Value,
}
struct Sampler {
    leader: (u32, u64),
    members: Vec<Member>,
    phases: Vec<Value>,
    spent: Duration,
    dropped_members: u64,
}
fn image(process: HANDLE) -> io::Result<String> {
    let mut buffer = vec![0u16; 32768];
    let mut n = buffer.len() as u32;
    unsafe {
        win(QueryFullProcessImageNameW(
            process,
            0,
            buffer.as_mut_ptr(),
            &mut n,
        ))?;
    }
    String::from_utf16(&buffer[..n as usize]).map_err(io::Error::other)
}
impl Sampler {
    fn second_held(&self) -> bool {
        self.members.iter().any(|m| {
            !m.leader
                && m.pid != 0
                && m.created.is_some()
                && m.record["aliveAtCapture"] == true
                && m.record["image"].as_str().is_some_and(|s| !s.is_empty())
        })
    }
    /// Polls until a non-leader member handle is bound or the window closes.
    fn sample(&mut self, child: &OwnedQemuProcess, label: &str, window: Duration) {
        let start = Instant::now();
        let until = start + window;
        let (mut samples, mut errors, mut last) = (0u32, Vec::new(), Value::Null);
        let mut dropped_errors = 0u64;
        let mut raw_pids_seen = Vec::new();
        let mut dropped_pids = 0u64;
        loop {
            samples += 1;
            match child.diagnostic_member_handles(until) {
                Ok(observation) => {
                    let (total, active, assigned, complete, entries) = (
                        observation.total,
                        observation.active,
                        observation.assigned,
                        observation.complete,
                        observation.entries,
                    );
                    last = json!({"total":total,"active":active,"assigned":assigned,
                        "listed":entries.len(),"completePidList":complete,
                        "rawPids":entries.iter().map(|(pid,_)|*pid).collect::<Vec<_>>()});
                    for (pid, entry) in entries {
                        if !raw_pids_seen.contains(&pid) {
                            if raw_pids_seen.len() < 256 {
                                raw_pids_seen.push(pid);
                            } else {
                                dropped_pids += 1;
                            }
                        }
                        match entry {
                            Ok(handle) => self.bind(handle, label, &last),
                            Err(e) if errors.len() < 16 => {
                                errors.push(json!({"pid":pid,
                                "error":e.to_string(),"rawOsError":e.raw_os_error(),
                                "requestedAccess":"SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION"}))
                            }
                            Err(_) => dropped_errors += 1,
                        }
                    }
                }
                Err(e) if errors.len() < 16 => errors
                    .push(json!({"pid":null,"error":e.to_string(),"rawOsError":e.raw_os_error()})),
                Err(_) => dropped_errors += 1,
            }
            if self.second_held() || Instant::now() >= until {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let elapsed = start.elapsed();
        self.spent += elapsed;
        self.phases.push(
            json!({"phase":label,"samples":samples,"elapsedMs":elapsed.as_millis() as u64,
            "lastAccounting":last,"errors":errors,"droppedErrors":dropped_errors,"rawPidsSeen":raw_pids_seen,"droppedPids":dropped_pids,
            "secondMemberHeld":self.second_held()}),
        );
    }
    fn bind(&mut self, handle: Handle, label: &str, accounting: &Value) {
        let raw = handle.raw();
        let pid = unsafe { GetProcessId(raw) };
        // A still-live held PID cannot be reused, even if its time query failed.
        if pid != 0
            && self.members.iter().any(|m| {
                m.pid == pid && unsafe { WaitForSingleObject(m.handle.raw(), 0) } == WAIT_TIMEOUT
            })
        {
            return;
        }
        if self.members.len() >= 32 {
            self.dropped_members += 1;
            return;
        }
        let created = creation_time(raw);
        let created_ok = created.as_ref().ok().copied();
        if self
            .members
            .iter()
            .any(|m| m.pid == pid && m.created == created_ok && created_ok.is_some())
        {
            return; // Same live member already held; drop this duplicate handle.
        }
        let leader = (pid, created_ok) == (self.leader.0, Some(self.leader.1));
        let alive = unsafe { WaitForSingleObject(raw, 0) } == WAIT_TIMEOUT;
        let image = image(raw);
        let record = json!({"pid":pid,
            "creationTime":created.as_ref().ok(),"creationTimeError":created.as_ref().err().map(|e|e.to_string()),
            "image":image.as_ref().ok(),"imageError":image.as_ref().err().map(|e|e.to_string()),
            "isProcessInJob":true,"aliveAtCapture":alive,"leader":leader,"firstPhase":label,"accountingAtCapture":accounting});
        self.members.push(Member {
            handle,
            pid,
            created: created_ok,
            leader,
            record,
        });
    }
}
fn exchange(
    rx: &mpsc::Receiver<io::Result<Value>>,
    tx: &mut File,
    stdin: &mut File,
) -> io::Result<Value> {
    let greeting = receive(rx, None)?;
    let mut replies = Vec::new();
    // Capability and status only: no quit, no machine initialization.
    for (id, command) in [(1, "qmp_capabilities"), (2, "query-status")] {
        let mut bytes =
            serde_json::to_vec(&json!({"execute":command,"id":id})).map_err(io::Error::other)?;
        bytes.push(b'\n');
        tx.write_all(&bytes)?;
        tx.flush()?;
        stdin.write_all(&bytes)?;
        stdin.flush()?;
        let reply = receive(rx, Some(id))?;
        if id == 2 && !crate::no_vm_prelaunch(&reply) {
            return Err(io::Error::other(format!("not exact prelaunch: {reply}")));
        }
        replies.push(reply);
    }
    Ok(json!({"greeting":greeting,"replies":replies,"exactPrelaunch":true}))
}
pub(super) fn run(mut l: Launch) -> io::Result<()> {
    let process = l.child.process_handle();
    let leader = (
        unsafe { GetProcessId(process.raw()) },
        creation_time(process.raw())?,
    );
    let logs = std::env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("stage missing"))?
        .join("logs");
    check(logs.is_dir())?;
    let rx_path = logs.join("job-member-diagnostic-qmp-rx.jsonl");
    let tx_path = logs.join("job-member-diagnostic-qmp-tx.jsonl");
    let mut qmp_tx = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tx_path)?;
    let rx = reader(l.stdout, &rx_path)?;
    let mut sampler = Sampler {
        leader,
        members: Vec::new(),
        phases: Vec::new(),
        spent: Duration::ZERO,
        dropped_members: 0,
    };
    l.child.resume()?;
    sampler.sample(
        &l.child,
        "immediate-after-resume",
        Duration::from_millis(50),
    );
    // Match the guard-only positive snapshot boundary: after resume and pipe-pump
    // setup, but before control/Winsock connection and unrelated loader probes.
    let parent = match handle_snapshot(unsafe { GetCurrentProcess() }, &l.guard, &l.canary) {
        Ok(counts) => json!({"complete":true,"counts":counts,"acceptance":"not-evaluated"}),
        Err(e) => json!({"complete":false,"error":e.to_string()}),
    };
    sampler.sample(
        &l.child,
        "after-parent-snapshot",
        (SAMPLING / 2).saturating_sub(sampler.spent),
    );
    let mut failures = Vec::new();
    let until = Instant::now() + Duration::from_secs(30);
    while !l.endpoint.exists() && !l.child.exited() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    emit(
        json!({"stage":"job-member-diagnostic-ready","sourceHead":crate::SOURCE_HEAD,"diagnosticOnly":true,
        "fullContainmentQualified":false,"workerPid":unsafe { GetCurrentProcessId() },"endpoint":l.endpoint,
        "endpointPresent":l.endpoint.exists()}),
    )?;
    let mut qmp = json!({"rx":rx_path,"tx":tx_path});
    let connection = match l.child.connect_control() {
        Ok(connection) => Some(connection),
        Err(e) => {
            failures.push(format!(
                "AF_UNIX peer/metadata before QMP: {e}; stderr={}",
                String::from_utf8_lossy(&l.errors.lock().unwrap())
            ));
            None
        }
    };
    if let Some(connection) = &connection {
        qmp["qemuPeerPid"] = json!(connection.peer_pid());
        match exchange(&rx, &mut qmp_tx, &mut l.stdin) {
            Ok(v) => qmp["exchange"] = v,
            Err(e) => failures.push(format!("QMP: {e}")),
        }
    }
    let remaining = SAMPLING.saturating_sub(sampler.spent);
    sampler.sample(&l.child, "after-query-status", remaining);
    // Terminate while the leader may still be alive; held handles prove each exit.
    let leader_alive = !l.child.exited();
    let started = Instant::now();
    let abort = l.child.abort().map_err(|e| e.to_string());
    let mut signaled = true;
    for member in &mut sampler.members {
        let left = (started + CLEANUP).saturating_duration_since(Instant::now());
        let wait = unsafe { WaitForSingleObject(member.handle.raw(), left.as_millis() as u32) };
        let mut code = 0;
        let exit = unsafe { GetExitCodeProcess(member.handle.raw(), &mut code) } != 0;
        member.record["signaledAfterAbort"] = json!(wait == WAIT_OBJECT_0);
        member.record["exitCode"] = json!(exit.then_some(code));
        signaled &= wait == WAIT_OBJECT_0;
    }
    drop(connection);
    let identified = sampler.second_held();
    emit(
        json!({"stage":"job-member-diagnostic-result","sourceHead":crate::SOURCE_HEAD,
        "diagnosticOnly":true,"fullContainmentQualified":false,
        "imports":{"diagnosticImports":crate::DIAGNOSTIC_IMPORTS,"derivedNotProduct":true,
            "jobSourcePath":crate::JOB_SOURCE_PATH,"jobSourceSha256":crate::JOB_SOURCE_SHA256,
            "jobImportPath":crate::JOB_IMPORT_PATH,"jobImportSha256":crate::JOB_IMPORT_SHA256},
        "leader":{"pid":leader.0,"creationTime":leader.1},"inherited":l.inherited,
        "parentSnapshotBeforeControl":parent,"secondMemberIdentified":identified,
        "members":sampler.members.iter().map(|m| &m.record).collect::<Vec<_>>(),
        "sampling":{"boundMs":SAMPLING.as_millis() as u64,"spentMs":sampler.spent.as_millis() as u64,"phases":sampler.phases,"storedMemberLimit":32,"droppedMembers":sampler.dropped_members},
        "qmp":qmp,"failures":failures,"leaderAliveAtAbort":leader_alive,"abort":abort,
        "memberHandlesSignaled":signaled,"cleanupBoundSecs":CLEANUP.as_secs()}),
    )?;
    drop(rx);
    check(signaled).map_err(|_| io::Error::other("held Job member handles unsignaled within 45 s"))
}
