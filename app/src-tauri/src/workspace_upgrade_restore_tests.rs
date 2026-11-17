use super::*;

fn transaction(s: &Scene) -> Uuid {
    WorkspaceUpgrade::open(&s.data, &s.local)
        .unwrap()
        .read_intent()
        .unwrap()
        .unwrap()
        .transaction
}
fn restore(s: &Scene, id: Uuid, stop: Option<&str>) -> Result<UpgradeStatus, RuntimeError> {
    WorkspaceUpgrade::open(&s.data, &s.local)?.restore_with(id, |point| {
        if Some(point) == stop {
            Err(RuntimeError::Closed)
        } else {
            Ok(())
        }
    })
}
fn inventory(root: &Path) -> Vec<(PathBuf, FileIdentity, Vec<u8>)> {
    fn walk(root: &Path, path: &Path, out: &mut Vec<(PathBuf, FileIdentity, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().into(),
                    file_identity(&File::open(&path).unwrap()).unwrap(),
                    fs::read(&path).unwrap(),
                ));
            }
        }
    }
    let mut out = vec![];
    walk(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
// This models a boot-consumed journal, not a live guest boot qualification.
pub(super) fn consumed_destination(s: &Scene) -> Installation {
    let root = s.local.join("local-vm-v1");
    let mut record = read_record(&root).unwrap();
    record.phase = StatePhase::Consumed;
    record.boot_nonce = Some(Uuid::new_v4());
    fs::write(
        root.join("record.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    installation(&root).unwrap().unwrap()
}
pub(super) fn destination_metadata(s: &Scene, id: Uuid) -> (Vec<u8>, Vec<u8>) {
    let slot = Uuid::new_v4();
    let model = serde_json::to_vec(&serde_json::json!({"slot":slot,"installation":id,"phase":"stored","nonce":null,"connection":null})).unwrap();
    let account =
        serde_json::to_vec(&serde_json::json!({"slot":slot,"login":"new-account"})).unwrap();
    fs::write(s.data.join("setup-v1/model-v1"), &model).unwrap();
    fs::write(s.data.join("setup-v1/model-account-v1"), &account).unwrap();
    (model, account)
}
#[test]
fn restore_preserves_complete_a_and_b_and_current_b_metadata_without_booting_either() {
    for target in [ImageTarget::MacosArm64, ImageTarget::WindowsAmd64] {
        let mut s = Scene::new(target);
        let root = s.local.join("local-vm-v1");
        let a = inventory(&root);
        let model_a = fs::read(s.data.join("setup-v1/model-v1")).unwrap();
        let account_a = fs::read(s.data.join("setup-v1/model-account-v1")).unwrap();
        s.replace(None).unwrap();
        let b_id = consumed_destination(&s);
        fs::write(root.join("efi-stand-in"), b"new-firmware-state").unwrap();
        let b = inventory(&root);
        let (model_b, account_b) = destination_metadata(&s, b_id.installation);
        let id = transaction(&s);
        assert_eq!(
            WorkspaceUpgrade::open(&s.data, &s.local)
                .unwrap()
                .recover()
                .unwrap(),
            UpgradeStatus::Consumed
        );
        assert_eq!(
            WorkspaceUpgrade::open(&s.data, &s.local)
                .unwrap()
                .restore(id)
                .unwrap(),
            UpgradeStatus::Restored
        );
        assert_eq!(inventory(&root), a);
        assert_eq!(fs::read(s.data.join("setup-v1/model-v1")).unwrap(), model_a);
        assert_eq!(
            fs::read(s.data.join("setup-v1/model-account-v1")).unwrap(),
            account_a
        );
        let mut owner = WorkspaceUpgrade::open(&s.data, &s.local).unwrap();
        let intent = owner.read_intent().unwrap().unwrap();
        let retained_b = s.local.join(format!("retained-new-{id}"));
        assert_eq!(installation(&retained_b).unwrap(), Some(b_id));
        assert_eq!(inventory(&retained_b), b);
        let archived = intent.restore.unwrap();
        assert_eq!(archived.model, Some(model_b));
        assert_eq!(archived.account, Some(account_b));
        assert_eq!(owner.recover().unwrap(), UpgradeStatus::Restored);
        assert!(owner.restore(id).is_err());
        assert!(admit(&root).is_err()); // Still unexposed pending credential guards.
    }
}
#[test]
fn restore_refuses_foreign_transaction_live_journal_and_changed_identities_without_mutation() {
    for scenario in [
        "transaction",
        "live",
        "source-uuid",
        "source-directory",
        "destination-directory",
        "unknown-canonical",
        "retained-collision",
        "metadata",
    ] {
        let mut s = Scene::new(ImageTarget::WindowsAmd64);
        s.replace(None).unwrap();
        let id = transaction(&s);
        let root = s.local.join("local-vm-v1");
        let owner = WorkspaceUpgrade::open(&s.data, &s.local).unwrap();
        let intent = owner.read_intent().unwrap().unwrap();
        let source = s.local.join(&intent.retained);
        drop(owner);
        let mut live = None;
        match scenario {
            "live" => live = Some(Journal::open(&root).unwrap()),
            "source-uuid" => {
                let mut r = read_record(&source).unwrap();
                r.installation_id = Uuid::new_v4();
                fs::write(source.join("record.json"), serde_json::to_vec(&r).unwrap()).unwrap();
            }
            "source-directory" | "destination-directory" => {
                let path = if scenario == "source-directory" {
                    &source
                } else {
                    &root
                };
                let held = s.local.join("held-directory");
                fs::rename(path, &held).unwrap();
                fs::create_dir(path).unwrap();
                for e in fs::read_dir(held).unwrap() {
                    let e = e.unwrap();
                    fs::rename(e.path(), path.join(e.file_name())).unwrap();
                }
            }
            "unknown-canonical" => {
                fs::rename(&root, s.local.join("unknown-kept")).unwrap();
                drop(Journal::create(&root, BackendKind::Qemu, vec![8]).unwrap());
            }
            "retained-collision" => {
                fs::create_dir(s.local.join(format!("retained-new-{id}"))).unwrap()
            }
            "metadata" => {
                fs::write(s.data.join("setup-v1/model-v1"), b"unknown-schema").unwrap();
            }
            _ => {}
        }
        let before = fs::read(s.local.join(STORE).join("intent.json")).unwrap();
        let root_before = installation(&root).unwrap();
        assert!(
            restore(
                &s,
                if scenario == "transaction" {
                    Uuid::new_v4()
                } else {
                    id
                },
                None
            )
            .is_err(),
            "{scenario}"
        );
        assert_eq!(installation(&root).unwrap(), root_before, "{scenario}");
        assert_eq!(
            fs::read(s.local.join(STORE).join("intent.json")).unwrap(),
            before,
            "{scenario}"
        );
        drop(live);
    }
}
#[test]
fn restore_resumes_each_directory_and_cross_root_metadata_boundary() {
    for point in [
        "restore-intent",
        "restore-model-archived",
        "restore-account-archived",
        "destination-retained",
        "source-restored",
        "restore-model-published",
        "restore-account-published",
        "restored",
    ] {
        let mut s = Scene::new(ImageTarget::WindowsAmd64);
        let a = installation(&s.local.join("local-vm-v1")).unwrap();
        let a_files = inventory(&s.local.join("local-vm-v1"));
        s.replace(None).unwrap();
        let b = consumed_destination(&s);
        destination_metadata(&s, b.installation);
        let id = transaction(&s);
        assert!(restore(&s, id, Some(point)).is_err(), "{point}");
        let mut owner = WorkspaceUpgrade::open(&s.data, &s.local).unwrap();
        assert!(owner.recover().is_ok(), "{point}");
        if point == "restored" {
            assert!(owner.restore(id).is_err());
        } else {
            assert_eq!(
                owner.restore(id).unwrap(),
                UpgradeStatus::Restored,
                "{point}"
            );
        }
        assert_eq!(
            installation(&s.local.join("local-vm-v1")).unwrap(),
            a,
            "{point}"
        );
        assert_eq!(inventory(&s.local.join("local-vm-v1")), a_files, "{point}");
        assert_eq!(
            installation(&s.local.join(format!("retained-new-{id}"))).unwrap(),
            Some(b)
        );
        assert!(owner.take_destination().is_none());
    }
}
#[test]
fn restored_transaction_never_reopens_b_and_fresh_c_keeps_b_and_its_archive() {
    let mut s = Scene::new(ImageTarget::WindowsAmd64);
    s.replace(None).unwrap();
    let b = consumed_destination(&s);
    let id = transaction(&s);
    restore(&s, id, None).unwrap();
    let retained = s.local.join(format!("retained-new-{id}"));
    let b_files = inventory(&retained);
    // The matching older app can legitimately change A's current association.
    let (current_model, _) = destination_metadata(&s, s.source_id);
    assert_eq!(
        WorkspaceUpgrade::open(&s.data, &s.local)
            .unwrap()
            .recover()
            .unwrap(),
        UpgradeStatus::Restored
    );
    assert_eq!(
        fs::read(s.data.join("setup-v1/model-v1")).unwrap(),
        current_model
    );
    // Reacquire the exact already-verified target; never reuse retained B.
    let new_dir = s._temp.path().join("third-cache");
    fs::create_dir(&new_dir).unwrap();
    s.target = Some(
        PreparedImage::from_verified(fixture::verified_with_identity(
            &new_dir,
            ImageTarget::WindowsAmd64,
            Some("new-state"),
            Some("new-image"),
        ))
        .unwrap(),
    );
    assert_eq!(s.replace(None).unwrap(), UpgradeStatus::Fresh);
    let c = installation(&s.local.join("local-vm-v1")).unwrap().unwrap();
    assert_ne!(c.installation, b.installation);
    assert_ne!(c.installation, s.source_id);
    assert_eq!(inventory(&retained), b_files);
    assert!(s
        .local
        .join(STORE)
        .join(format!("completed-{id}.json"))
        .exists());
    assert!(restore(&s, id, None).is_err());
}
#[test]
fn restore_retains_incomplete_b_without_reopening_it_or_recovering_creation_authority() {
    for phase in [StatePhase::Empty, StatePhase::Preparing] {
        let mut s = Scene::new(ImageTarget::WindowsAmd64);
        assert!(s.replace(Some("destination-recorded")).is_err());
        let root = s.local.join("local-vm-v1");
        let mut record = read_record(&root).unwrap();
        record.phase = phase;
        fs::write(
            root.join("record.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let id = transaction(&s);
        let before = inventory(&root);
        assert_eq!(restore(&s, id, None).unwrap(), UpgradeStatus::Restored);
        assert_eq!(
            inventory(&s.local.join(format!("retained-new-{id}"))),
            before
        );
        assert_eq!(read_record(&root).unwrap().installation_id, s.source_id);
    }
}
#[test]
fn restore_releases_its_unhanded_destination_and_never_returns_that_creation_authority() {
    let mut s = Scene::new(ImageTarget::WindowsAmd64);
    let mut owner = WorkspaceUpgrade::open(&s.data, &s.local).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (_sender, closed) = watch::channel(false);
    owner
        .replace_with(
            &s.source,
            vec![2],
            &closed,
            runtime.handle(),
            || Ok(s.target.take().unwrap()),
            |_| Ok(()),
            available_space,
        )
        .unwrap();
    let id = owner.read_intent().unwrap().unwrap().transaction;
    assert_eq!(owner.restore(id).unwrap(), UpgradeStatus::Restored);
    assert!(owner.take_destination().is_none());
}
#[test]
fn restore_can_cancel_an_exact_forward_switch_before_b_exists() {
    for point in [
        "intent",
        "model-archived",
        "account-archived",
        "source-retained",
    ] {
        let mut s = Scene::new(ImageTarget::WindowsAmd64);
        let a = inventory(&s.local.join("local-vm-v1"));
        let model = fs::read(s.data.join("setup-v1/model-v1")).unwrap();
        let account = fs::read(s.data.join("setup-v1/model-account-v1")).unwrap();
        assert!(s.replace(Some(point)).is_err());
        let id = transaction(&s);
        assert_eq!(
            restore(&s, id, None).unwrap(),
            UpgradeStatus::Restored,
            "{point}"
        );
        assert_eq!(inventory(&s.local.join("local-vm-v1")), a);
        assert_eq!(fs::read(s.data.join("setup-v1/model-v1")).unwrap(), model);
        assert_eq!(
            fs::read(s.data.join("setup-v1/model-account-v1")).unwrap(),
            account
        );
        assert!(restore(&s, id, None).is_err());
    }
}
#[test]
fn restore_does_not_resurrect_metadata_after_acknowledged_slot_removal() {
    let mut s = Scene::new(ImageTarget::WindowsAmd64);
    s.replace(None).unwrap();
    let mut owner = WorkspaceUpgrade::open(&s.data, &s.local).unwrap();
    let mut intent = owner.read_intent().unwrap().unwrap();
    intent.credential = CredentialDisposition::Removed;
    owner.persist(&intent).unwrap();
    assert_eq!(
        owner.restore(intent.transaction).unwrap(),
        UpgradeStatus::Restored
    );
    assert!(!s.data.join("setup-v1/model-v1").exists());
    assert!(!s.data.join("setup-v1/model-account-v1").exists());
}
