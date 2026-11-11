//! Small local Send records, never execution handles or recovery instructions.
use crate::agent_folder::{invalid, read, replace};
use bridge_runtime_protocol::{SnapshotId, Uuid};
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io, path::PathBuf, sync::Mutex};

const VERSION: u32 = 1;
const MAX_ATTEMPTS: usize = 256;
const MAX_BYTES: u64 = 256 * 1024;
const MAX_TEXT: usize = 4096;
const MAX_TIME: u64 = 253_402_300_799;
const FILE_NAME: &str = "history.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AttemptVerdict {
    Running,
    Passed,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetUids {
    pub service_namespace: String,
    pub service_uid: String,
    pub deployment_namespace: String,
    pub deployment_uid: String,
    pub watched_namespace: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptSummary {
    pub id: String,
    pub source: SnapshotId,
    pub target_display: String,
    pub target_uids: TargetUids,
    pub provider: String,
    pub planned_agent_name: String,
    pub agent_uid: Option<String>,
    pub verdict: AttemptVerdict,
    pub time: u64,
}
impl AttemptSummary {
    fn validate(&self) -> Result<(), SummaryError> {
        fn text(value: &str) -> bool {
            !value.is_empty() && value.len() <= MAX_TEXT
        }
        fn hash(value: &str) -> bool {
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        if Uuid::parse_str(&self.id).is_err()
            || !hash(&self.source.agent)
            || !hash(&self.source.tests)
            || !text(&self.target_display)
            || !text(&self.provider)
            || !text(&self.planned_agent_name)
            || self.agent_uid.as_ref().is_some_and(|uid| !text(uid))
            || ![
                &self.target_uids.service_namespace,
                &self.target_uids.service_uid,
                &self.target_uids.deployment_namespace,
                &self.target_uids.deployment_uid,
                &self.target_uids.watched_namespace,
            ]
            .into_iter()
            .all(|field| text(field))
            || self.time > MAX_TIME
        {
            return Err(SummaryError::Corrupt);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SummaryError {
    Unavailable,
    Corrupt,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Ledger {
    version: u32,
    attempts: Vec<AttemptSummary>,
}
impl Default for Ledger {
    fn default() -> Self {
        Self {
            version: VERSION,
            attempts: Vec::new(),
        }
    }
}

pub struct SummaryStore {
    app_local_data: Option<PathBuf>,
    lock: Mutex<()>,
}
impl SummaryStore {
    pub fn new(app_local_data: Option<PathBuf>) -> Self {
        Self {
            app_local_data,
            lock: Mutex::new(()),
        }
    }
    pub fn begin(&self, summary: AttemptSummary) -> io::Result<()> {
        let _lock = self.lock.lock().map_err(|_| invalid())?;
        summary.validate().map_err(Self::as_io)?;
        let root = self.root(true)?.ok_or_else(invalid)?;
        let mut ledger = Self::read_ledger(Some(&root)).map_err(Self::as_io)?;
        upsert(&mut ledger.attempts, summary);
        let bytes = loop {
            let bytes = serde_json::to_vec(&ledger).map_err(|_| invalid())?;
            if bytes.len() as u64 <= MAX_BYTES {
                break bytes;
            }
            // Bound bytes as well as count; long display names must not fill the
            // store permanently. Always keep the newest attempt.
            if ledger.attempts.len() <= 1 {
                return Err(invalid());
            }
            ledger.attempts.remove(0);
        };
        // Retained no-follow directory; fresh inode, atomic rename and directory flush.
        replace(&root, FILE_NAME, &bytes)
    }
    pub fn finish(&self, summary: AttemptSummary) -> io::Result<()> {
        self.begin(summary)
    }
    pub fn list(&self) -> Result<Vec<AttemptSummary>, SummaryError> {
        let _lock = self.lock.lock().map_err(|_| SummaryError::Unavailable)?;
        let root = self.root(false).map_err(|_| SummaryError::Unavailable)?;
        Ok(Self::read_ledger(root.as_ref())?.attempts)
    }
    fn root(&self, create: bool) -> io::Result<Option<Dir>> {
        let parent = self.app_local_data.as_ref().ok_or_else(invalid)?;
        match fs::symlink_metadata(parent) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                // The native app-local-data root is trusted, not renderer-supplied.
                fs::create_dir_all(parent)?;
            }
            Err(e) => return Err(e),
            Ok(meta) if !meta.file_type().is_dir() => return Err(invalid()),
            _ => (),
        }
        let path = parent.join("send-history-v1");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    fs::DirBuilder::new().mode(0o700).create(&path)?;
                }
                #[cfg(windows)]
                drop(crate::runtime::windows::security::create_af_directory(
                    &path,
                )?);
            }
            Err(e) => return Err(e),
            Ok(meta) if !meta.file_type().is_dir() => return Err(invalid()),
            _ => (),
        }
        #[cfg(windows)]
        let _private = crate::runtime::windows::security::private_af_directory(&path)?;
        let parent = Dir::open_ambient_dir(parent, cap_std::ambient_authority())?;
        let root = parent.open_dir_nofollow("send-history-v1")?;
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt;
            if root.dir_metadata()?.permissions().mode() & 0o077 != 0 {
                return Err(invalid());
            }
        }
        Ok(Some(root))
    }
    fn read_ledger(root: Option<&Dir>) -> Result<Ledger, SummaryError> {
        let Some(root) = root else {
            return Ok(Ledger::default());
        };
        match root.symlink_metadata(FILE_NAME) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Ledger::default()),
            Err(_) => return Err(SummaryError::Unavailable),
            Ok(_) => (),
        }
        let bytes = read(root, FILE_NAME, MAX_BYTES as usize).map_err(|_| SummaryError::Corrupt)?;
        let ledger: Ledger = serde_json::from_slice(&bytes).map_err(|_| SummaryError::Corrupt)?;
        if ledger.version != VERSION || ledger.attempts.len() > MAX_ATTEMPTS {
            return Err(SummaryError::Corrupt);
        }
        let mut ids = HashSet::new();
        for attempt in &ledger.attempts {
            attempt.validate()?;
            if !ids.insert(&attempt.id) {
                return Err(SummaryError::Corrupt);
            }
        }
        Ok(ledger)
    }
    fn as_io(error: SummaryError) -> io::Error {
        io::Error::other(match error {
            SummaryError::Unavailable => "send summary storage unavailable",
            SummaryError::Corrupt => "send summary storage corrupt",
        })
    }
}

fn upsert(attempts: &mut Vec<AttemptSummary>, summary: AttemptSummary) {
    attempts.retain(|attempt| attempt.id != summary.id);
    attempts.push(summary);
    if attempts.len() > MAX_ATTEMPTS {
        attempts.drain(..attempts.len() - MAX_ATTEMPTS);
    }
}
