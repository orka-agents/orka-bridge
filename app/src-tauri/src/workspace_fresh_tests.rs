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
    fn replace(&self, store: &Store, stop_at: Option<&str>) -> Result<Settled, Failure> {
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
fn refused(reason: Refusal) -> Result<(), Failure> {
    Err(Failure::Refused(reason))
}
fn earlier(intent: &Intent) -> Vec<Uuid> {
    intent.earlier.iter().map(|e| e.transaction).collect()
}
/// As the transaction reads it: opened relative to its parent, without following links, so
/// the tests take the real Windows path (a plain `File::open` cannot open a directory there).
fn directory_identity(path: &Path) -> FileIdentity {
    let parent =
        Dir::open_ambient_dir(path.parent().unwrap(), cap_std::ambient_authority()).unwrap();
    entry_identity(&parent, path.file_name().unwrap().to_str().unwrap())
        .unwrap()
        .unwrap()
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
        // Until it settles, no workspace may be opened or created in its place, unless the
        // intent provably moved nothing: then the old workspace may start as it was.
        let admitted = matches!(point, "intent" | "rename" | "settled");
        assert_eq!(admit(&scene.local).is_ok(), admitted, "{point}");
        assert_eq!(
            crate::workspace_upgrade::admit(&scene.data, &scene.local.join(CANONICAL)).is_ok(),
            admitted,
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
        refused(Refusal::InUse)
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
            refused(Refusal::Layout),
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
        refused(Refusal::Layout)
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
        refused(Refusal::Layout)
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
        refused(Refusal::Layout)
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
        refused(Refusal::Layout)
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
    assert_eq!(result, refused(Refusal::InUse));
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
    assert_eq!(earlier(&scene.intent().unwrap()), vec![first.transaction]);
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
    // Connecting on the new workspace names the mismatch, which is what offers Reset.
    let offered = scene.setup.reset_mismatched_model(
        installation,
        &Store {
            fail: true,
            ..Store::default()
        },
    );
    let offered = serde_json::to_value(offered.unwrap_err()).unwrap();
    assert_eq!(offered["kind"], "modelStore");
    assert!(offered["recovery"].is_object());
    let retry = Store::default();
    scene
        .setup
        .reset_mismatched_model(installation, &retry)
        .unwrap();
    assert_eq!(*retry.deleted.lock().unwrap(), vec![scene.slot]);
    assert!(!scene.data.join("setup-v1/model-v1").exists());
    // Reset forgets the login it showed for that slot, as a fresh workspace does.
    assert!(!scene.data.join("setup-v1/model-account-v1").exists());
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
        refused(Refusal::UpdateHistory)
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
    // Launch says what Bridge found, by path, and keeps the workspace stopped.
    let Some(FreshStatusDto::Conflict { found }) =
        recover_startup(&scene.data, &scene.local, &scene.setup, &store).unwrap()
    else {
        panic!("a conflict must be reported");
    };
    assert_eq!(
        found,
        vec![
            scene.local.join(CANONICAL).display().to_string(),
            scene.local.join(&retired[0]).display().to_string()
        ]
    );
    assert!(FreshStatusDto::Conflict { found }.blocks_launch());
    assert!(admit(&scene.local).is_err());
    // The action reports the same conflict, and update actions refuse it too.
    assert!(matches!(
        scene.replace(&store, None),
        Err(Failure::Conflict(_))
    ));
    assert!(crate::workspace_upgrade::validate_new_replacement(&scene.data, &scene.local).is_err());
    assert_eq!(scene.intact(), before);
    assert_eq!(scene.discarded(), retired);
    assert!(store.deleted.lock().unwrap().is_empty());
    // Moving both folders away, as the screen says, lets Bridge finish on the next launch.
    let aside = scene.temp.path().join("aside");
    fs::create_dir(&aside).unwrap();
    fs::rename(scene.local.join(CANONICAL), aside.join(CANONICAL)).unwrap();
    fs::rename(scene.local.join(&retired[0]), aside.join(&retired[0])).unwrap();
    assert!(matches!(
        recover_startup(&scene.data, &scene.local, &scene.setup, &store).unwrap(),
        Some(FreshStatusDto::Replaced { .. })
    ));
    admit(&scene.local).unwrap();
    assert_eq!(*store.deleted.lock().unwrap(), vec![scene.slot]);
}

#[test]
fn an_unreadable_intent_blocks_launch_and_is_never_guessed_past() {
    let scene = Scene::new();
    drop(scene.owner());
    fs::write(scene.local.join(STORE).join("intent.json"), b"garbage").unwrap();
    let before = scene.intact();
    assert!(admit(&scene.local).is_err());
    assert_eq!(
        recover_startup(&scene.data, &scene.local, &scene.setup, &Store::default()).unwrap(),
        Some(FreshStatusDto::Conflict {
            found: vec![scene
                .local
                .join(STORE)
                .join("intent.json")
                .display()
                .to_string()]
        })
    );
    assert_eq!(
        scene.replace(&Store::default(), None).map(|_| ()),
        Err(Failure::Conflict(vec![INTENT.into()]))
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
        serde_json::to_value(FreshStatusDto::refused(Refusal::StopUnconfirmed)).unwrap(),
        serde_json::json!({"state":"refused","reason":"stopUnconfirmed","restarted":false})
    );
    assert_eq!(
        serde_json::to_value(FreshStatusDto::Refused {
            reason: Refusal::NoSpace,
            restarted: false,
            needed_bytes: Some(7),
            reclaimable: vec!["/native/discarded".into()]
        })
        .unwrap(),
        serde_json::json!({"state":"refused","reason":"noSpace","restarted":false,"neededBytes":7,"reclaimable":["/native/discarded"]})
    );
    assert_eq!(
        serde_json::to_value(FreshStatusDto::Interrupted {
            transaction: Uuid::nil()
        })
        .unwrap(),
        serde_json::json!({"state":"interrupted","transaction":Uuid::nil()})
    );
    assert_eq!(
        serde_json::to_value(FreshStatusDto::Conflict {
            found: vec!["/native/local-vm-v1".into()]
        })
        .unwrap(),
        serde_json::json!({"state":"conflict","found":["/native/local-vm-v1"]})
    );
    assert_eq!(
        serde_json::to_value(FreshPlan {
            blocked: Some(Refusal::UpdateHistory),
            image: FreshImage::Download { bytes: Some(9) }
        })
        .unwrap(),
        serde_json::json!({"blocked":"updateHistory","image":{"kind":"download","bytes":9}})
    );
    assert_eq!(
        serde_json::to_value(FreshPlan {
            blocked: None,
            image: FreshImage::Cached
        })
        .unwrap(),
        serde_json::json!({"image":{"kind":"cached"}})
    );
    assert_eq!(
        serde_json::to_value(FreshStatusDto::Replacing).unwrap(),
        serde_json::json!({"state":"replacing"})
    );
}

#[test]
fn every_durable_step_is_flushed_in_order() {
    let scene = Scene::new();
    let mut owner = scene.owner();
    owner
        .replace_with(&scene.setup, &Store::default(), |_| Ok(()))
        .unwrap();
    // Each record is flushed before it replaces the last one, and the store after; the rename
    // is flushed before the intent says it happened; the AI is reset only once the retirement
    // is durable, and recorded as settled only after.
    assert_eq!(
        *owner.trace.lock().unwrap(),
        [
            "flush intent",
            "record intent",
            "flush store intent",
            "rename",
            "flush rename",
            "flush retired",
            "record retired",
            "flush store retired",
            "reset ai",
            "flush settled",
            "record settled",
            "flush store settled",
        ]
    );
}

#[test]
fn a_failure_after_the_rename_is_interrupted_never_unchanged() {
    for step in [
        "flush rename",
        "flush retired",
        "record retired",
        "flush store retired",
        "flush settled",
        "record settled",
        "flush store settled",
    ] {
        let scene = Scene::new();
        let before = scene.intact();
        let store = Store::default();
        fail_once(&scene.local, step);
        let result = scene.replace(&store, None);
        let Err(Failure::Interrupted(transaction)) = result else {
            panic!("{step}: {result:?}")
        };
        // The workspace really is set aside: that is what the reply must say.
        assert!(!scene.local.join(CANONICAL).exists(), "{step}");
        assert_eq!(
            directory_identity(&scene.local.join(discarded(transaction))),
            before.identity,
            "{step}"
        );
        // A settled record that reached disk admits first install; anything earlier does not.
        let settled = scene.intent().unwrap().phase == Phase::Settled;
        assert_eq!(admit(&scene.local).is_ok(), settled, "{step}");
        // Bridge finishes it on the next try, as recovery does.
        let finished = scene.owner().resume(&scene.setup, &store).unwrap();
        assert_eq!(finished.transaction, transaction, "{step}");
        assert_eq!(scene.intent().unwrap().phase, Phase::Settled, "{step}");
        admit(&scene.local).unwrap();
    }
}

#[test]
fn the_action_retries_recovery_once_before_reporting_an_interruption() {
    // One failure past the rename: the action's own retry finishes it.
    let scene = Scene::new();
    fail_once(&scene.local, "record retired");
    let settled = scene
        .owner()
        .replace(&scene.setup, &Store::default())
        .unwrap();
    assert_eq!(scene.intent().unwrap().transaction, settled.transaction);
    admit(&scene.local).unwrap();
    // The same failure twice: interrupted, and launch finishes it.
    let scene = Scene::new();
    fail_once(&scene.local, "record retired");
    fail_once(&scene.local, "record retired");
    let result = scene.owner().replace(&scene.setup, &Store::default());
    let Err(Failure::Interrupted(transaction)) = result else {
        panic!("{result:?}")
    };
    assert!(admit(&scene.local).is_err());
    let Some(FreshStatusDto::Replaced {
        transaction: finished,
        ..
    }) = recover_startup(&scene.data, &scene.local, &scene.setup, &Store::default()).unwrap()
    else {
        panic!("launch must finish it");
    };
    assert_eq!(finished, transaction);
    admit(&scene.local).unwrap();
}

#[test]
fn a_failure_before_the_rename_reports_nothing_changed_and_never_blocks_launch() {
    for steps in [
        &["flush intent"][..],
        &["record intent"],
        &["flush store intent"],
        // The withdrawal fails too: the intent stays, but it provably moved nothing.
        &["flush store intent", "withdraw"],
    ] {
        let scene = Scene::new();
        let before = scene.intact();
        for step in steps {
            fail_once(&scene.local, step);
        }
        assert_eq!(
            scene.replace(&Store::default(), None).map(|_| ()),
            refused(Refusal::Unavailable),
            "{steps:?}"
        );
        assert_eq!(scene.intact(), before, "{steps:?}");
        assert!(scene.discarded().is_empty(), "{steps:?}");
        admit(&scene.local).unwrap();
        crate::workspace_upgrade::admit(&scene.data, &scene.local.join(CANONICAL)).unwrap();
        crate::workspace_upgrade::validate_new_replacement(&scene.data, &scene.local).unwrap();
        // Launch withdraws a leftover intent and reports nothing.
        assert_eq!(
            recover_startup(&scene.data, &scene.local, &scene.setup, &Store::default()).unwrap(),
            None,
            "{steps:?}"
        );
        assert!(scene.intent().is_none(), "{steps:?}");
    }
}

#[cfg(unix)]
#[test]
fn the_workspace_lock_is_held_through_the_rename_where_renaming_allows_it() {
    let scene = Scene::new();
    let root = scene.local.join(CANONICAL);
    let held = std::cell::Cell::new(None);
    scene
        .owner()
        .replace_with(&scene.setup, &Store::default(), |point| {
            if point == "rename" {
                held.set(Some(Journal::recovery_owner(&root).is_err()));
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(held.get(), Some(true));
}

#[test]
fn deletion_leaves_anything_that_is_not_the_recorded_directory() {
    let scene = Scene::new();
    let settled = scene.replace(&Store::default(), None).unwrap();
    let name = discarded(settled.transaction);
    // Something else now has the retired name.
    let aside = scene.temp.path().join("aside");
    fs::rename(scene.local.join(&name), &aside).unwrap();
    fs::create_dir(scene.local.join(&name)).unwrap();
    fs::write(scene.local.join(&name).join("precious"), b"keep").unwrap();
    assert_eq!(scene.owner().clean().unwrap(), vec![name.clone()]);
    assert_eq!(
        fs::read(scene.local.join(&name).join("precious")).unwrap(),
        b"keep"
    );
    // The recorded directory is deleted once it is back under its name.
    fs::remove_dir_all(scene.local.join(&name)).unwrap();
    fs::rename(&aside, scene.local.join(&name)).unwrap();
    assert!(scene.owner().clean().unwrap().is_empty());
}

#[test]
fn a_settled_transaction_never_blocks_launch_whatever_it_left_behind() {
    let scene = Scene::new();
    let settled = scene.replace(&Store::default(), None).unwrap();
    let name = discarded(settled.transaction);
    // A plain file where the retired directory was.
    fs::remove_dir_all(scene.local.join(&name)).unwrap();
    fs::write(scene.local.join(&name), b"odd").unwrap();
    admit(&scene.local).unwrap();
    let Some(FreshStatusDto::Replaced {
        reclaimable,
        cleaning,
        ..
    }) = recover_startup(&scene.data, &scene.local, &scene.setup, &Store::default()).unwrap()
    else {
        panic!("leftovers are reported, not blocking");
    };
    assert_eq!(
        reclaimable,
        vec![scene.local.join(&name).display().to_string()]
    );
    assert!(cleaning);
    // Cleanup leaves what is not the recorded directory and keeps reporting it.
    assert_eq!(scene.owner().clean().unwrap(), vec![name.clone()]);
    assert_eq!(fs::read(scene.local.join(&name)).unwrap(), b"odd");
    // A later fresh workspace still starts.
    scene.replace(&Store::default(), None).unwrap();
}

#[test]
fn update_actions_refuse_while_a_fresh_workspace_is_unfinished() {
    let scene = Scene::new();
    assert!(scene.replace(&Store::default(), Some("renamed")).is_err());
    fs::create_dir(scene.local.join("workspace-upgrades-v1")).unwrap();
    assert!(crate::workspace_upgrade::validate_new_replacement(&scene.data, &scene.local).is_err());
    assert!(crate::workspace_upgrade::validate_transaction(
        &scene.data,
        &scene.local,
        Uuid::new_v4()
    )
    .is_err());
    scene.replace(&Store::default(), None).unwrap();
    crate::workspace_upgrade::validate_new_replacement(&scene.data, &scene.local).unwrap();
}

#[test]
fn a_saved_record_with_unusable_connection_fields_is_still_forgotten() {
    let scene = Scene::new();
    let installation = scene.installation;
    let invalid = serde_json::json!({"slot":scene.slot,"installation":installation,"phase":"connected","nonce":Uuid::new_v4(),
        "connection":{"id":Uuid::nil(),"generation":Uuid::nil(),"selection":{"provider":"copilot","model":"gpt-5"}}});
    fs::write(
        scene.data.join("setup-v1/model-v1"),
        serde_json::to_vec(&invalid).unwrap(),
    )
    .unwrap();
    let store = Store::default();
    assert_eq!(scene.replace(&store, None).unwrap().ai, AiOutcome::Removed);
    assert_eq!(*store.deleted.lock().unwrap(), vec![scene.slot]);
    assert!(!scene.data.join("setup-v1/model-v1").exists());
    // If even that removal is unconfirmed, the reply does not promise a Reset it can't offer.
    let scene = Scene::new();
    fs::write(
        scene.data.join("setup-v1/model-v1"),
        serde_json::to_vec(&invalid).unwrap(),
    )
    .unwrap();
    let failing = Store {
        fail: true,
        ..Store::default()
    };
    assert_eq!(
        scene.replace(&failing, None).unwrap().ai,
        AiOutcome::Unconfirmed
    );
}

#[test]
fn the_status_plan_reads_update_history_without_taking_any_lock() {
    let scene = Scene::new();
    assert_eq!(blocked(&scene.local), None);
    let store_dir = scene.local.join("workspace-upgrades-v1");
    fs::create_dir(&store_dir).unwrap();
    assert_eq!(blocked(&scene.local), None);
    fs::write(
        store_dir.join(format!("completed-{}.json", Uuid::new_v4())),
        b"{}",
    )
    .unwrap();
    assert_eq!(blocked(&scene.local), Some(Refusal::UpdateHistory));
    // While the upgrade owner is held, the plan still answers.
    let held = crate::workspace_upgrade::WorkspaceUpgrade::open(&scene.data, &scene.local);
    assert_eq!(blocked(&scene.local), Some(Refusal::UpdateHistory));
    drop(held);
    // Finishing an interrupted fresh workspace is never blocked.
    let scene = Scene::new();
    assert!(scene.replace(&Store::default(), Some("renamed")).is_err());
    fs::create_dir(scene.local.join("workspace-upgrades-v1")).unwrap();
    fs::write(scene.local.join("workspace-upgrades-v1/intent.json"), b"{}").unwrap();
    assert_eq!(blocked(&scene.local), None);
    assert_eq!(preflight(&scene.data, &scene.local), None);
}

#[test]
fn the_cached_image_and_download_sizes_come_from_the_pinned_release() {
    use crate::runtime::image_types::ImageTarget;
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("image-cache-v1");
    let sha = "a".repeat(64);
    assert!(!image_cached(&cache, &sha));
    fs::create_dir_all(cache.join("images/v2").join(&sha)).unwrap();
    assert!(!image_cached(&cache, &sha));
    fs::write(cache.join("images/v2").join(&sha).join("ready.json"), b"{}").unwrap();
    assert!(image_cached(&cache, &sha));
    assert!(!image_cached(&cache, "../a"));
    for (target, manifest, disk) in [
        (
            ImageTarget::MacosArm64,
            include_bytes!("../../../images/release/macos-arm64.json").as_slice(),
            17_179_869_184,
        ),
        (
            ImageTarget::WindowsAmd64,
            include_bytes!("../../../images/release/windows-amd64.json").as_slice(),
            3_765_960_704,
        ),
    ] {
        let (download, needed) = download_sizes(manifest).unwrap();
        assert_eq!(Some(download), crate::catalog::image_download_bytes(target));
        // The cache's own check: the download plus the whole decoded disk.
        assert_eq!(needed, download + disk);
    }
    assert_eq!(space_check(Some(10), 11), Err(Refusal::NoSpace));
    assert_eq!(space_check(Some(11), 11), Ok(()));
    assert_eq!(space_check(None, 11), Ok(()));
}

#[tokio::test]
async fn the_download_check_refuses_before_anything_changes() {
    use crate::runtime::image_types::{ImagePin, ImageTarget, TransferLimits};
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("image-cache-v1");
    // The cache folder need not exist yet: its parent's free space counts.
    assert!(available_space(&cache).is_some());
    let pin = ImagePin {
        release_id: "fixture".into(),
        manifest_url: "https://fixture.invalid/manifest.json".into(),
        manifest_sha256: "a".repeat(64),
        accepted_schema_sha256: "a".repeat(64),
        target: ImageTarget::WindowsAmd64,
        asset_base: "https://fixture.invalid/".into(),
        redirect_origins: vec![],
        limits: TransferLimits {
            compressed_bytes: 1,
            disk_bytes: 1,
            kernel_bytes: 1,
            initrd_bytes: 1,
        },
    };
    assert_eq!(
        download_check(&cache, &pin, u64::MAX).await,
        Err(Refusal::NoSpace)
    );
    // A download that cannot be reached.
    assert_eq!(download_check(&cache, &pin, 0).await, Err(Refusal::Offline));
}

/// A junction where the workspace should be: Windows' unprivileged directory link.
#[cfg(windows)]
#[test]
fn a_junction_is_never_followed_out_of_the_local_data_folder() {
    let scene = Scene::new();
    let elsewhere = scene.temp.path().join("elsewhere");
    fs::rename(scene.local.join(CANONICAL), &elsewhere).unwrap();
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(scene.local.join(CANONICAL))
        .arg(&elsewhere)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let before = files(&elsewhere);
    assert_eq!(
        scene.replace(&Store::default(), None).map(|_| ()),
        refused(Refusal::Layout)
    );
    assert!(scene.local.join(CANONICAL).exists());
    assert_eq!(files(&elsewhere), before);
    assert!(scene.data.join("setup-v1/model-v1").exists());
    assert!(scene.discarded().is_empty());
}

/// Windows' way to refuse the rename: another handle on the workspace that does not share
/// deletion, as a process still holding it would.
#[cfg(windows)]
#[test]
fn a_held_directory_handle_refuses_the_rename_and_keeps_the_workspace() {
    use std::os::windows::fs::OpenOptionsExt;
    let scene = Scene::new();
    let before = scene.intact();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1 | 2) // FILE_SHARE_READ | FILE_SHARE_WRITE, not FILE_SHARE_DELETE
        .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS: open the directory itself
        .open(scene.local.join(CANONICAL))
        .unwrap();
    assert_eq!(
        scene.replace(&Store::default(), None).map(|_| ()),
        refused(Refusal::InUse)
    );
    drop(held);
    assert_eq!(scene.intact(), before);
    assert!(scene.intent().is_none());
    assert!(scene.discarded().is_empty());
    admit(&scene.local).unwrap();
}

/// Rewrites the scene's committed record to the phase an install reached before it stopped.
/// `consumed` also records the boot that consumed it, as a real first boot does.
fn record_phase(scene: &Scene, phase: &str) {
    let path = scene.local.join(CANONICAL).join("record.json");
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["phase"] = phase.into();
    if phase == "consumed" {
        record["bootNonce"] = Uuid::new_v4().to_string().into();
    }
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
}
/// A first install interrupted while it copied the image: the staged generation, no binding.
fn interrupted_copy(scene: &Scene) {
    let root = scene.local.join(CANONICAL);
    fs::remove_dir_all(root.join("image")).unwrap();
    fs::create_dir_all(root.join("image/.pending-1")).unwrap();
    fs::write(root.join("image/.pending-1/os.qcow2"), b"partial").unwrap();
    fs::write(root.join("image-owner.lock"), b"").unwrap();
}
fn discard(scene: &Scene, store: &Store) -> Result<bool, FreshStatusDto> {
    discard_unfinished_install(&scene.data, &scene.local, &scene.setup, store)
}

#[test]
fn an_install_that_never_booted_is_started_again_from_scratch_at_launch() {
    // Every point a first install can stop at once `local-vm-v1` exists: before the first
    // record, while copying the image, while preparing the state disk, prepared but before the
    // guest was handed it.
    for phase in [None, Some("empty"), Some("preparing"), Some("prepared")] {
        let scene = Scene::new();
        interrupted_copy(&scene);
        match phase {
            Some(phase) => record_phase(&scene, phase),
            None => {
                let root = scene.local.join(CANONICAL);
                fs::remove_dir_all(&root).unwrap();
                fs::create_dir(&root).unwrap();
                fs::write(root.join("owner.lock"), b"").unwrap();
                fs::write(root.join("state.disk"), b"").unwrap();
            }
        }
        let before = scene.intact();
        let store = Store::default();
        assert_eq!(discard(&scene, &store), Ok(true), "{phase:?}");
        // Retired the way a fresh workspace retires one: moved whole, never copied.
        assert!(!scene.local.join(CANONICAL).exists());
        let [name] = scene.discarded().try_into().unwrap();
        assert_eq!(
            directory_identity(&scene.local.join(&name)),
            before.identity
        );
        assert_eq!(files(&scene.local.join(&name)), before.files);
        assert_eq!(scene.intent().unwrap().phase, Phase::Settled);
        scene.assert_kept_outside_workspace();
        // First install may create the new installation; cleanup deletes only the retired one.
        admit(&scene.local).unwrap();
        crate::workspace_upgrade::admit(&scene.data, &scene.local.join(CANONICAL)).unwrap();
        assert!(scene.owner().clean().unwrap().is_empty());
        assert!(scene.discarded().is_empty());
        // Nothing else is retired at the next launch, and nothing is reported then.
        assert_eq!(discard(&scene, &store), Ok(false));
        assert_eq!(
            recover_startup(&scene.data, &scene.local, &scene.setup, &store).unwrap(),
            None
        );
    }
}

#[test]
fn a_workspace_a_guest_booted_is_never_discarded_at_launch() {
    let scene = Scene::new();
    record_phase(&scene, "consumed");
    let before = scene.intact();
    assert_eq!(discard(&scene, &Store::default()), Ok(false));
    assert_eq!(scene.intact(), before);
    assert!(scene.intent().is_none());
    assert!(scene.discarded().is_empty());
}

#[test]
fn update_records_or_an_unreadable_update_store_keep_an_unfinished_install_in_place() {
    for record in [
        "intent.json",
        "completed-00000000-0000-4000-8000-000000000001.json",
    ] {
        let scene = Scene::new();
        interrupted_copy(&scene);
        let before = scene.intact();
        let store_dir = scene.local.join("workspace-upgrades-v1");
        fs::create_dir(&store_dir).unwrap();
        fs::write(store_dir.join(record), b"{\"unreadable\":true}").unwrap();
        assert_eq!(discard(&scene, &Store::default()), Ok(false), "{record}");
        assert_eq!(scene.intact(), before);
        assert!(scene.discarded().is_empty());
    }
    // An update store holding only its lock has no records: the install starts again.
    let scene = Scene::new();
    interrupted_copy(&scene);
    let store_dir = scene.local.join("workspace-upgrades-v1");
    fs::create_dir(&store_dir).unwrap();
    fs::write(store_dir.join("owner.lock"), b"").unwrap();
    assert_eq!(discard(&scene, &Store::default()), Ok(true));
}

#[test]
fn a_live_worker_or_an_unfinished_fresh_workspace_keeps_the_install_in_place() {
    let scene = Scene::new();
    interrupted_copy(&scene);
    let before = scene.intact();
    let live = Journal::open(&scene.local.join(CANONICAL)).unwrap();
    assert_eq!(discard(&scene, &Store::default()), Ok(false));
    drop(live);
    assert_eq!(scene.intact(), before);
    assert!(scene.intent().is_none());
    // A fresh workspace that stopped after its intent is recovery's to finish, never this.
    assert!(scene.replace(&Store::default(), Some("intent")).is_err());
    let intent = scene.intent().unwrap();
    assert_eq!(intent.phase, Phase::Intent);
    assert_eq!(discard(&scene, &Store::default()), Ok(false));
    assert_eq!(scene.intent(), Some(intent));
    assert_eq!(scene.intact(), before);
}

#[test]
fn a_crash_while_discarding_is_finished_like_any_fresh_workspace() {
    for point in [
        "intent",
        "rename",
        "renamed",
        "retired-recorded",
        "ai",
        "settled",
    ] {
        let scene = Scene::new();
        interrupted_copy(&scene);
        let before = scene.intact();
        let stopped =
            scene
                .owner()
                .discard_unfinished_with(&scene.setup, &Store::default(), |at| {
                    if at == point {
                        Err(RuntimeError::Closed)
                    } else {
                        Ok(())
                    }
                });
        assert!(stopped.is_err(), "{point}");
        // The next launch finishes or undoes it, and then the install starts from scratch.
        let store = Store::default();
        let status = recover_startup(&scene.data, &scene.local, &scene.setup, &store).unwrap();
        assert!(
            status.as_ref().is_none_or(|status| !status.blocks_launch()),
            "{point}: {status:?}"
        );
        assert!(discard(&scene, &store).is_ok(), "{point}");
        assert!(!scene.local.join(CANONICAL).exists(), "{point}");
        admit(&scene.local).unwrap();
        // Whatever moved is the unfinished install, whole.
        let [name] = scene.discarded().try_into().unwrap();
        assert_eq!(files(&scene.local.join(&name)), before.files, "{point}");
    }
}
