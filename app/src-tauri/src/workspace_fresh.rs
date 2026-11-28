//! Start a fresh workspace: retire the local workspace on this computer so that first install
//! builds a new one of the same release. First install reuses the image already in the cache
//! when it is the current release's; otherwise it downloads it, and the action checks first that
//! the download can be reached and fits before anything is retired.
//!
//! One durable intent in `workspace-fresh-v1` names the directory being retired by its file
//! identity. The directory is renamed, never copied or deleted in place, to a Bridge-owned
//! sibling named from the transaction (`discarded-<transaction>`), so every crash leaves either
//! the original `local-vm-v1` intact or a retired directory the intent recognises. Recovery
//! reconciles identities, not the last phase label. Only a settled transaction admits an ordinary
//! launch, apart from an intent that provably moved nothing, and a retired directory is deleted
//! only after the transaction is settled and only while it is still the recorded directory.
//!
//! Once the intent is durable, no failure is reported as "nothing changed" unless the directory
//! identities prove it: a failure after the rename is an interrupted transaction that Bridge
//! finishes on the next try or the next launch.
//!
//! The caller owns lifecycle quiescence: the VM is stopped and its native custody released before
//! this backend runs. Acquiring the workspace's own owner lock here proves no native worker still
//! holds it. No path is renderer input.
use crate::runtime::{
    credential_store::CredentialStore,
    identity::{file_identity, FileIdentity},
    image_cache,
    journal::{InstallationRecord, StatePhase},
    types::RuntimeError,
};
use crate::setup::SetupStore;
use bridge_runtime_protocol::Uuid;
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(crate) const STORE: &str = "workspace-fresh-v1";
const CANONICAL: &str = "local-vm-v1";
/// The intent file, as a name relative to the local data folder.
const INTENT: &str = "workspace-fresh-v1/intent.json";
const MAX_INTENT: u64 = 16 * 1024;
/// The most retired directories whose deletion stays unconfirmed and is reported.
const MAX_EARLIER: usize = 8;

