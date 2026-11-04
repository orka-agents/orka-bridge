use crate::staging::{create, directory, invalid, Staging};
use bridge_runtime_protocol::*;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};
pub trait Runner {
    fn execute(&mut self, action: &EngineActionWire) -> io::Result<EngineResult>;
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    action: EngineActionWire,
    snapshot: Option<SnapshotId>,
    state: OperationState,
    result: Option<EngineResult>,
}
pub struct Operations<R> {
    root: PathBuf,
    runner: R,
}
impl<R: Runner> Operations<R> {
    pub fn new(root: PathBuf, runner: R) -> io::Result<Self> {
        directory(&root)?;
        directory(&root.join("results"))?;
        Ok(Self { root, runner })
    }
    pub fn runner(&self) -> &R {
        &self.runner
    }
    fn path(&self, id: Uuid) -> io::Result<PathBuf> {
        if id.is_nil() {
            return Err(invalid());
        }
        Ok(self.root.join(id.to_string()))
    }
    fn record(&self, id: Uuid) -> io::Result<Option<Record>> {
        let path = self.path(id)?;
        match crate::linux::read_regular(&path, (MAX_FILE * 5) as u64) {
            Ok(data) => {
                let mut r: Record = serde_json::from_slice(&data).map_err(|_| invalid())?;
                match crate::linux::read_regular(
                    &self.root.join("results").join(id.to_string()),
                    (MAX_FILE * 5) as u64,
                ) {
                    Ok(data) => {
                        r.result = Some(serde_json::from_slice(&data).map_err(|_| invalid())?)
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
                if r.result.as_ref().is_some_and(|x| x.stdout.len() > MAX_FILE) {
                    return Err(invalid());
                }
                Ok(Some(r))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn status(&self, id: Uuid) -> io::Result<OperationState> {
        Ok(self
            .record(id)?
            .map(|r| match r.state {
                OperationState::Pending | OperationState::Running => OperationState::Unknown,
                _ if r.result.is_none() => OperationState::Unknown,
                s => s,
            })
            .unwrap_or(OperationState::Unknown))
    }
    pub fn info(&self, id: Uuid) -> io::Result<OperationInfo> {
        let state = self.status(id)?;
        let r = self.record(id)?.and_then(|r| r.result);
        Ok(OperationInfo {
            state,
            exit: r.as_ref().map(|r| r.exit),
            bytes: r.as_ref().map_or(0, |r| r.stdout.len()),
            has_diagnostics: r.is_some_and(|r| r.has_diagnostics),
        })
    }
    pub fn read(&self, id: Uuid, offset: usize) -> io::Result<Vec<u8>> {
        let r = self
            .record(id)?
            .and_then(|r| r.result)
            .ok_or_else(invalid)?;
        if offset > r.stdout.len() {
            return Err(invalid());
        }
        Ok(r.stdout[offset..r.stdout.len().min(offset.saturating_add(MAX_CHUNK))].to_vec())
    }
    pub fn execute(
        &mut self,
        id: Uuid,
        action: EngineActionWire,
        snapshot: Option<SnapshotId>,
        staging: &Staging,
    ) -> io::Result<OperationState> {
        validate_action(&action).map_err(|_| invalid())?;
        if let Some(old) = self.record(id)? {
            if old.action != action || old.snapshot != snapshot {
                return Err(invalid());
            }
            return self.status(id);
        }
        if let Some(s) = &snapshot {
            if !staging.contains(s)? {
                return Err(invalid());
            }
        }
        let mut r = Record {
            action,
            snapshot,
            state: OperationState::Running,
            result: None,
        };
        let path = self.path(id)?;
        create(&path, &serde_json::to_vec(&r)?)?;
        fs::File::open(&self.root)?.sync_all()?;
        match self.runner.execute(&r.action) {
            Ok(mut out) if out.stdout.len() <= MAX_FILE => {
                if out.exit != 0 {
                    out.stdout.clear();
                } else {
                    match projection(&r.action, &out.stdout) {
                        Ok(bytes) => {
                            out.has_diagnostics |= matches!(r.action, EngineActionWire::Version {})
                                && out.stdout != bytes;
                            out.stdout = bytes;
                        }
                        Err(_) => {
                            out.exit = 1;
                            out.stdout.clear();
                        }
                    }
                }
                r.state = if out.exit == 0 {
                    OperationState::Completed
                } else {
                    OperationState::Failed
                };
                r.result = Some(out)
            }
            _ => r.state = OperationState::Unknown,
        }
        if let Some(output) = r.result.take() {
            let root = self.root.join("results");
            let mut entries = fs::read_dir(&root)?.collect::<io::Result<Vec<_>>>()?;
            entries.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
            while entries.len() >= MAX_OPERATIONS {
                let old = entries.remove(0);
                fs::remove_file(old.path())?;
            }
            create(&root.join(id.to_string()), &serde_json::to_vec(&output)?)?;
            fs::File::open(root)?.sync_all()?;
        }
        let next = path.with_extension("tmp");
        create(&next, &serde_json::to_vec(&r)?)?;
        fs::rename(next, path)?;
        fs::File::open(&self.root)?.sync_all()?;
        Ok(r.state)
    }
    pub fn handle(&mut self, staging: &mut Staging, body: &RequestBody) -> io::Result<ResultBody> {
        match body {
            RequestBody::StageBegin { upload, snapshot } => {
                staging.begin(*upload, snapshot.clone())?;
                Ok(ResultBody::Ready {})
            }
            RequestBody::StageChunk {
                upload,
                slot,
                offset,
                data,
            } => {
                staging.chunk(
                    *upload,
                    *slot,
                    *offset,
                    &decode_chunk(data).map_err(|_| invalid())?,
                )?;
                Ok(ResultBody::Ready {})
            }
            RequestBody::StageCommit { upload } => Ok(ResultBody::Snapshot {
                snapshot: staging.commit(*upload)?,
            }),
            RequestBody::Execute {
                operation,
                action,
                snapshot,
            } => {
                self.execute(*operation, action.clone(), snapshot.clone(), staging)?;
                Ok(ResultBody::Operation {
                    operation: *operation,
                    info: self.info(*operation)?,
                })
            }
            RequestBody::Status { operation } => Ok(ResultBody::Operation {
                operation: *operation,
                info: self.info(*operation)?,
            }),
            RequestBody::ReadResult { operation, offset } => Ok(ResultBody::Chunk {
                data: encode_chunk(&self.read(*operation, *offset)?),
            }),
            _ => Err(invalid()),
        }
    }
}
// Persist only B's finite response projection, not raw CRDs, specs or stderr.
fn projection(action: &EngineActionWire, bytes: &[u8]) -> io::Result<Vec<u8>> {
    use serde_json::{json, Value};
    match action {
        EngineActionWire::Version {} => {
            let s = std::str::from_utf8(bytes).map_err(|_| invalid())?;
            let token = s
                .lines()
                .next()
                .and_then(|s| s.strip_prefix("kmx "))
                .and_then(|s| s.split_whitespace().next())
                .ok_or_else(invalid)?;
            if token.len() > 128
                || !token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
            {
                return Err(invalid());
            }
            Ok(format!("kmx {token}\n").into_bytes())
        }
        EngineActionWire::ListAgents { .. } => {
            let v: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
            if !((v["kind"] == "List" && v["apiVersion"] == "v1")
                || (v["kind"] == "AgentList" && v["apiVersion"] == "core.orka.ai/v1alpha1"))
            {
                return Err(invalid());
            }
            if v["metadata"]["continue"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
                || v["metadata"]["remainingItemCount"].as_u64().unwrap_or(0) > 0
            {
                return Err(invalid());
            }
            let mut items = Vec::new();
            for item in v["items"].as_array().ok_or_else(invalid)? {
                if item["kind"] != "Agent" || item["apiVersion"] != "core.orka.ai/v1alpha1" {
                    return Err(invalid());
                }
                let mut metadata = serde_json::Map::new();
                for key in ["name", "namespace", "uid"] {
                    let s = item["metadata"][key]
                        .as_str()
                        .filter(|s| !s.is_empty() && s.len() <= 4096)
                        .ok_or_else(invalid)?;
                    metadata.insert(key.into(), json!(s));
                }
                items.push(json!({"apiVersion":"core.orka.ai/v1alpha1","kind":"Agent","metadata":metadata}));
            }
            serde_json::to_vec(&json!({"apiVersion":"v1","kind":"List","items":items}))
                .map_err(|_| invalid())
        }
    }
}
pub struct Kmx;
pub fn argv(action: &EngineActionWire) -> io::Result<Vec<String>> {
    validate_action(action).map_err(|_| invalid())?;
    Ok(match action {
        EngineActionWire::Version {} => vec!["version".into()],
        EngineActionWire::ListAgents { context, namespace } => [
            "cluster",
            "orka",
            "list",
            "--context",
            context,
            "--namespace",
            namespace,
            "-o",
            "json",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    })
}
impl Runner for Kmx {
    fn execute(&mut self, action: &EngineActionWire) -> io::Result<EngineResult> {
        use std::{
            io::Read,
            os::unix::process::CommandExt,
            process::{Command, Stdio},
            thread,
            time::{Duration, Instant},
        };
        let mut child = Command::new("/usr/local/bin/kmx")
            .args(argv(action)?)
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("HOME", "/root")
            .env("KUBECONFIG", "/root/.kube/config")
            .env("LC_ALL", "C")
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        let (tx, rx) = std::sync::mpsc::channel();
        for (tag, mut pipe) in [
            (
                false,
                Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
            ),
            (
                true,
                Box::new(child.stderr.take().unwrap()) as Box<dyn Read + Send>,
            ),
        ] {
            let tx = tx.clone();
            thread::spawn(move || {
                let mut b = Vec::new();
                let r = pipe
                    .by_ref()
                    .take((MAX_FILE + 1) as u64)
                    .read_to_end(&mut b);
                let _ = tx.send((tag, r.map(|_| b)));
            });
        }
        drop(tx);
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut out = None;
        let mut diagnostics = None;
        let result = (|| {
            loop {
                while let Ok((stderr, bytes)) = rx.try_recv() {
                    let b = bytes?;
                    if b.len() > MAX_FILE {
                        return Err(invalid());
                    }
                    if stderr {
                        diagnostics = Some(!b.is_empty())
                    } else {
                        out = Some(b)
                    }
                }
                let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
                // SAFETY: child is ours and never reaped until group cleanup; status is writable.
                if unsafe {
                    libc::waitid(
                        libc::P_PID,
                        child.id(),
                        &mut status,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                } != 0
                {
                    return Err(io::Error::last_os_error());
                }
                if unsafe { status.si_pid() } != 0 {
                    if let Some(has_diagnostics) = diagnostics {
                        if let Some(stdout) = out.take() {
                            return Ok(EngineResult {
                                exit: 0,
                                stdout,
                                has_diagnostics,
                            });
                        }
                    }
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::from(io::ErrorKind::TimedOut));
                }
                thread::sleep(Duration::from_millis(5));
            }
        })();
        // The owned group is still unreaped: its ID cannot be recycled before this kill.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let status = child.wait()?;
        result.map(|mut r| {
            r.exit = status.code().unwrap_or(-1);
            r
        })
    }
}
