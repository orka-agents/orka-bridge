//! Start a fresh workspace: retire the local workspace on this computer so that first install
//! builds a new one of the same release from the cached image.
//!
//! One durable intent in `workspace-fresh-v1` names the directory being retired by its file
//! identity. The directory is renamed, never copied or deleted in place, to a Bridge-owned
//! sibling named from the transaction (`discarded-<transaction>`), so every crash leaves either
//! the original `local-vm-v1` intact or a retired directory the intent recognises. Recovery
//! reconciles identities, not the last phase label. Only a settled transaction admits an ordinary
//! launch, and the retired directory is deleted only after the transaction is settled.
//!
//! The caller owns lifecycle quiescence: the VM is stopped and its native custody released before
//! this backend runs. Acquiring the workspace's own owner lock here proves no native worker still
//! holds it. No path is renderer input.
use crate::runtime::{
    credential_store::CredentialStore,
    identity::{file_identity, FileIdentity},
    image_cache,
    journal::InstallationRecord,
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
const MAX_INTENT: u64 = 16 * 1024;
/// The most retired directories whose deletion stays unconfirmed and is reported.
const MAX_EARLIER: usize = 8;

/// Why a fresh workspace was not started. In every case the workspace on this computer is left
/// exactly as it was, apart from the VM having been stopped.
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
    /// Removal was not confirmed; the saved record is kept so the existing Reset can retry it.
    Kept,
}
/// Nonsecret projection for the renderer. Paths are native-selected and display-only.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
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
    Refused {
        reason: Refusal,
    },
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
    /// Earlier settled transactions whose retired directory still exists.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    earlier: Vec<Uuid>,
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
                .all(|t| !t.is_nil() && *t != self.transaction)
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
    /// The workspace's own owner lock, proving no native worker holds it. Dropped before the
    /// rename: Windows denies renaming a directory while a deny-sharing handle inside is open.
    _owner: crate::runtime::journal::OwnerLock,
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
        })
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
        let store = self.local.join(STORE);
        let pending = store.join(format!("intent-{}.next", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pending)
            .map_err(unavailable)?;
        file.write_all(&bytes).map_err(unavailable)?;
        file.sync_all().map_err(unavailable)?;
        drop(file);
        crate::runtime::journal::replace(&pending, &store.join("intent.json"))
            .map_err(unavailable)?;
        image_cache::fs::sync(&self.directory).map_err(unavailable)
    }
    /// Forget an intent that never moved anything, so a refusal leaves no transaction behind.
    fn abandon(&self, transaction: Uuid) -> Result<(), Refusal> {
        if self.read_intent()?.is_some_and(|intent| {
            intent.transaction == transaction && intent.phase == Phase::Intent
        }) {
            self.directory
                .remove_file("intent.json")
                .map_err(unavailable)?;
            image_cache::fs::sync(&self.directory).map_err(unavailable)?;
        }
        Ok(())
    }
    /// The identity of a directory entry in the local data folder, without following links.
    fn identity_of(&self, name: &str) -> Result<Option<FileIdentity>, Refusal> {
        match self.parent.symlink_metadata(name) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(Refusal::Layout),
            Err(_) => return Err(Refusal::Unavailable),
        }
        let dir = self
            .parent
            .open_dir_nofollow(name)
            .map_err(|_| Refusal::Layout)?;
        let file = dir.into_std_file();
        image_cache::not_reparse(&file).map_err(|_| Refusal::Layout)?;
        file_identity(&file).map(Some).map_err(unavailable)
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
        let installation = match read_small(&dir, "record.json", 32 * 1024)? {
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
                Some(record.installation_id)
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
                None
            }
        };
        if self.identity_of(CANONICAL)?.as_ref() != Some(&identity) {
            return Err(Refusal::Layout);
        }
        Ok(Some(Canonical {
            identity,
            installation,
            _owner: owner,
        }))
    }
    /// Finish or undo an interrupted transaction. Returns the current intent, if any remains.
    /// Nothing here guesses: an unexpected combination of directories is a conflict and is left
    /// exactly as found.
    fn recover_with(
        &self,
        setup: &SetupStore,
        store: &impl CredentialStore,
        point: &impl Fn(&str) -> Result<(), RuntimeError>,
    ) -> Result<Option<Intent>, Refusal> {
        let Some(mut intent) = self.read_intent()? else {
            return Ok(None);
        };
        if intent.phase == Phase::Settled {
            return Ok(Some(intent));
        }
        let name = discarded(intent.transaction);
        let canonical = self.identity_of(CANONICAL)?;
        let retired = self.identity_of(&name)?;
        match (&intent.directory, &canonical, &retired) {
            // The rename never happened: nothing moved, so the intent is simply withdrawn.
            (Some(directory), Some(current), None)
                if current == directory && intent.phase == Phase::Intent =>
            {
                self.abandon(intent.transaction)?;
                return Ok(None);
            }
            (Some(directory), None, Some(moved)) if moved == directory => {}
            (None, None, None) => {}
            _ => return Err(Refusal::Layout),
        }
        if intent.phase == Phase::Intent {
            intent.phase = Phase::Retired;
            self.persist(&intent)?;
            point("retired-recorded").map_err(unavailable)?;
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
    ) -> Result<(), Refusal> {
        // The workspace that owned the saved AI connection is gone from its place. Forget it as
        // the installation-mismatch Reset does. An unconfirmed removal keeps the record, so the
        // existing Reset offers it again; it never blocks the new workspace.
        let ai = match setup.reset_retired_model(store) {
            Ok(true) => AiOutcome::Removed,
            Ok(false) => AiOutcome::Absent,
            Err(_) => AiOutcome::Kept,
        };
        point("ai").map_err(unavailable)?;
        intent.ai = Some(ai);
        intent.phase = Phase::Settled;
        self.persist(intent)?;
        point("settled").map_err(unavailable)
    }
    pub(crate) fn replace(
        &mut self,
        setup: &SetupStore,
        store: &impl CredentialStore,
    ) -> Result<Settled, Refusal> {
        self.replace_with(setup, store, |_| Ok(()))
    }
    fn replace_with(
        &mut self,
        setup: &SetupStore,
        store: &impl CredentialStore,
        point: impl Fn(&str) -> Result<(), RuntimeError>,
    ) -> Result<Settled, Refusal> {
        // Never start a second transaction over an unfinished one: finish or undo it first.
        let interrupted = self
            .read_intent()?
            .is_some_and(|intent| intent.phase != Phase::Settled);
        let previous = self.recover_with(setup, store, &point)?;
        if interrupted {
            if let Some(intent) = previous {
                // Repeating the action finishes the one already under way.
                return self.settled(&intent);
            }
        }
        if crate::workspace_upgrade::has_transaction(&self.data, &self.local).map_err(|error| {
            match error {
                RuntimeError::Busy => Refusal::Busy,
                _ => Refusal::UpdateHistory,
            }
        })? {
            return Err(Refusal::UpdateHistory);
        }
        let mut earlier: Vec<Uuid> = Vec::new();
        if let Some(previous) = &previous {
            for transaction in previous
                .earlier
                .iter()
                .copied()
                .chain(Some(previous.transaction))
            {
                if self.identity_of(&discarded(transaction))?.is_some() {
                    earlier.push(transaction);
                }
            }
        }
        if earlier.len() >= MAX_EARLIER {
            // Too many unconfirmed deletions to keep track of: never lose sight of one.
            return Err(Refusal::Unavailable);
        }
        let canonical = self.canonical()?;
        let transaction = Uuid::new_v4();
        let name = discarded(transaction);
        if self.identity_of(&name)?.is_some() {
            return Err(Refusal::Layout);
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
        self.persist(&intent)?;
        point("intent").map_err(unavailable)?;
        if let Some(canonical) = canonical {
            let Canonical {
                identity, _owner, ..
            } = canonical;
            drop(_owner);
            // The name must still be the directory whose lock was held.
            if self.identity_of(CANONICAL)?.as_ref() != Some(&identity) {
                self.abandon(transaction)?;
                return Err(Refusal::Layout);
            }
            point("rename").map_err(unavailable)?;
            // Atomic and same-directory: the workspace is either in place or retired, never
            // copied and never half-moved. An existing name is never replaced.
            if image_cache::fs::publish_named(&self.parent, CANONICAL, &name).is_err() {
                self.abandon(transaction)?;
                return Err(Refusal::InUse);
            }
            point("renamed").map_err(unavailable)?;
            // A failed flush leaves the intent at Intent; recovery decides by identity.
            image_cache::fs::sync(&self.parent).map_err(unavailable)?;
        }
        intent.phase = Phase::Retired;
        self.persist(&intent)?;
        point("retired-recorded").map_err(unavailable)?;
        self.settle(&mut intent, setup, store, &point)?;
        self.settled(&intent)
    }
    fn settled(&self, intent: &Intent) -> Result<Settled, Refusal> {
        Ok(Settled {
            transaction: intent.transaction,
            ai: intent.ai.ok_or(Refusal::Unavailable)?,
            reclaimable: self.reclaimable(intent)?,
        })
    }
    fn reclaimable(&self, intent: &Intent) -> Result<Vec<String>, Refusal> {
        let mut names = Vec::new();
        for transaction in intent
            .earlier
            .iter()
            .copied()
            .chain(Some(intent.transaction))
        {
            let name = discarded(transaction);
            if self.identity_of(&name)?.is_some() {
                names.push(name);
            }
        }
        Ok(names)
    }
    /// The settled transaction's retired directories, as display paths.
    pub(crate) fn display_paths(&self, names: &[String]) -> Vec<String> {
        names
            .iter()
            .map(|name| self.local.join(name).display().to_string())
            .collect()
    }
    /// Delete the retired directories of a settled transaction, relative to the local data
    /// folder and without following links. A directory that cannot be deleted is left in
    /// place and reported as reclaimable space; this never fails the action.
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
        for name in self.reclaimable(&intent)? {
            let _ = remove(&self.parent, &name);
        }
        let _ = image_cache::fs::sync(&self.parent);
        let remaining = self.reclaimable(&intent)?;
        let earlier: Vec<Uuid> = intent
            .earlier
            .iter()
            .copied()
            .filter(|t| remaining.contains(&discarded(*t)))
            .collect();
        if earlier != intent.earlier {
            intent.earlier = earlier;
            self.persist(&intent)?;
        }
        Ok(remaining)
    }
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
/// Launch gate, read-only and lock-free: an ordinary start may proceed only when no fresh
/// transaction is unfinished. Cleanup of a settled transaction may run concurrently.
pub(crate) fn admit(local: &Path) -> Result<(), RuntimeError> {
    let store = local.join(STORE);
    if !store.try_exists()? {
        return Ok(());
    }
    let directory = Dir::open_ambient_dir(&store, cap_std::ambient_authority())?;
    match read_intent(&directory) {
        Ok(None) => Ok(()),
        Ok(Some(intent)) if intent.phase == Phase::Settled => Ok(()),
        _ => Err(RuntimeError::RetainedState),
    }
}
/// Denial-only check before the VM is stopped: a refusal here touches nothing.
pub(crate) fn preflight(data: &Path, local: &Path) -> Option<Refusal> {
    match crate::workspace_upgrade::has_transaction(data, local) {
        Ok(false) => None,
        Ok(true) => Some(Refusal::UpdateHistory),
        Err(RuntimeError::Busy) => Some(Refusal::Busy),
        Err(_) => Some(Refusal::UpdateHistory),
    }
}
/// Runs at launch, before the VM is configured. Finishes or undoes an interrupted transaction
/// and reports one finished now, or a settled one whose retired directory still needs deleting.
pub fn recover_startup(
    data: &Path,
    local: &Path,
    setup: &SetupStore,
    store: &impl CredentialStore,
) -> Result<Option<FreshStatusDto>, RuntimeError> {
    if !local.join(STORE).try_exists()? {
        return Ok(None);
    }
    let failed = |_| RuntimeError::RetainedState;
    let owner = FreshWorkspace::open(data, local).map_err(failed)?;
    let interrupted = owner
        .read_intent()
        .map_err(failed)?
        .is_some_and(|intent| intent.phase != Phase::Settled);
    let Some(intent) = owner
        .recover_with(setup, store, &|_| Ok(()))
        .map_err(failed)?
    else {
        return Ok(None);
    };
    let settled = owner.settled(&intent).map_err(failed)?;
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
#[cfg(test)]
#[path = "workspace_fresh_tests.rs"]
mod tests;
