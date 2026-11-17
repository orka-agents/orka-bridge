use super::*;
use crate::runtime::credential_store::{CredentialStore, StoreError};
use bridge_runtime_protocol::CredentialPayload;
struct Vault(std::sync::Mutex<Vec<Uuid>>);
impl CredentialStore for Vault {
    fn save(&self, _: Uuid, _: &[u8]) -> Result<(), StoreError> {
        panic!("retry saved a secret")
    }
    fn load(&self, _: Uuid) -> Result<CredentialPayload, StoreError> {
        panic!("retry loaded a secret")
    }
    fn delete(&self, slot: Uuid) -> Result<(), StoreError> {
        self.0.lock().unwrap().push(slot);
        Ok(())
    }
}
#[test]
fn retry_archived_metadata_guards_every_slot_and_removal_stays_removed_through_restore() {
    let mut scene = Scene::new(ImageTarget::WindowsAmd64);
    scene.replace(None).unwrap();
    let transaction = id(&scene);
    let root = scene.local.join("local-vm-v1");
    let b = installation(&root).unwrap().unwrap();
    let (model, account) = restore::destination_metadata(&scene, b.installation);
    let archived_slot = SetupStore::upgrade_slot(Some(&model)).unwrap().unwrap();
    retry(&scene, transaction, None).unwrap();
    let mut owner = WorkspaceUpgrade::open(&scene.data, &scene.local).unwrap();
    let intent = owner.read_intent().unwrap().unwrap();
    assert_eq!(intent.incomplete[0].model, Some(model));
    assert_eq!(intent.incomplete[0].account, Some(account));
    assert!(owner.guard_model(Some(archived_slot), true).is_err());
    let vault = Vault(std::sync::Mutex::new(vec![]));
    owner
        .remove_credential(transaction, archived_slot, &vault)
        .unwrap();
    assert_eq!(*vault.0.lock().unwrap(), vec![archived_slot]);
    owner.restore(transaction).unwrap();
    assert!(owner.slot_removed(archived_slot).unwrap());
    assert!(owner.guard_model(Some(archived_slot), false).is_err());
    assert!(owner.guard_model(Some(archived_slot), true).is_err());
    assert_ne!(
        SetupStore::upgrade_slot(Some(
            &fs::read(scene.data.join("setup-v1/model-v1")).unwrap()
        ))
        .unwrap(),
        Some(archived_slot)
    );
}
#[test]
fn retry_revalidates_all_retained_incomplete_identities_before_further_mutation() {
    let mut scene = Scene::new(ImageTarget::WindowsAmd64);
    scene.replace(None).unwrap();
    let transaction = id(&scene);
    let root = scene.local.join("local-vm-v1");
    let b = installation(&root).unwrap().unwrap();
    retry(&scene, transaction, None).unwrap();
    let retained = scene
        .local
        .join(format!("retained-incomplete-{}", b.installation));
    fs::rename(&retained, scene.local.join("kept-original")).unwrap();
    fs::create_dir(&retained).unwrap();
    let before = fs::read(scene.local.join(STORE).join("intent.json")).unwrap();
    assert!(retry(&scene, transaction, None).is_err());
    assert_eq!(
        fs::read(scene.local.join(STORE).join("intent.json")).unwrap(),
        before
    );
}
#[test]
fn retry_never_adopts_an_unrecorded_destination_after_creation_gap() {
    let mut scene = Scene::new(ImageTarget::WindowsAmd64);
    assert!(scene.replace(Some("intent")).is_err());
    let transaction = id(&scene);
    assert!(retry(&scene, transaction, Some("destination-created")).is_err());
    let root = scene.local.join("local-vm-v1");
    let unknown = installation(&root).unwrap();
    assert!(retry(&scene, transaction, None).is_err());
    assert!(WorkspaceUpgrade::open(&scene.data, &scene.local)
        .unwrap()
        .restore(transaction)
        .is_err());
    assert_eq!(installation(&root).unwrap(), unknown);
}
#[test]
fn oversized_retention_record_cannot_replace_the_last_readable_intent() {
    let mut scene = Scene::new(ImageTarget::WindowsAmd64);
    scene.replace(None).unwrap();
    let owner = WorkspaceUpgrade::open(&scene.data, &scene.local).unwrap();
    let mut intent = owner.read_intent().unwrap().unwrap();
    let before = fs::read(scene.local.join(STORE).join("intent.json")).unwrap();
    // The existing reader has a 128 KiB boundary. A growing retention ledger
    // must fail publication before replacing the last recoverable intent.
    intent.account = Some(vec![1; 128 * 1024]);
    assert!(owner.persist(&intent).is_err());
    assert_eq!(
        fs::read(scene.local.join(STORE).join("intent.json")).unwrap(),
        before
    );
}