/// Why a fresh workspace was not started. In every case the workspace on this computer is left
/// exactly as it was: nothing was moved or deleted. Refusals found after the stop restart a
/// workspace that was running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Refusal {
    /// The workspace's stop could not be confirmed, so nothing was moved.
    StopUnconfirmed,
    /// Another workspace action, or the previous fresh workspace's cleanup, is still running.
    Busy,
    /// Something still holds the workspace's files: its owner lock, or the rename was refused.
    InUse,
    /// `local-vm-v1` is not a directory Bridge recognises as a workspace, or folders Bridge
    /// expected to find are different.
    Layout,
    /// A workspace update's records are present; replacing the workspace would strand them.
    UpdateHistory,
    /// First install must download the image and the download could not be reached.
    Offline,
    /// First install must download the image and the computer has too little free space.
    NoSpace,
    /// Bridge's own storage could not be read or written.
    Unavailable,
}
/// What happened to Bridge's saved AI sign-in or key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AiOutcome {
    /// The credential slot and the saved record were removed, as Reset removes them.
    Removed,
    /// Nothing was saved.
    Absent,
    /// Removal was not confirmed. The saved record is kept intact, so connecting on the new
    /// workspace offers Reset AI connection for the same slot.
    Kept,
    /// Removal was not confirmed and the saved record could not be read as one Reset can
    /// remove, so the AI card cannot offer it.
    Unconfirmed,
}
/// What first install needs for the new workspace, for the confirmation's wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FreshImage {
    /// The current release's image is already in the cache.
    Cached,
    /// The image must be downloaded: compressed image plus boot files.
    Download {
        #[serde(skip_serializing_if = "Option::is_none")]
        bytes: Option<u64>,
    },
    /// This build cannot tell (a development image).
    Unknown,
}
/// Whether the action can start, for the renderer to decide whether to offer it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreshPlan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<Refusal>,
    pub image: FreshImage,
}
/// Nonsecret projection for the renderer. Paths are native-selected and display-only.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FreshStatusDto {
    Replacing,
    Replaced {
        transaction: Uuid,
        ai: AiOutcome,
        /// Retired workspaces whose deletion has not been confirmed: reclaimable space.
        reclaimable: Vec<String>,
        /// Deletion of the retired workspace is still running.
        cleaning: bool,
    },
    /// Nothing was moved or deleted.
    Refused {
        reason: Refusal,
        /// Bridge started the workspace again after stopping it for the action.
        restarted: bool,
        /// Free space first install needs, for `noSpace`.
        #[serde(skip_serializing_if = "Option::is_none")]
        needed_bytes: Option<u64>,
        /// Folders an earlier fresh workspace left behind, still reported.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        reclaimable: Vec<String>,
    },
    /// The workspace was set aside but the transaction did not finish. Bridge finishes it on the
    /// next try or the next launch; no workspace starts until then.
    Interrupted {
        transaction: Uuid,
    },
    /// Bridge found folders it did not expect and left everything as it is. The workspace does
    /// not start until they are moved away.
    Conflict {
        found: Vec<String>,
    },
}
impl FreshStatusDto {
    pub fn refused(reason: Refusal) -> Self {
        Self::Refused {
            reason,
            restarted: false,
            needed_bytes: None,
            reclaimable: vec![],
        }
    }
    /// Whether the workspace must stay stopped until the transaction is finished or sorted out.
    pub fn blocks_launch(&self) -> bool {
        matches!(self, Self::Interrupted { .. } | Self::Conflict { .. })
    }
}
/// Why `replace` did not settle a transaction.
#[derive(Debug, PartialEq)]
pub(crate) enum Failure {
    /// Nothing was moved or deleted.
    Refused(Refusal),
    /// The intent is past the point where nothing changed; recovery finishes it.
    Interrupted(Uuid),
    /// Unexpected entries, as names relative to the local data folder; left as found.
    Conflict(Vec<String>),
}
impl From<Refusal> for Failure {
    fn from(reason: Refusal) -> Self {
        Self::Refused(reason)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Phase {
    /// Durable before any move. The directory may still be in place.
    Intent,
    /// `local-vm-v1` is gone; the retired directory is `discarded-<transaction>`.
    Retired,
    /// The saved AI connection was reset (or its removal recorded as unconfirmed). Ordinary
    /// first install may now build the new workspace.
    Settled,
}
/// An earlier settled transaction whose retired directory still exists, with that directory's
/// identity so deletion never removes anything else under its name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Earlier {
    transaction: Uuid,
    directory: FileIdentity,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Intent {
    version: u16,
    transaction: Uuid,
    created_at: u64,
    phase: Phase,
    /// The directory that was `local-vm-v1` when the intent was written; none if absent.
    directory: Option<FileIdentity>,
    /// Its installation, when it had a committed record.
    installation: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ai: Option<AiOutcome>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    earlier: Vec<Earlier>,
}
impl Intent {
    fn valid(&self) -> bool {
        self.version == 1
            && !self.transaction.is_nil()
            && self.installation.is_none_or(|id| !id.is_nil())
            && (self.phase == Phase::Settled) == self.ai.is_some()
            && self.earlier.len() <= MAX_EARLIER
            && self
                .earlier
                .iter()
                .all(|e| !e.transaction.is_nil() && e.transaction != self.transaction)
    }
    /// Every retired directory this intent may still own, with its recorded identity.
    fn retired(&self) -> Vec<(Uuid, Option<&FileIdentity>)> {
        self.earlier
            .iter()
            .map(|e| (e.transaction, Some(&e.directory)))
            .chain(Some((self.transaction, self.directory.as_ref())))
            .collect()
    }
}
fn discarded(transaction: Uuid) -> String {
    format!("discarded-{transaction}")
}
/// A settled transaction, as the caller reports it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Settled {
    pub transaction: Uuid,
    pub ai: AiOutcome,
    /// Names (relative to the local data folder) of retired directories still present.
    pub reclaimable: Vec<String>,
}
/// A directory that is `local-vm-v1` and has the layout of a Bridge workspace.
struct Canonical {
    identity: FileIdentity,
    installation: Option<Uuid>,
    /// The committed record's phase; none before `Journal::create` wrote its first record.
    phase: Option<StatePhase>,
    /// The workspace's own owner lock, proving no native worker holds it.
    owner: crate::runtime::journal::OwnerLock,
}
impl Canonical {
    /// A first install that never finished: no guest was ever handed its state disk to format,
    /// so the workspace holds nothing of the person's. After a restart nothing can finish it,
    /// because only the process that created the state disk may install its image or format it.
    fn never_booted(&self) -> bool {
        self.phase != Some(StatePhase::Consumed)
    }
}
/// Past the durable intent, a failure is an interrupted transaction.
fn interrupted<E>(transaction: Uuid) -> impl FnOnce(E) -> Failure {
    move |_| Failure::Interrupted(transaction)
}
fn unavailable<E>(_: E) -> Refusal {
    Refusal::Unavailable
}
fn read_small(dir: &Dir, name: &str, max: u64) -> Result<Option<Vec<u8>>, Refusal> {
    match dir.symlink_metadata(name) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(m) if m.is_file() && m.len() <= max => {}
        Ok(_) => return Err(Refusal::Layout),
        Err(_) => return Err(Refusal::Unavailable),
    }
    let file = image_cache::regular(dir, name, false, false).map_err(|_| Refusal::Layout)?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(unavailable)?;
    if bytes.len() as u64 > max {
        return Err(Refusal::Layout);
    }
    Ok(Some(bytes))
}
/// The identity of a directory entry, without following links. Anything but a plain directory
/// is a layout refusal.
fn entry_identity(parent: &Dir, name: &str) -> Result<Option<FileIdentity>, Refusal> {
    match parent.symlink_metadata(name) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(Refusal::Layout),
        Err(_) => return Err(Refusal::Unavailable),
    }
    let dir = parent
        .open_dir_nofollow(name)
        .map_err(|_| Refusal::Layout)?;
    let file = dir.into_std_file();
    image_cache::not_reparse(&file).map_err(|_| Refusal::Layout)?;
    file_identity(&file).map(Some).map_err(unavailable)
}
/// Whether anything at all has this name. Unknown counts as present: never lose sight of one.
fn entry_present(parent: &Dir, name: &str) -> bool {
    !matches!(parent.symlink_metadata(name), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
/// The fresh-workspace owner. Holds `workspace-fresh-v1/owner.lock` for its lifetime.
pub(crate) struct FreshWorkspace {
    data: PathBuf,
    local: PathBuf,
    /// The local data folder: every rename and deletion is relative to this handle.
    parent: Dir,
    directory: Dir,
    owner: File,
    owner_process: u32,
    /// Every durable step in order, so tests can prove each required flush happens and when.
    #[cfg(test)]
    trace: std::sync::Mutex<Vec<String>>,
}
/// Steps made to fail once, per local data folder, so tests can fail a real step of a
/// transaction they do not drive directly.
#[cfg(test)]
static FAIL: std::sync::Mutex<Vec<(PathBuf, String)>> = std::sync::Mutex::new(Vec::new());
#[cfg(test)]
pub(crate) fn fail_once(local: &Path, step: &str) {
    FAIL.lock()
        .unwrap()
        .push((local.to_owned(), step.to_owned()));
}
impl Drop for FreshWorkspace {
    fn drop(&mut self) {
        // As for the upgrade owner: only this process may release the transaction lock.
        if self.owner_process == std::process::id() {
            let _ = self.owner.unlock();
        }
    }
}
impl FreshWorkspace {
    pub(crate) fn open(data: &Path, local: &Path) -> Result<Self, Refusal> {
        let parent =
            Dir::open_ambient_dir(local, cap_std::ambient_authority()).map_err(unavailable)?;
        let directory = image_cache::directory(&parent, STORE.as_ref()).map_err(unavailable)?;
        image_cache::fs::sync(&parent).map_err(unavailable)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        let owner = directory
            .open_with("owner.lock", &options)
            .map_err(unavailable)?
            .into_std();
        image_cache::not_reparse(&owner).map_err(unavailable)?;
        owner.try_lock().map_err(|e| match e {
            fs::TryLockError::WouldBlock => Refusal::Busy,
            fs::TryLockError::Error(_) => Refusal::Unavailable,
        })?;
        Ok(Self {
            data: data.to_owned(),
            local: local.to_owned(),
            parent,
            directory,
            owner,
            owner_process: std::process::id(),
            #[cfg(test)]
            trace: Default::default(),
        })
    }
    /// One durable step. Tests see it in the trace and can make it fail once.
    fn step<T, E>(&self, name: &str, work: impl FnOnce() -> Result<T, E>) -> Result<T, Refusal> {
        #[cfg(test)]
        {
            self.trace.lock().unwrap().push(name.to_owned());
            let mut fail = FAIL.lock().unwrap();
            if let Some(index) = fail
                .iter()
                .position(|(local, step)| *local == self.local && step == name)
            {
                fail.remove(index);
                return Err(Refusal::Unavailable);
            }
        }
        #[cfg(not(test))]
        let _ = name;
        work().map_err(unavailable)
    }
    fn read_intent(&self) -> Result<Option<Intent>, Refusal> {
        read_intent(&self.directory)
    }
    fn persist(&self, intent: &Intent) -> Result<(), Refusal> {
        if !intent.valid() {
            return Err(Refusal::Unavailable);
        }
        let bytes = serde_json::to_vec(intent).map_err(unavailable)?;
        if bytes.len() as u64 > MAX_INTENT {
            return Err(Refusal::Unavailable);
        }
        let phase = match intent.phase {
            Phase::Intent => "intent",
            Phase::Retired => "retired",
            Phase::Settled => "settled",
        };
        let store = self.local.join(STORE);
        let pending = store.join(format!("intent-{}.next", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pending)
            .map_err(unavailable)?;
        file.write_all(&bytes).map_err(unavailable)?;
        self.step(&format!("flush {phase}"), || file.sync_all())?;
        drop(file);
        self.step(&format!("record {phase}"), || {
            crate::runtime::journal::replace(&pending, &store.join("intent.json"))
        })?;
        self.step(&format!("flush store {phase}"), || {
            image_cache::fs::sync(&self.directory)
        })
    }
    /// Forget an intent that never moved anything, so a refusal leaves no transaction behind.
    /// Best effort for its callers: an intent left at this phase with nothing moved never
    /// blocks a launch.
    fn abandon(&self, transaction: Uuid) -> Result<(), Refusal> {
        if self.read_intent()?.is_some_and(|intent| {
            intent.transaction == transaction && intent.phase == Phase::Intent
        }) {
            self.step("withdraw", || self.directory.remove_file("intent.json"))?;
            self.step("flush store withdrawn", || {
                image_cache::fs::sync(&self.directory)
            })?;
        }
        Ok(())
    }
    /// The identity of a directory entry in the local data folder, without following links.
    fn identity_of(&self, name: &str) -> Result<Option<FileIdentity>, Refusal> {
        entry_identity(&self.parent, name)
    }
    /// `local-vm-v1`, if present, as a workspace Bridge recognises, with its owner lock held.
    fn canonical(&self) -> Result<Option<Canonical>, Refusal> {
        let Some(identity) = self.identity_of(CANONICAL)? else {
            return Ok(None);
        };
        // Moving a directory to another volume would copy it; never do that.
        let parent = file_identity(
            &self
                .parent
                .try_clone()
                .map_err(unavailable)?
                .into_std_file(),
        )
        .map_err(unavailable)?;
        if !parent.same_volume(&identity) {
            return Err(Refusal::Layout);
        }
        let dir = self
            .parent
            .open_dir_nofollow(CANONICAL)
            .map_err(|_| Refusal::Layout)?;
        match dir.symlink_metadata("owner.lock") {
            Ok(m) if m.is_file() => {}
            _ => return Err(Refusal::Layout),
        }
        // A live or uncertain native worker keeps this lock: refuse rather than move its disk.
        let owner = crate::runtime::journal::Journal::recovery_owner(&self.local.join(CANONICAL))
            .map_err(|error| match error {
            RuntimeError::Busy => Refusal::InUse,
            RuntimeError::RetainedState => Refusal::Layout,
            _ => Refusal::InUse,
        })?;
        let (installation, phase) = match read_small(&dir, "record.json", 32 * 1024)? {
            Some(bytes) => {
                let record: InstallationRecord =
                    serde_json::from_slice(&bytes).map_err(|_| Refusal::Layout)?;
                if record.version != 1 || record.installation_id.is_nil() || record.generation == 0
                {
                    return Err(Refusal::Layout);
                }
                match dir.symlink_metadata("state.disk") {
                    Ok(m) if m.is_file() => {}
                    _ => return Err(Refusal::Layout),
                }
                (Some(record.installation_id), Some(record.phase))
            }
            None => {
                // Only what Journal::create writes before its first record: the lock, an empty
                // state disk and possibly a pending record.
                for entry in dir.entries().map_err(unavailable)? {
                    let entry = entry.map_err(unavailable)?;
                    let name = entry.file_name();
                    let kind = entry.file_type().map_err(unavailable)?;
                    match name.to_str() {
                        Some("owner.lock" | "record.next") if kind.is_file() => {}
                        Some("state.disk")
                            if kind.is_file()
                                && entry.metadata().map_err(unavailable)?.len() == 0 => {}
                        _ => return Err(Refusal::Layout),
                    }
                }
                (None, None)
            }
        };
        if self.identity_of(CANONICAL)?.as_ref() != Some(&identity) {
            return Err(Refusal::Layout);
        }
        Ok(Some(Canonical {
            identity,
            installation,
            phase,
            owner,
        }))
    }
    /// Which of the given names exist, for a conflict report.
    fn present(&self, names: &[&str]) -> Vec<String> {
        names
            .iter()
            .filter(|name| entry_present(&self.parent, name))
            .map(|name| name.to_string())
            .collect()
    }
    /// Finish or undo an interrupted transaction. Returns the current intent, if any remains.
    /// Nothing here guesses: an unexpected combination of directories is a conflict and is left
    /// exactly as found.
    fn recover_with(
        &self,
        setup: &SetupStore,
        store: &impl CredentialStore,
        point: &impl Fn(&str) -> Result<(), RuntimeError>,
    ) -> Result<Option<Intent>, Failure> {
        let Some(mut intent) = self
            .read_intent()
            .map_err(|_| Failure::Conflict(vec![INTENT.into()]))?
        else {
            return Ok(None);
        };
        if intent.phase == Phase::Settled {
            return Ok(Some(intent));
        }
        let transaction = intent.transaction;
        let name = discarded(transaction);
        let conflict = || Failure::Conflict(self.present(&[CANONICAL, &name]));
        let identity = |entry: &str| match self.identity_of(entry) {
            Ok(identity) => Ok(identity),
            Err(Refusal::Layout) => Err(conflict()),
            Err(_) => Err(Failure::Interrupted(transaction)),
        };
        let canonical = identity(CANONICAL)?;
        let retired = identity(&name)?;
        match (&intent.directory, &canonical, &retired) {
            // The rename never happened: nothing moved, so the intent is simply withdrawn.
            (Some(directory), Some(current), None)
                if current == directory && intent.phase == Phase::Intent =>
            {
                let _ = self.abandon(transaction);
                return Ok(None);
            }
            (Some(directory), None, Some(moved)) if moved == directory => {}
            // Nothing is in place: whatever happened to the directory, no workspace can be using
            // the saved connection, and first install may proceed once it is reset.
            (_, None, None) => {}
            _ => return Err(conflict()),
        }
        if intent.phase == Phase::Intent {
            intent.phase = Phase::Retired;
            self.persist(&intent).map_err(interrupted(transaction))?;
            point("retired-recorded").map_err(interrupted(transaction))?;
        }
        self.settle(&mut intent, setup, store, point)?;
        Ok(Some(intent))
    }
    fn settle(
        &self,
        intent: &mut Intent,
        setup: &SetupStore,
        store: &impl CredentialStore,
        point: &impl Fn(&str) -> Result<(), RuntimeError>,
    ) -> Result<(), Failure> {
        let transaction = intent.transaction;
        // The workspace that owned the saved AI connection is gone from its place. Forget it as
        // the installation-mismatch Reset does. An unconfirmed removal keeps the record, so the
        // existing Reset offers it again; it never blocks the new workspace.
        #[cfg(test)]
        self.trace.lock().unwrap().push("reset ai".into());
        let ai = match setup.reset_retired_model(store) {
            Ok(true) => AiOutcome::Removed,
            Ok(false) => AiOutcome::Absent,
            Err(crate::setup::RetiredReset::Kept) => AiOutcome::Kept,
            Err(crate::setup::RetiredReset::Unconfirmed) => AiOutcome::Unconfirmed,
        };
        point("ai").map_err(interrupted(transaction))?;
        intent.ai = Some(ai);
        intent.phase = Phase::Settled;
        self.persist(intent).map_err(interrupted(transaction))?;
        point("settled").map_err(interrupted(transaction))
    }
    /// Retire the workspace and settle. A failure past the durable intent is retried once
    /// through recovery before it is reported as interrupted.
    pub(crate) fn replace(
        &mut self,
        setup: &SetupStore,
        store: &impl CredentialStore,
    ) -> Result<Settled, Failure> {
        match self.replace_with(setup, store, |_| Ok(())) {
            Err(Failure::Interrupted(_)) => self.resume(setup, store),
            other => other,
        }
    }
    /// Recovery once more, as after a relaunch.
    fn resume(&self, setup: &SetupStore, store: &impl CredentialStore) -> Result<Settled, Failure> {
        match self.recover_with(setup, store, &|_| Ok(()))? {
            Some(intent) if intent.phase == Phase::Settled => Ok(self.settled(&intent)),
            Some(intent) => Err(Failure::Interrupted(intent.transaction)),
            // Recovery proved nothing moved and withdrew the intent.
            None => Err(Failure::Refused(Refusal::Unavailable)),
        }
    }
    /// `point` names crash points for tests: a failing point ends the call at once, as a crash
    /// would, with no cleanup.
    fn replace_with(
        &mut self,
        setup: &SetupStore,
        store: &impl CredentialStore,
        point: impl Fn(&str) -> Result<(), RuntimeError>,
    ) -> Result<Settled, Failure> {
        // Never start a second transaction over an unfinished one: finish or undo it first.
        let unfinished = self
            .read_intent()
            .map_err(|_| Failure::Conflict(vec![INTENT.into()]))?
            .is_some_and(|intent| intent.phase != Phase::Settled);
        let previous = self.recover_with(setup, store, &point)?;
        if unfinished {
            if let Some(intent) = previous {
                // Repeating the action finishes the one already under way.
                return Ok(self.settled(&intent));
            }
        }
        if crate::workspace_upgrade::has_transaction(&self.data, &self.local).map_err(|error| {
            match error {
                RuntimeError::Busy => Refusal::Busy,
                _ => Refusal::UpdateHistory,
            }
        })? {
            return Err(Refusal::UpdateHistory.into());
        }
        let canonical = self.canonical()?;
        self.retire(previous.as_ref(), canonical, setup, store, &point)
    }
    /// Start a first install that never finished again from scratch, at launch, before anything
    /// opens the workspace. Only a `local-vm-v1` that no guest ever booted is retired, and only
    /// while no workspace update has records and no fresh workspace is unfinished; it goes the
    /// way Start a fresh workspace retires a workspace, so a crash here is finished the same way.
    /// Anything else is left exactly as found for the ordinary launch to judge.
    pub(crate) fn discard_unfinished(
        &mut self,
        setup: &SetupStore,
        store: &impl CredentialStore,
    ) -> Result<Option<Settled>, Failure> {
        self.discard_unfinished_with(setup, store, |_| Ok(()))
    }
    fn discard_unfinished_with(
        &mut self,
        setup: &SetupStore,
        store: &impl CredentialStore,
        point: impl Fn(&str) -> Result<(), RuntimeError>,
    ) -> Result<Option<Settled>, Failure> {
        let previous = self
            .read_intent()
            .map_err(|_| Failure::Conflict(vec![INTENT.into()]))?;
        // An unfinished fresh workspace is recovery's to finish, never this.
        if previous
            .as_ref()
            .is_some_and(|intent| intent.phase != Phase::Settled)
        {
            return Ok(None);
        }
        // Update records name the workspace in `local-vm-v1`, including a destination that is
        // still being built; an update store that cannot be read counts as records.
        if !matches!(
            crate::workspace_upgrade::has_transaction(&self.data, &self.local),
            Ok(false)
        ) {
            return Ok(None);
        }
        let canonical = match self.canonical() {
            Ok(Some(canonical)) if canonical.never_booted() => canonical,
            _ => return Ok(None),
        };
        self.retire(previous.as_ref(), Some(canonical), setup, store, &point)
            .map(Some)
    }
    /// Retire `canonical` (or record that nothing was in place) and settle.
    fn retire(
        &mut self,
        previous: Option<&Intent>,
        canonical: Option<Canonical>,
        setup: &SetupStore,
        store: &impl CredentialStore,
        point: &impl Fn(&str) -> Result<(), RuntimeError>,
    ) -> Result<Settled, Failure> {
        let mut earlier: Vec<Earlier> = Vec::new();
        if let Some(previous) = previous {
            for (transaction, directory) in previous.retired() {
                if let Some(directory) = directory {
                    if entry_present(&self.parent, &discarded(transaction)) {
                        earlier.push(Earlier {
                            transaction,
                            directory: directory.clone(),
                        });
                    }
                }
            }
        }
        if earlier.len() >= MAX_EARLIER {
            // Too many unconfirmed deletions to keep track of: never lose sight of one.
            return Err(Refusal::Unavailable.into());
        }
        let transaction = Uuid::new_v4();
        let name = discarded(transaction);
        if entry_present(&self.parent, &name) {
            return Err(Refusal::Layout.into());
        }
        let mut intent = Intent {
            version: 1,
            transaction,
            created_at: now(),
            phase: Phase::Intent,
            directory: canonical.as_ref().map(|c| c.identity.clone()),
            installation: canonical.as_ref().and_then(|c| c.installation),
            ai: None,
            earlier,
        };
        if let Err(reason) = self.persist(&intent) {
            // Nothing moved. The intent may or may not have reached disk; withdrawing it is
            // best effort because an intent that moved nothing never blocks a launch.
            let _ = self.abandon(transaction);
            return Err(reason.into());
        }
        point("intent").map_err(interrupted(transaction))?;
        if let Some(Canonical {
            identity, owner, ..
        }) = canonical
        {
            // Windows refuses to rename a directory while a deny-sharing handle inside it is
            // open, so the workspace lock is released just before the rename there. Elsewhere
            // it is held through the rename, so no worker can take the workspace in between.
            #[cfg(windows)]
            drop(owner);
            // The name must still be the directory whose lock was held.
            if self.identity_of(CANONICAL).ok().flatten().as_ref() != Some(&identity) {
                let _ = self.abandon(transaction);
                return Err(Refusal::Layout.into());
            }
            point("rename").map_err(interrupted(transaction))?;
            // Atomic and same-directory: the workspace is either in place or retired, never
            // copied and never half-moved. An existing name is never replaced.
            if self
                .step("rename", || {
                    image_cache::fs::publish_named(&self.parent, CANONICAL, &name)
                })
                .is_err()
            {
                // Report "nothing changed" only when the identities prove it.
                let unmoved = self.identity_of(CANONICAL).ok().flatten().as_ref()
                    == Some(&identity)
                    && !entry_present(&self.parent, &name);
                if !unmoved {
                    return Err(Failure::Interrupted(transaction));
                }
                let _ = self.abandon(transaction);
                return Err(Refusal::InUse.into());
            }
            #[cfg(not(windows))]
            drop(owner);
            point("renamed").map_err(interrupted(transaction))?;
            // Past here the workspace is set aside: every failure is an interrupted transaction.
            self.step("flush rename", || image_cache::fs::sync(&self.parent))
                .map_err(interrupted(transaction))?;
        }
        intent.phase = Phase::Retired;
        self.persist(&intent).map_err(interrupted(transaction))?;
        point("retired-recorded").map_err(interrupted(transaction))?;
        self.settle(&mut intent, setup, store, point)?;
        Ok(self.settled(&intent))
    }
    fn settled(&self, intent: &Intent) -> Settled {
        Settled {
            transaction: intent.transaction,
            ai: intent.ai.unwrap_or(AiOutcome::Unconfirmed),
            reclaimable: self.reclaimable(intent),
        }
    }
    /// Best effort: anything under a retired name is reported, whatever it is.
    fn reclaimable(&self, intent: &Intent) -> Vec<String> {
        intent
            .retired()
            .into_iter()
            .map(|(transaction, _)| discarded(transaction))
            .filter(|name| entry_present(&self.parent, name))
            .collect()
    }
    /// Names relative to the local data folder, as display paths.
    pub(crate) fn display_paths(&self, names: &[String]) -> Vec<String> {
        display_names(&self.local, names)
    }
    /// Delete the retired directories of a settled transaction, relative to the local data
    /// folder and without following links, and only while each is still the recorded
    /// directory. Anything else under a retired name, or a directory that cannot be deleted,
    /// is left in place and reported as reclaimable space; this never fails the action.
    pub(crate) fn clean(&self) -> Result<Vec<String>, Refusal> {
        self.clean_with(|parent, name| parent.remove_dir_all(name))
    }
    fn clean_with(
        &self,
        remove: impl Fn(&Dir, &str) -> std::io::Result<()>,
    ) -> Result<Vec<String>, Refusal> {
        let Some(mut intent) = self.read_intent()? else {
            return Ok(vec![]);
        };
        if intent.phase != Phase::Settled {
            return Err(Refusal::Busy);
        }
        for (transaction, recorded) in intent.retired() {
            let name = discarded(transaction);
            if recorded.is_some() && self.identity_of(&name).ok().flatten().as_ref() == recorded {
                let _ = self.step(&format!("delete {name}"), || remove(&self.parent, &name));
            }
        }
        let _ = image_cache::fs::sync(&self.parent);
        let remaining = self.reclaimable(&intent);
        let earlier: Vec<Earlier> = intent
            .earlier
            .iter()
            .filter(|e| remaining.contains(&discarded(e.transaction)))
            .cloned()
            .collect();
        if earlier != intent.earlier {
            intent.earlier = earlier;
            // A longer list than needed is harmless: entries are checked again by name.
            let _ = self.persist(&intent);
        }
        Ok(remaining)
    }
}
/// Names relative to the local data folder, as display paths.
pub(crate) fn display_names(local: &Path, names: &[String]) -> Vec<String> {
    names
        .iter()
        .map(|name| {
            name.split('/')
                .fold(local.to_owned(), |path, part| path.join(part))
                .display()
                .to_string()
        })
        .collect()
}
fn read_intent(directory: &Dir) -> Result<Option<Intent>, Refusal> {
    let Some(bytes) = read_small(directory, "intent.json", MAX_INTENT)? else {
        return Ok(None);
    };
    let intent: Intent = serde_json::from_slice(&bytes).map_err(|_| Refusal::Layout)?;
    if !intent.valid() {
        return Err(Refusal::Layout);
    }
    Ok(Some(intent))
}
fn stored_intent(local: &Path) -> Result<Option<Intent>, Refusal> {
    let store = local.join(STORE);
    match fs::symlink_metadata(&store) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Refusal::Unavailable),
        Ok(_) => {}
    }
    let directory =
        Dir::open_ambient_dir(&store, cap_std::ambient_authority()).map_err(|_| Refusal::Layout)?;
    read_intent(&directory)
}
/// Launch gate, read-only and lock-free: an ordinary start may proceed only when no fresh
/// transaction is unfinished. Cleanup of a settled transaction may run concurrently. An intent
/// that provably moved nothing (its directory still in place, nothing retired) does not block:
/// a refusal whose withdrawal failed must not keep the workspace from starting.
pub(crate) fn admit(local: &Path) -> Result<(), RuntimeError> {
    let intent = match stored_intent(local) {
        Ok(None) => return Ok(()),
        Ok(Some(intent)) => intent,
        Err(_) => return Err(RuntimeError::RetainedState),
    };
    if intent.phase == Phase::Settled {
        return Ok(());
    }
    if intent.phase == Phase::Intent {
        if let Some(directory) = &intent.directory {
            let parent = Dir::open_ambient_dir(local, cap_std::ambient_authority())?;
            if entry_identity(&parent, CANONICAL).ok().flatten().as_ref() == Some(directory)
                && !entry_present(&parent, &discarded(intent.transaction))
            {
                return Ok(());
            }
        }
    }
    Err(RuntimeError::RetainedState)
}
/// What a panic during the transaction left, judged as the launch gate judges it.
pub(crate) fn after_panic(local: &Path) -> Failure {
    if admit(local).is_ok() {
        return Failure::Refused(Refusal::Unavailable);
    }
    match stored_intent(local) {
        Ok(Some(intent)) => Failure::Interrupted(intent.transaction),
        _ => Failure::Conflict(vec![INTENT.into()]),
    }
}
fn unsettled(local: &Path) -> bool {
    stored_intent(local).map_or(true, |intent| {
        intent.is_some_and(|intent| intent.phase != Phase::Settled)
    })
}
/// Denial-only check before the VM is stopped: a refusal here touches nothing. Finishing an
/// interrupted transaction is never refused here.
pub(crate) fn preflight(data: &Path, local: &Path) -> Option<Refusal> {
    if unsettled(local) {
        return None;
    }
    match crate::workspace_upgrade::has_transaction(data, local) {
        Ok(false) => None,
        Ok(true) => Some(Refusal::UpdateHistory),
        Err(RuntimeError::Busy) => Some(Refusal::Busy),
        Err(_) => Some(Refusal::UpdateHistory),
    }
}
/// The same answer as `preflight`, for periodic status: lock-free, so polling never makes a
/// workspace update or this action busy. It reads which records exist, not their contents; an
/// unreadable update store counts as history, as `preflight` counts it.
pub(crate) fn blocked(local: &Path) -> Option<Refusal> {
    if unsettled(local) {
        return None;
    }
    match fs::read_dir(local.join(crate::workspace_upgrade::STORE)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(Refusal::UpdateHistory),
        Ok(entries) => entries
            .map(|entry| {
                entry.map(|entry| {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    name == "intent.json"
                        || name.starts_with("completed-") && name.ends_with(".json")
                })
            })
            .any(|history| history.unwrap_or(true))
            .then_some(Refusal::UpdateHistory),
    }
}
// Used by the native factory, which is built only for the shipping platforms.
#[cfg_attr(
    not(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )),
    allow(dead_code)
)]
/// Whether the cache already holds the image with this manifest, ready to install. The cache
/// verifies it again before use; this only decides the confirmation's wording and whether to
/// check the download first.
pub(crate) fn image_cached(cache: &Path, manifest_sha256: &str) -> bool {
    if manifest_sha256.len() != 64
        || !manifest_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return false;
    }
    fs::symlink_metadata(
        cache
            .join("images")
            .join("v2")
            .join(manifest_sha256)
            .join("ready.json"),
    )
    .is_ok_and(|m| m.is_file())
}
// Used by the native factory, which is built only for the shipping platforms.
#[cfg_attr(
    not(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )),
    allow(dead_code)
)]
/// From a pinned manifest: the download (compressed image plus boot files) and the free space
/// the cache asks for before it writes (the download plus the decoded disk).
pub(crate) fn download_sizes(manifest: &[u8]) -> Option<(u64, u64)> {
    let value: serde_json::Value = serde_json::from_slice(manifest).ok()?;
    let download = value["artifact"]["compressedBytes"]
        .as_u64()?
        .checked_add(value["directBoot"]["kernel"]["bytes"].as_u64()?)?
        .checked_add(value["directBoot"]["initrd"]["bytes"].as_u64()?)?;
    let needed = download.checked_add(value["artifact"]["uncompressedBytes"].as_u64()?)?;
    Some((download, needed))
}
// Used by the native factory, which is built only for the shipping platforms.
#[cfg_attr(
    not(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )),
    allow(dead_code)
)]
/// Free space for a download, as first install's cache counts it. Unknown space is not a
/// refusal: the cache checks again before it writes.
pub(crate) fn space_check(available: Option<u64>, needed: u64) -> Result<(), Refusal> {
    match available {
        Some(available) if available < needed => Err(Refusal::NoSpace),
        _ => Ok(()),
    }
}
// Used by the native factory, which is built only for the shipping platforms.
#[cfg_attr(
    not(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )),
    allow(dead_code)
)]
/// Free space where the cache lives: the cache folder, or the local data folder before it exists.
pub(crate) fn available_space(cache: &Path) -> Option<u64> {
    let dir = Dir::open_ambient_dir(cache, cap_std::ambient_authority())
        .ok()
        .or_else(|| Dir::open_ambient_dir(cache.parent()?, cap_std::ambient_authority()).ok())?;
    image_cache::available_space(&dir).ok()
}
// Used by the native factory, which is built only for the shipping platforms.
#[cfg_attr(
    not(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )),
    allow(dead_code)
)]
/// Before anything is stopped or retired, when first install must download the image: refuse
/// when the download cannot be reached or does not fit.
pub(crate) async fn download_check(
    cache: &Path,
    pin: &crate::runtime::image_types::ImagePin,
    needed: u64,
) -> Result<(), Refusal> {
    space_check(available_space(cache), needed)?;
    let client = crate::runtime::acquire::ImageHttpClient::new(pin).map_err(unavailable)?;
    let (_closed, open) = tokio::sync::watch::channel(false);
    match tokio::time::timeout(std::time::Duration::from_secs(30), client.manifest(open)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(crate::runtime::image_types::ImageError::NoSpace)) => Err(Refusal::NoSpace),
        Ok(Err(crate::runtime::image_types::ImageError::Network(_))) | Err(_) => {
            Err(Refusal::Offline)
        }
        Ok(Err(_)) => Err(Refusal::Unavailable),
    }
}
/// Runs at launch, before the VM is configured. Finishes or undoes an interrupted transaction
/// and reports one finished now, a settled one whose retired directory still needs deleting, or
/// one that could not be finished. A settled transaction never blocks a launch, whatever its
/// leftovers; only an unfinished one does, and then the reply says what Bridge found.
pub fn recover_startup(
    data: &Path,
    local: &Path,
    setup: &SetupStore,
    store: &impl CredentialStore,
) -> Result<Option<FreshStatusDto>, RuntimeError> {
    if !local.join(STORE).try_exists()? {
        return Ok(None);
    }
    let owner = match FreshWorkspace::open(data, local) {
        Ok(owner) => owner,
        // Without the owner nothing is reconciled; launch is still admitted when it may be.
        Err(_) => {
            return match admit(local) {
                Ok(()) => Ok(None),
                Err(error) => Err(error),
            };
        }
    };
    let interrupted = unsettled(local);
    match owner.recover_with(setup, store, &|_| Ok(())) {
        Ok(None) => Ok(None),
        Ok(Some(intent)) if intent.phase == Phase::Settled => {
            let settled = owner.settled(&intent);
            if !interrupted && settled.reclaimable.is_empty() {
                return Ok(None);
            }
            let reclaimable = owner.display_paths(&settled.reclaimable);
            Ok(Some(FreshStatusDto::Replaced {
                transaction: settled.transaction,
                ai: settled.ai,
                cleaning: !reclaimable.is_empty(),
                reclaimable,
            }))
        }
        Ok(Some(intent)) => Ok(Some(FreshStatusDto::Interrupted {
            transaction: intent.transaction,
        })),
        Err(Failure::Interrupted(transaction)) => {
            Ok(Some(FreshStatusDto::Interrupted { transaction }))
        }
        Err(Failure::Conflict(found)) => Ok(Some(FreshStatusDto::Conflict {
            found: owner.display_paths(&found),
        })),
        Err(Failure::Refused(_)) => Err(RuntimeError::RetainedState),
    }
}
/// At launch, after `recover_startup` and before the workspace is configured: a first install
/// that a crash or a forced quit interrupted is started again from scratch rather than left as a
/// workspace nothing can open. `Ok` says whether its retired directory waits for deletion, which
/// is the same background cleanup a fresh workspace uses; nothing retired is `Ok(false)`. A
/// retirement that could not finish is reported as a fresh workspace that did not finish, which
/// keeps the workspace stopped until Bridge finishes it, as after any interrupted fresh workspace.
pub fn discard_unfinished_install(
    data: &Path,
    local: &Path,
    setup: &SetupStore,
    store: &impl CredentialStore,
) -> Result<bool, FreshStatusDto> {
    if !matches!(fs::symlink_metadata(local.join(CANONICAL)), Ok(m) if m.is_dir()) {
        return Ok(false);
    }
    let Ok(mut owner) = FreshWorkspace::open(data, local) else {
        return Ok(false);
    };
    match owner.discard_unfinished(setup, store) {
        Ok(Some(settled)) => Ok(!settled.reclaimable.is_empty()),
        Ok(None) | Err(Failure::Refused(_)) => Ok(false),
        Err(Failure::Interrupted(transaction)) => Err(FreshStatusDto::Interrupted { transaction }),
        Err(Failure::Conflict(found)) => Err(FreshStatusDto::Conflict {
            found: owner.display_paths(&found),
        }),
    }
}
#[cfg(test)]
#[path = "workspace_fresh_tests.rs"]
mod tests;
