use super::*;
use crate::runtime as fixture_runtime;
use crate::runtime::{image_install::*, image_types::*, RuntimeWorker};
// The portable harness also loads this fixture for its integration tests.
mod fixture {
    include!("../../tests/support/runtime_image_fixture.rs");
}
use std::sync::Mutex;
#[derive(Default)]
struct Backend {
    events: Arc<Mutex<Vec<&'static str>>>,
    images: Arc<Mutex<Vec<Arc<InstalledImageSet>>>>,
}
impl VmBackend for Backend {
    fn create_state(&mut self, spec: &LaunchSpec) -> Result<(), RuntimeError> {
        self.events.lock().unwrap().push("create");
        self.images.lock().unwrap().push(spec.image.clone());
        File::options()
            .write(true)
            .open(&spec.state_path)?
            .set_len(64)?;
        Ok(())
    }
    fn start(&mut self, spec: &LaunchSpec) -> Result<BackendHandle, RuntimeError> {
        self.events.lock().unwrap().push("start");
        self.images.lock().unwrap().push(spec.image.clone());
        Ok(BackendHandle::new(()))
    }
    fn stop(&mut self, _: &BackendHandle) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn pause(&mut self, _: &BackendHandle) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn resume(&mut self, _: &BackendHandle) -> Result<(), RuntimeError> {
        Ok(())
    }
}
fn installation(kind: BackendKind) -> (tempfile::TempDir, Journal, Arc<InstalledImageSet>) {
    let temp = tempfile::tempdir().unwrap();
    let target = match kind {
        BackendKind::Apple => ImageTarget::MacosArm64,
        BackendKind::Qemu => ImageTarget::WindowsAmd64,
    };
    // Acquisition/cache ownership finishes before installation ownership begins.
    let verified = fixture::verified(temp.path(), target);
    let root = temp.path().join("install");
    let journal = Journal::create(&root, kind, vec![1]).unwrap();
    let installed = {
        let target = journal
            .image_target(&verified.image().identity().state_compatibility_id)
            .unwrap();
        let (_sender, closed) = tokio::sync::watch::channel(false);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(materialize_image(&verified, &target, closed, |_| {}))
            .unwrap()
    };
    (temp, journal, Arc::new(installed))
}
fn snapshot(root: &Path) -> (Vec<u8>, Vec<u8>) {
    (
        fs::read(root.join("record.json")).unwrap(),
        fs::read(root.join("state.disk")).unwrap(),
    )
}
#[test]
fn changed_boot_after_construction_cannot_mutate_state_record_or_call_backend() {
    for change in ["corrupt-kernel", "replace-initrd", "missing-binding"] {
        let (temp, journal, image) = installation(BackendKind::Apple);
        let root = temp.path().join("install");
        let backend = Backend::default();
        let events = backend.events.clone();
        let mut worker = RuntimeWorker::new(Box::new(backend), journal, image.clone()).unwrap();
        match change {
            "corrupt-kernel" => {
                fs::write(root.join(image.kernel_path()), b"corrupt immutable boot").unwrap()
            }
            "replace-initrd" => {
                let file = root.join(image.initrd_path());
                let retained = root.join("retained.initrd");
                fs::rename(&file, &retained).unwrap();
                fs::copy(&retained, &file).unwrap();
            }
            _ => fs::rename(
                root.join("image-binding.json"),
                root.join("retained-binding.json"),
            )
            .unwrap(),
        }
        let before = snapshot(&root);
        for _ in 0..2 {
            assert!(
                matches!(worker.start(), Err(RuntimeError::Image(_))),
                "{change} must refuse before starting"
            );
            assert!(
                events.lock().unwrap().is_empty(),
                "rejection must have no backend effects"
            );
            assert_eq!(
                snapshot(&root),
                before,
                "record revision/phase/nonce and state bytes must not change"
            );
        }
    }
}
#[test]
fn constructor_rejects_foreign_installation_wrong_platform_and_missing_binding_without_effects() {
    for change in ["foreign", "platform", "missing-binding"] {
        let (temp, mut journal, mut image) = installation(BackendKind::Apple);
        let root = temp.path().join("install");
        match change {
            "foreign" => {
                let (_other, _owner, foreign) = installation(BackendKind::Apple);
                image = foreign;
            }
            "platform" => {
                drop(journal);
                let path = root.join("record.json");
                let mut record: InstallationRecord =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                record.backend = BackendKind::Qemu;
                fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
                journal = Journal::open(&root).unwrap();
            }
            _ => fs::rename(
                root.join("image-binding.json"),
                root.join("retained-binding.json"),
            )
            .unwrap(),
        }
        let before = snapshot(&root);
        let backend = Backend::default();
        let events = backend.events.clone();
        assert!(
            matches!(
                RuntimeWorker::new(Box::new(backend), journal, image),
                Err(RuntimeError::Image(_))
            ),
            "{change}"
        );
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(snapshot(&root), before);
    }
}
#[cfg(unix)]
#[test]
fn replaced_root_with_the_same_state_inode_is_rejected_before_effects() {
    for after_construction in [false, true] {
        let (temp, mut journal, image) = installation(BackendKind::Apple);
        let root = temp.path().join("install");
        let retained = temp.path().join("retained");
        let before = snapshot(&root);
        let backend = Backend::default();
        let events = backend.events.clone();
        let replace_root = || {
            fs::rename(&root, &retained).unwrap();
            fs::create_dir(&root).unwrap();
            fs::hard_link(retained.join("state.disk"), root.join("state.disk")).unwrap();
        };
        if after_construction {
            let mut worker = RuntimeWorker::new(Box::new(backend), journal, image).unwrap();
            replace_root();
            for _ in 0..2 {
                assert!(matches!(worker.start(), Err(RuntimeError::RetainedState)));
            }
        } else {
            replace_root();
            assert!(matches!(
                journal.set_phase(StatePhase::Preparing),
                Err(RuntimeError::RetainedState)
            ));
            assert!(matches!(
                RuntimeWorker::new(Box::new(backend), journal, image),
                Err(RuntimeError::RetainedState)
            ));
        }
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(snapshot(&retained), before);
        assert_eq!(fs::read(root.join("state.disk")).unwrap(), before.1);
        assert!(!root.join("record.json").exists());
        assert!(!root.join("record.next").exists());
    }
}
#[test]
fn apple_and_qemu_forward_the_same_installed_arc_to_both_specs() {
    for kind in [BackendKind::Qemu, BackendKind::Apple] {
        let (_temp, journal, image) = installation(kind);
        let backend = Backend::default();
        let images = backend.images.clone();
        let mut worker = RuntimeWorker::new(Box::new(backend), journal, image.clone()).unwrap();
        worker.start().unwrap();
        let observed = images.lock().unwrap();
        assert_eq!(observed.len(), 2);
        assert!(observed.iter().all(|value| Arc::ptr_eq(value, &image)));
        assert!(Arc::ptr_eq(&worker.lease().unwrap().spec.image, &image));
        assert_eq!(worker.lease().unwrap().spec.backend, kind);
    }
}
#[test]
fn mutable_private_os_is_provenance_and_image_clones_do_not_own_the_journal_lock() {
    let (temp, journal, image) = installation(BackendKind::Qemu);
    let root = temp.path().join("install");
    fs::write(
        root.join(image.disk_path()),
        b"guest-mutated private disk including growth",
    )
    .unwrap();
    let clone = image.clone();
    let mut worker = RuntimeWorker::new(Box::new(Backend::default()), journal, image).unwrap();
    worker.start().unwrap();
    drop(worker);
    let reopened = Journal::open(&root).unwrap();
    reopened.validate_image(&clone).unwrap();
}
#[test]
fn reopening_an_unprepared_installation_does_not_restore_image_creation_authority() {
    let (temp, journal, image) = installation(BackendKind::Apple);
    let root = temp.path().join("install");
    drop(journal);
    let journal = Journal::open(&root).unwrap();
    let target = journal
        .image_target(&image.identity().state_compatibility_id)
        .unwrap();
    open_installed_image(&target).unwrap();
    let before = snapshot(&root);
    let mut worker = RuntimeWorker::new(Box::new(Backend::default()), journal, image).unwrap();
    assert!(matches!(worker.start(), Err(RuntimeError::RetainedState)));
    assert_eq!(snapshot(&root), before);
}
