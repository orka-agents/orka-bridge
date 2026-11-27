use super::*;
use crate::runtime::{credential_store::StoreError, journal::Journal, types::BackendKind};
use std::sync::Mutex;

/// A credential store that records which slots were deleted and can refuse deletion.
#[derive(Default)]
struct Store {
    deleted: Mutex<Vec<Uuid>>,
    fail: bool,
}
impl CredentialStore for Store {
    fn save(&self, _: Uuid, _: &[u8]) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
    fn load(&self, _: Uuid) -> Result<bridge_runtime_protocol::CredentialPayload, StoreError> {
        Err(StoreError::Unavailable)
    }
    fn delete(&self, slot: Uuid) -> Result<(), StoreError> {
        if self.fail {
            return Err(StoreError::Unavailable);
        }
        self.deleted.lock().unwrap().push(slot);
        Ok(())
    }
}
struct Scene {
    temp: tempfile::TempDir,
    data: PathBuf,
    local: PathBuf,
    setup: SetupStore,
    slot: Uuid,
    installation: Uuid,
}
/// What must be unchanged when Bridge refuses: the directory itself, every file in it, and the
/// saved AI connection.
#[derive(Debug, PartialEq)]
struct Intact {
    identity: FileIdentity,
    files: Vec<(String, Vec<u8>)>,
    model: Option<Vec<u8>>,
    account: Option<Vec<u8>>,
}
impl Scene {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("roaming");
        let local = temp.path().join("local");
        fs::create_dir_all(data.join("setup-v1")).unwrap();
        fs::create_dir(&local).unwrap();
        // The local workspace is the saved setup choice.
        fs::write(data.join("setup-v1/local-vm"), b"").unwrap();
        let journal = Journal::create(&local.join(CANONICAL), BackendKind::Apple, vec![1]).unwrap();
        let installation = journal.record().installation_id;
        drop(journal);
        fs::write(local.join("local-vm-v1/efi.vars"), b"firmware").unwrap();
        fs::create_dir(local.join("local-vm-v1/image")).unwrap();
        fs::write(local.join("local-vm-v1/image/os.raw"), b"disk").unwrap();
        fs::create_dir(local.join("image-cache-v1")).unwrap();
        fs::write(local.join("image-cache-v1/kept"), b"cache").unwrap();
        let slot = Uuid::new_v4();
        fs::write(
            data.join("setup-v1/model-v1"),
            serde_json::to_vec(&serde_json::json!({"slot":slot,"installation":installation,"phase":"stored","nonce":null,"connection":null})).unwrap(),
        )
        .unwrap();
        fs::write(
            data.join("setup-v1/model-account-v1"),
            serde_json::to_vec(&serde_json::json!({"slot":slot,"login":"octocat"})).unwrap(),
        )
        .unwrap();
        let setup = SetupStore::with_upgrade_root(Some(data.clone()), Some(local.clone()));
        Self {
            temp,
            data,
            local,
            setup,
            slot,
            installation,
        }
    }
    fn owner(&self) -> FreshWorkspace {
        FreshWorkspace::open(&self.data, &self.local).unwrap()
    }
    fn replace(&self, store: &Store, stop_at: Option<&str>) -> Result<Settled, Refusal> {
        self.owner().replace_with(&self.setup, store, |point| {
            if Some(point) == stop_at {
                Err(RuntimeError::Closed)
            } else {
                Ok(())
            }
        })
    }
    fn intact(&self) -> Intact {
        Intact {
            identity: directory_identity(&self.local.join(CANONICAL)),
            files: files(&self.local.join(CANONICAL)),
            model: fs::read(self.data.join("setup-v1/model-v1")).ok(),
            account: fs::read(self.data.join("setup-v1/model-account-v1")).ok(),
        }
    }
    fn discarded(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(&self.local)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.starts_with("discarded-"))
            .collect();
        names.sort();
        names
    }
    fn intent(&self) -> Option<Intent> {
        read_intent(
            &Dir::open_ambient_dir(self.local.join(STORE), cap_std::ambient_authority()).unwrap(),
        )
        .unwrap()
    }
    fn assert_kept_outside_workspace(&self) {
        assert_eq!(
            fs::read(self.local.join("image-cache-v1/kept")).unwrap(),
            b"cache"
        );
        assert!(self.data.join("setup-v1/local-vm").exists());
    }
}
fn directory_identity(path: &Path) -> FileIdentity {
    file_identity(&File::open(path).unwrap()).unwrap()
}
fn files(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let name = path.strip_prefix(root).unwrap().display().to_string();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, out);
            } else {
                out.push((name, fs::read(&path).unwrap_or_default()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

#[test]
fn replace_retires_the_workspace_resets_the_saved_connection_and_admits_first_install() {
    let scene = Scene::new();
    let before = scene.intact();
    crate::workspace_upgrade::admit(&scene.data, &scene.local.join(CANONICAL)).unwrap();
    let store = Store::default();
    let settled = scene.replace(&store, None).unwrap();
    assert_eq!(settled.ai, AiOutcome::Removed);
    let name = discarded(settled.transaction);
    assert_eq!(settled.reclaimable, vec![name.clone()]);
    // Moved, not copied: the same directory, with every file, under a Bridge-owned name.
    assert!(!scene.local.join(CANONICAL).exists());
    assert_eq!(
        directory_identity(&scene.local.join(&name)),
        before.identity
    );
    assert_eq!(files(&scene.local.join(&name)), before.files);
    // The saved connection is gone exactly as Reset removes it, with its login record.
    assert_eq!(*store.deleted.lock().unwrap(), vec![scene.slot]);
    assert!(!scene.data.join("setup-v1/model-v1").exists());
    assert!(!scene.data.join("setup-v1/model-account-v1").exists());
    // The setup choice and the image cache are kept.
    scene.assert_kept_outside_workspace();
    let intent = scene.intent().unwrap();
    assert_eq!(intent.phase, Phase::Settled);
    assert_eq!(intent.installation, Some(scene.installation));
    // First install may now create the new installation, with a new installation id.
    admit(&scene.local).unwrap();
    crate::workspace_upgrade::admit(&scene.data, &scene.local.join(CANONICAL)).unwrap();
    let fresh = Journal::create(&scene.local.join(CANONICAL), BackendKind::Apple, vec![1]).unwrap();
    assert_ne!(fresh.record().installation_id, scene.installation);
    drop(fresh);
    // Background deletion removes only the retired directory.
    assert!(scene.owner().clean().unwrap().is_empty());
    assert!(scene.discarded().is_empty());
    assert!(scene.local.join(CANONICAL).join("record.json").exists());
    scene.assert_kept_outside_workspace();
}

#[test]
fn a_crash_at_every_point_leaves_the_old_workspace_or_a_state_recovery_finishes() {
    for point in [
        "intent",
        "rename",
        "renamed",
        "retired-recorded",
        "ai",
        "settled",
    ] {
        let scene = Scene::new();
        let before = scene.intact();
        let store = Store::default();
        assert!(scene.replace(&store, Some(point)).is_err(), "{point}");
        // The process is gone: only what reached disk remains.
        let moved = !matches!(point, "intent" | "rename");
        if moved {
            assert!(!scene.local.join(CANONICAL).exists(), "{point}");
            assert_eq!(scene.discarded().len(), 1, "{point}");
            assert_eq!(
                directory_identity(&scene.local.join(&scene.discarded()[0])),
                before.identity,
                "{point}"
            );
        } else {
            assert_eq!(scene.intact(), before, "{point}");
            assert!(scene.discarded().is_empty(), "{point}");
        }
        // Until it settles, no workspace may be opened or created in its place.
        let settled_on_disk = point == "settled";
        assert_eq!(admit(&scene.local).is_ok(), settled_on_disk, "{point}");
        assert_eq!(
            crate::workspace_upgrade::admit(&scene.data, &scene.local.join(CANONICAL)).is_ok(),
            settled_on_disk,
            "{point}"
        );
        // Next launch: an unmoved workspace is kept and the intent withdrawn; a moved one is
        // finished, including the AI reset, and reported.
        let recovered = recover_startup(&scene.data, &scene.local, &scene.setup, &store).unwrap();
        admit(&scene.local).unwrap();
        if moved {
            let Some(FreshStatusDto::Replaced {
                ai,
                reclaimable,
                cleaning,
                ..
            }) = recovered
            else {
                panic!("{point}: {recovered:?}")
            };
            // A crash after removal but before it was recorded reports nothing saved: the
            // slot and record are gone either way.
            assert!(
                matches!(ai, AiOutcome::Removed | AiOutcome::Absent),
                "{point}"
            );
            assert_eq!(reclaimable.len(), 1, "{point}");
            assert!(cleaning, "{point}");
            assert!(!scene.data.join("setup-v1/model-v1").exists(), "{point}");
            assert_eq!(*store.deleted.lock().unwrap(), vec![scene.slot], "{point}");
            assert_eq!(scene.intent().unwrap().phase, Phase::Settled, "{point}");
        } else {
            assert_eq!(recovered, None, "{point}");
            assert_eq!(scene.intact(), before, "{point}");
            assert!(scene.intent().is_none(), "{point}");
            assert!(store.deleted.lock().unwrap().is_empty(), "{point}");
        }
        scene.assert_kept_outside_workspace();
        // Recovery is idempotent: a settled transaction with its directory still present
        // keeps being reported, and nothing is removed twice.
        let again = recover_startup(&scene.data, &scene.local, &scene.setup, &store).unwrap();
        assert_eq!(again.is_some(), moved, "{point}");
        assert!(store.deleted.lock().unwrap().len() <= 1, "{point}");
    }
}

#[test]
fn a_rename_lost_with_an_unflushed_directory_is_undone_and_the_workspace_kept() {
    let scene = Scene::new();
    let before = scene.intact();
    let store = Store::default();
    // Stop just after the rename, then put the directory back as a lost, unflushed rename.
    assert!(scene.replace(&store, Some("renamed")).is_err());
    let name = scene.discarded().pop().unwrap();
    fs::rename(scene.local.join(&name), scene.local.join(CANONICAL)).unwrap();
    assert_eq!(
        recover_startup(&scene.data, &scene.local, &scene.setup, &store).unwrap(),
        None
    );
    assert_eq!(scene.intact(), before);
    assert!(scene.intent().is_none());
    assert!(store.deleted.lock().unwrap().is_empty());
}

#[test]
fn a_held_workspace_lock_refuses_and_leaves_everything_in_place() {
    let scene = Scene::new();
    let before = scene.intact();
    // A live native worker holds the journal; the VM may still be using its disk.
    let live = Journal::open(&scene.local.join(CANONICAL)).unwrap();
    assert_eq!(
        scene.replace(&Store::default(), None).map(|_| ()),
        Err(Refusal::InUse)
    );
    drop(live);
    assert_eq!(scene.intact(), before);
    assert!(scene.intent().is_none());
    assert!(scene.discarded().is_empty());
}

#[test]
fn a_directory_that_is_not_a_workspace_is_refused_untouched() {
    type Arrange = fn(&Path);
    let cases: [(&str, Arrange); 5] = [
        ("files without a record", |root| {
            fs::remove_file(root.join("record.json")).unwrap();
        }),
        ("an unreadable record", |root| {
            fs::write(root.join("record.json"), b"{}").unwrap();
        }),
        ("no owner lock", |root| {
            fs::remove_file(root.join("owner.lock")).unwrap();
        }),
        ("a directory as the state disk", |root| {
            fs::remove_file(root.join("state.disk")).unwrap();
            fs::create_dir(root.join("state.disk")).unwrap();
        }),
        ("no state disk", |root| {
            fs::remove_file(root.join("state.disk")).unwrap();
        }),
    ];
    for (name, arrange) in cases {
        let scene = Scene::new();
        arrange(&scene.local.join(CANONICAL));
        let before = scene.intact();
        assert_eq!(
            scene.replace(&Store::default(), None).map(|_| ()),
            Err(Refusal::Layout),
            "{name}"
        );
        assert_eq!(scene.intact(), before, "{name}");
        assert!(scene.intent().is_none(), "{name}");
        assert!(scene.discarded().is_empty(), "{name}");
    }
    // A plain file where the workspace should be.
    let scene = Scene::new();
    fs::remove_dir_all(scene.local.join(CANONICAL)).unwrap();
    fs::write(scene.local.join(CANONICAL), b"not a workspace").unwrap();
    assert_eq!(
        scene.replace(&Store::default(), None).map(|_| ()),
        Err(Refusal::Layout)
    );
    assert_eq!(
        fs::read(scene.local.join(CANONICAL)).unwrap(),
        b"not a workspace"
    );
    assert!(scene.data.join("setup-v1/model-v1").exists());
}

#[test]
fn a_record_less_creation_is_a_workspace_bridge_can_retire() {
    // Journal::create interrupted before its first record: the lock and an empty state disk.
    let scene = Scene::new();
    let root = scene.local.join(CANONICAL);
    fs::remove_dir_all(&root).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(root.join("owner.lock"), b"").unwrap();
    fs::write(root.join("state.disk"), b"").unwrap();
    let settled = scene.replace(&Store::default(), None).unwrap();
    assert_eq!(scene.intent().unwrap().installation, None);
    assert_eq!(settled.reclaimable.len(), 1);
    // But a record-less directory holding anything more is not.
    let scene = Scene::new();
    let root = scene.local.join(CANONICAL);
    fs::remove_file(root.join("record.json")).unwrap();
    fs::remove_dir_all(root.join("image")).unwrap();
    fs::remove_file(root.join("efi.vars")).unwrap();
    fs::write(root.join("state.disk"), b"written").unwrap();
    assert_eq!(
        scene.replace(&Store::default(), None).map(|_| ()),
        Err(Refusal::Layout)
    );
}

#[cfg(unix)]
#[test]
fn symlinks_are_never_followed_out_of_the_local_data_folder() {
    // The workspace name is a link to a workspace elsewhere.
    let scene = Scene::new();
    let elsewhere = scene.temp.path().join("elsewhere");
    fs::rename(scene.local.join(CANONICAL), &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, scene.local.join(CANONICAL)).unwrap();
    let before = files(&elsewhere);
    assert_eq!(
        scene.replace(&Store::default(), None).map(|_| ()),
        Err(Refusal::Layout)
    );
    assert!(fs::symlink_metadata(scene.local.join(CANONICAL))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(files(&elsewhere), before);
    assert!(scene.data.join("setup-v1/model-v1").exists());
    // A record that is a link is not a workspace's record.
    let scene = Scene::new();
    let root = scene.local.join(CANONICAL);
    let record = scene.temp.path().join("record.json");
    fs::rename(root.join("record.json"), &record).unwrap();
    std::os::unix::fs::symlink(&record, root.join("record.json")).unwrap();
    let before = scene.intact();
    assert_eq!(
        scene.replace(&Store::default(), None).map(|_| ()),
        Err(Refusal::Layout)
    );
    assert_eq!(scene.intact(), before);
    // Deleting a retired workspace removes a link inside it, never what it points to.
    let scene = Scene::new();
    let outside = scene.temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("precious"), b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, scene.local.join(CANONICAL).join("link")).unwrap();
    scene.replace(&Store::default(), None).unwrap();
    assert!(scene.owner().clean().unwrap().is_empty());
    assert_eq!(fs::read(outside.join("precious")).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn a_refused_rename_withdraws_the_intent_and_keeps_the_workspace() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        return; // Directory permissions do not bind root.
    }
    let scene = Scene::new();
    // Create the store first, then make the local data folder refuse renames.
    drop(scene.owner());
    let before = scene.intact();
    fs::set_permissions(&scene.local, fs::Permissions::from_mode(0o500)).unwrap();
    let result = scene.replace(&Store::default(), None).map(|_| ());
    fs::set_permissions(&scene.local, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result, Err(Refusal::InUse));
    assert_eq!(scene.intact(), before);
    assert!(scene.intent().is_none());
    assert!(scene.discarded().is_empty());
}

#[test]
fn a_failed_deletion_leaves_reclaimable_space_without_failing_the_action() {
    let scene = Scene::new();
    let first = scene.replace(&Store::default(), None).unwrap();
    let name = discarded(first.transaction);
    let remaining = scene
        .owner()
        .clean_with(|_, _| Err(std::io::Error::other("denied")))
        .unwrap();
    assert_eq!(remaining, vec![name.clone()]);
    assert!(scene.local.join(&name).join("record.json").exists());
    // Launch reports it as reclaimable space.
    let Some(FreshStatusDto::Replaced { reclaimable, .. }) =
        recover_startup(&scene.data, &scene.local, &scene.setup, &Store::default()).unwrap()
    else {
        panic!("reclaimable space must be reported");
    };
    assert_eq!(
        reclaimable,
        vec![scene.local.join(&name).display().to_string()]
    );
    // A later fresh workspace keeps reporting it and retries its deletion.
    let second = scene.replace(&Store::default(), None).unwrap();
    assert_eq!(second.reclaimable[0], name);
    assert_eq!(scene.intent().unwrap().earlier, vec![first.transaction]);
    assert!(scene.owner().clean().unwrap().is_empty());
    assert!(scene.discarded().is_empty());
    assert!(scene.intent().unwrap().earlier.is_empty());
    assert_eq!(
        recover_startup(&scene.data, &scene.local, &scene.setup, &Store::default()).unwrap(),
        None
    );
}

#[test]
fn an_unconfirmed_credential_removal_keeps_the_record_and_still_settles() {
    let scene = Scene::new();
    let model = fs::read(scene.data.join("setup-v1/model-v1")).unwrap();
    let store = Store {
        fail: true,
        ..Store::default()
    };
    let settled = scene.replace(&store, None).unwrap();
    assert_eq!(settled.ai, AiOutcome::Kept);
    // Kept exactly, so the existing Reset retries this same slot on the new workspace.
    assert_eq!(
        fs::read(scene.data.join("setup-v1/model-v1")).unwrap(),
        model
    );
    assert!(scene.data.join("setup-v1/model-account-v1").exists());
    assert!(!scene.local.join(CANONICAL).exists());
    admit(&scene.local).unwrap();
    let fresh = Journal::create(&scene.local.join(CANONICAL), BackendKind::Apple, vec![1]).unwrap();
    let installation = fresh.record().installation_id;
    drop(fresh);
    let retry = Store::default();
    scene
        .setup
        .reset_mismatched_model(installation, &retry)
        .unwrap();
    assert_eq!(*retry.deleted.lock().unwrap(), vec![scene.slot]);
    assert!(!scene.data.join("setup-v1/model-v1").exists());
}

#[test]
fn nothing_saved_settles_as_absent() {
    let scene = Scene::new();
    fs::remove_file(scene.data.join("setup-v1/model-v1")).unwrap();
    fs::remove_file(scene.data.join("setup-v1/model-account-v1")).unwrap();
    let store = Store::default();
    assert_eq!(scene.replace(&store, None).unwrap().ai, AiOutcome::Absent);
    assert!(store.deleted.lock().unwrap().is_empty());
}

#[test]
fn the_reset_never_forgets_a_connection_while_a_workspace_is_in_place() {
    let scene = Scene::new();
    let before = scene.intact();
    assert!(scene.setup.reset_retired_model(&Store::default()).is_err());
    assert_eq!(scene.intact(), before);
}

#[test]
fn repeating_the_action_finishes_an_interrupted_one_or_starts_another() {
    let scene = Scene::new();
    let store = Store::default();
    assert!(scene.replace(&store, Some("retired-recorded")).is_err());
    let interrupted = scene.intent().unwrap().transaction;
    // Pressing it again finishes the transaction already under way; it never retires twice.
    let resumed = scene.replace(&store, None).unwrap();
    assert_eq!(resumed.transaction, interrupted);
    assert_eq!(scene.discarded(), vec![discarded(interrupted)]);
    // After it settled, pressing it again with nothing in place still settles.
    let again = scene.replace(&store, None).unwrap();
    assert_ne!(again.transaction, interrupted);
    assert_eq!(scene.intent().unwrap().directory, None);
    assert_eq!(again.reclaimable, vec![discarded(interrupted)]);
    // A concurrent owner is refused as busy.
    let held = scene.owner();
    assert!(matches!(
        FreshWorkspace::open(&scene.data, &scene.local),
        Err(Refusal::Busy)
    ));
    drop(held);
}

#[test]
fn update_records_refuse_before_anything_is_touched() {
    let scene = Scene::new();
    let before = scene.intact();
    let store_dir = scene.local.join("workspace-upgrades-v1");
    fs::create_dir(&store_dir).unwrap();
    // An update store without a transaction does not refuse.
    assert_eq!(preflight(&scene.data, &scene.local), None);
    fs::write(store_dir.join("intent.json"), b"{\"unreadable\":true}").unwrap();
    assert_eq!(
        preflight(&scene.data, &scene.local),
        Some(Refusal::UpdateHistory)
    );
    assert_eq!(
        scene.replace(&Store::default(), None).map(|_| ()),
        Err(Refusal::UpdateHistory)
    );
    assert_eq!(scene.intact(), before);
    assert!(scene.intent().is_none());
}

#[test]
fn unexpected_directories_are_a_conflict_left_exactly_as_found() {
    let scene = Scene::new();
    let store = Store::default();
    assert!(scene.replace(&store, Some("renamed")).is_err());
    // Something else created a workspace in its place before recovery.
    let other = Journal::create(&scene.local.join(CANONICAL), BackendKind::Apple, vec![1]).unwrap();
    drop(other);
    let before = scene.intact();
    let retired = scene.discarded();
    assert!(recover_startup(&scene.data, &scene.local, &scene.setup, &store).is_err());
    assert!(admit(&scene.local).is_err());
    assert_eq!(scene.intact(), before);
    assert_eq!(scene.discarded(), retired);
    assert!(store.deleted.lock().unwrap().is_empty());
}

#[test]
fn an_unreadable_intent_blocks_launch_and_is_never_guessed_past() {
    let scene = Scene::new();
    drop(scene.owner());
    fs::write(scene.local.join(STORE).join("intent.json"), b"garbage").unwrap();
    let before = scene.intact();
    assert!(admit(&scene.local).is_err());
    assert!(recover_startup(&scene.data, &scene.local, &scene.setup, &Store::default()).is_err());
    assert_eq!(
        scene.replace(&Store::default(), None).map(|_| ()),
        Err(Refusal::Layout)
    );
    assert_eq!(scene.intact(), before);
}

#[test]
fn the_reply_names_only_native_paths_and_finite_states() {
    let dto = FreshStatusDto::Replaced {
        transaction: Uuid::nil(),
        ai: AiOutcome::Kept,
        reclaimable: vec!["/native/discarded".into()],
        cleaning: true,
    };
    assert_eq!(
        serde_json::to_value(dto).unwrap(),
        serde_json::json!({"state":"replaced","transaction":Uuid::nil(),"ai":"kept","reclaimable":["/native/discarded"],"cleaning":true})
    );
    assert_eq!(
        serde_json::to_value(FreshStatusDto::Refused {
            reason: Refusal::StopUnconfirmed
        })
        .unwrap(),
        serde_json::json!({"state":"refused","reason":"stopUnconfirmed"})
    );
    assert_eq!(
        serde_json::to_value(FreshStatusDto::Replacing).unwrap(),
        serde_json::json!({"state":"replacing"})
    );
}
