use super::*;

#[cfg(unix)]
#[test]
fn transaction_drop_releases_ownership_even_while_a_fork_child_holds_the_fd() {
    const KEY: &str = "BRIDGE_TEST_IMAGE_CACHE_FORK_CHILD";
    if std::env::var_os(KEY).is_none() {
        // Keep the controlled fork away from unrelated journals in the parent test process.
        let name = format!(
            "{}::transaction_drop_releases_ownership_even_while_a_fork_child_holds_the_fd",
            module_path!().split_once("::").unwrap().1
        );
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name, "--nocapture"])
            .env(KEY, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let image = image();
    let owner = cache.begin(&image).unwrap();
    let mut pipe = [0; 2];
    assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0);
    if pid == 0 {
        unsafe {
            libc::close(pipe[1]);
            let mut byte = 0u8;
            libc::read(pipe[0], (&mut byte as *mut u8).cast(), 1);
            libc::_exit(0)
        }
    }
    unsafe {
        libc::close(pipe[0]);
    }
    drop(owner);
    let reopened = cache.begin(&image);
    let message = reopened.as_ref().err().cloned();
    let sent = unsafe { libc::write(pipe[1], b"x".as_ptr().cast(), 1) };
    unsafe {
        libc::close(pipe[1]);
    }
    let mut status = 0;
    let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
    assert_eq!(sent, 1);
    assert_eq!(waited, pid);
    assert_eq!(status, 0);
    assert!(
        reopened.is_ok(),
        "cache owner drop retained by inherited fd: {message:?}"
    );
}

#[test]
fn symlinks_or_junctions_in_pin_stage_or_generation_are_never_adopted() {
    for component in ["pin", "stage", "generation"] {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
        let image = image();
        let root = temp
            .path()
            .join("cache/images/v2")
            .join(&image.identity().manifest_sha256);
        let link = match component {
            "pin" => root.clone(),
            "stage" => {
                std::fs::create_dir(&root).unwrap();
                root.join("staging")
            }
            _ => {
                std::fs::create_dir(&root).unwrap();
                root.join("generation-1")
            }
        };
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link.components().collect::<std::path::PathBuf>())
            .arg(outside.path().components().collect::<std::path::PathBuf>())
            .status()
            .unwrap()
            .success());
        assert!(cache.begin(&image).is_err(), "{component}");
        assert_eq!(
            std::fs::read_dir(outside.path()).unwrap().count(),
            0,
            "{component}"
        );
    }
}
#[cfg(unix)]
#[test]
fn pending_hardlinks_and_special_files_are_refused_without_writing_the_target() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let (image, _) = publication::complete_image();
    let tx = cache.begin(&image).unwrap();
    let pending = temp
        .path()
        .join("cache/images/v2")
        .join(&image.identity().manifest_sha256)
        .join("staging/pending-0");
    let target = outside.path().join("retained");
    std::fs::write(&target, b"keep").unwrap();
    std::fs::hard_link(&target, &pending).unwrap();
    assert!(matches!(tx.start_asset(0), Err(ImageError::UnsafePath)));
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    std::fs::remove_file(&pending).unwrap();
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(pending.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(matches!(tx.start_asset(0), Err(ImageError::UnsafePath)));
}
#[test]
fn independent_process_observes_the_same_pin_lock() {
    const KEY: &str = "BRIDGE_TEST_IMAGE_CACHE_OWNER_ROOT";
    if let Some(root) = std::env::var_os(KEY) {
        let cache = ImageCache::open(std::path::Path::new(&root)).unwrap();
        assert!(matches!(cache.begin(&image()), Err(ImageError::Busy)));
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("cache");
    let cache = ImageCache::open(&root).unwrap();
    let _owner = cache.begin(&image()).unwrap();
    let name = format!(
        "{}::independent_process_observes_the_same_pin_lock",
        module_path!().split_once("::").unwrap().1
    );
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture"])
        .env(KEY, &root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[test]
fn cache_manifests_are_pinned_bounded_and_not_silently_refetched() {
    let temp = tempfile::tempdir().unwrap();
    let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let image = image();
    let pin = ImagePin {
        release_id: "fixture".into(),
        manifest_url: "https://fixture.invalid/m".into(),
        manifest_sha256: image.identity().manifest_sha256.clone(),
        accepted_schema_sha256: "a".repeat(64),
        target: image.target(),
        asset_base: "https://fixture.invalid/".into(),
        redirect_origins: vec![],
        limits: image.limits(),
    };
    assert!(cache.manifest(&pin).unwrap().is_none());
    let _tx = cache.begin(&image).unwrap();
    assert_eq!(
        cache.manifest(&pin).unwrap().unwrap(),
        image.manifest_bytes()
    );
    let manifest = temp
        .path()
        .join("cache/images/v2")
        .join(&pin.manifest_sha256)
        .join("staging/manifest.json");
    std::fs::write(&manifest, b"corrupt").unwrap();
    assert_eq!(cache.manifest(&pin), Err(ImageError::Integrity));
    std::fs::write(&manifest, vec![0; 8 * 1024 * 1024 + 1]).unwrap();
    assert_eq!(cache.manifest(&pin), Err(ImageError::LimitExceeded));
    let mut invalid = pin;
    invalid.manifest_sha256 = "../outside".into();
    assert_eq!(
        cache.manifest(&invalid),
        Err(ImageError::Invalid("cache pin"))
    );
}

#[test]
fn resumed_chunks_need_only_remaining_space_not_the_fresh_download_budget() {
    let temp = tempfile::tempdir().unwrap();
    let mut cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let (image, files) = publication::complete_image();
    let tx = cache.begin(&image).unwrap();
    for (i, data) in files
        .iter()
        .enumerate()
        .filter(|(i, _)| matches!(image.assets[*i].role, ImageAssetRole::Chunk(_)))
    {
        let mut f = tx.start_asset(i).unwrap().unwrap();
        f.write_all(data).unwrap();
        drop(f);
        tx.finish_asset(i).unwrap();
    }
    drop(tx);
    cache.available = Some(
        image.disk_bytes
            + image
                .assets
                .iter()
                .filter(|a| !matches!(a.role, ImageAssetRole::Chunk(_)))
                .map(|a| a.bytes)
                .sum::<u64>(),
    );
    let tx = cache
        .begin(&image)
        .expect("retained chunks leave enough space for remaining files");
    for (i, data) in files.iter().enumerate() {
        if let Some(mut f) = tx.start_asset(i).unwrap() {
            f.write_all(data).unwrap();
            drop(f);
            tx.finish_asset(i).unwrap();
        }
    }
    let (_sender, closed) = tokio::sync::watch::channel(false);
    tx.commit(&closed).unwrap();
    assert!(cache.verified(&image).unwrap().is_some());
}

pub(crate) type Point = Arc<dyn Fn(&str) -> Result<(), ImageError> + Send + Sync>;
#[test]
fn every_publication_boundary_is_recoverable_without_selecting_a_partial_set() {
    for phase in [
        "generation-record",
        "generation-published",
        "before-selector",
        "selector-published",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
        let (image, files) = publication::complete_image();
        let (_sender, closed) = tokio::sync::watch::channel(false);
        let mut tx = cache.begin(&image).unwrap();
        for (i, data) in files.iter().enumerate() {
            let mut f = tx.start_asset(i).unwrap().unwrap();
            f.write_all(data).unwrap();
            drop(f);
            tx.finish_asset(i).unwrap();
        }
        tx.point = Some(Arc::new(move |name| {
            if name == phase {
                Err(ImageError::Storage(io::ErrorKind::Interrupted))
            } else {
                Ok(())
            }
        }));
        assert!(
            matches!(
                tx.commit(&closed),
                Err(ImageError::Storage(io::ErrorKind::Interrupted))
            ),
            "{phase}"
        );
        assert_eq!(
            cache.verified(&image).unwrap().is_some(),
            phase == "selector-published"
        );
        if phase != "selector-published" {
            let tx = cache.begin(&image).unwrap();
            for i in 0..files.len() {
                assert!(tx.start_asset(i).unwrap().is_none());
            }
            tx.commit(&closed).unwrap();
        }
        assert!(cache.verified(&image).unwrap().is_some(), "{phase}");
    }
}
#[test]
fn generation_publication_never_replaces_an_existing_destination() {
    let temp = tempfile::tempdir().unwrap();
    let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let (image, files) = publication::complete_image();
    let (_sender, closed) = tokio::sync::watch::channel(false);
    let tx = cache.begin(&image).unwrap();
    for (i, data) in files.iter().enumerate() {
        let mut f = tx.start_asset(i).unwrap().unwrap();
        f.write_all(data).unwrap();
        drop(f);
        tx.finish_asset(i).unwrap();
    }
    let root = temp
        .path()
        .join("cache/images/v2")
        .join(&image.identity().manifest_sha256);
    std::fs::create_dir(root.join("generation-1")).unwrap();
    std::fs::write(root.join("generation-1/foreign"), b"retained").unwrap();
    assert!(matches!(
        tx.commit(&closed),
        Err(ImageError::Storage(io::ErrorKind::AlreadyExists))
    ));
    assert_eq!(
        std::fs::read(root.join("generation-1/foreign")).unwrap(),
        b"retained"
    );
    assert!(!root.join("ready.json").exists());
}

#[test]
fn free_space_is_rechecked_before_asset_writes_and_decompression() {
    let temp = tempfile::tempdir().unwrap();
    let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let (image, files) = publication::complete_image();
    let mut tx = cache.begin(&image).unwrap();
    tx.available = Some(0);
    assert!(matches!(tx.start_asset(0), Err(ImageError::NoSpace)));
    tx.available = None;
    for (i, data) in files.iter().enumerate() {
        let mut f = tx.start_asset(i).unwrap().unwrap();
        f.write_all(data).unwrap();
        drop(f);
        tx.finish_asset(i).unwrap();
    }
    tx.available = Some(0);
    let (_sender, closed) = tokio::sync::watch::channel(false);
    assert!(matches!(tx.commit(&closed), Err(ImageError::NoSpace)));
}

#[test]
fn unselected_complete_generation_can_recover_when_bulk_space_is_gone() {
    let temp = tempfile::tempdir().unwrap();
    let mut cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let (image, files) = publication::complete_image();
    let (_sender, closed) = tokio::sync::watch::channel(false);
    let tx = cache.begin(&image).unwrap();
    for (i, data) in files.iter().enumerate() {
        let mut f = tx.start_asset(i).unwrap().unwrap();
        f.write_all(data).unwrap();
        drop(f);
        tx.finish_asset(i).unwrap();
    }
    tx.commit(&closed).unwrap();
    let root = temp
        .path()
        .join("cache/images/v2")
        .join(&image.identity().manifest_sha256);
    std::fs::remove_file(root.join("ready.json")).unwrap();
    cache.available = Some(0);
    cache
        .begin(&image)
        .expect("no bulk writes needed to finish a complete generation")
        .commit(&closed)
        .unwrap();
    assert!(cache.verified(&image).unwrap().is_some());
}

#[test]
fn a_crash_before_manifest_publication_recovers_the_same_staging_directory() {
    let temp = tempfile::tempdir().unwrap();
    let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let image = image();
    let stage = temp
        .path()
        .join("cache/images/v2")
        .join(&image.identity().manifest_sha256)
        .join("staging");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(
        stage.join("manifest.json.pending"),
        b"partial interrupted write",
    )
    .unwrap();
    let tx = cache
        .begin(&image)
        .expect("owned unpublished staging must recover");
    assert_eq!(
        std::fs::read(stage.join("manifest.json")).unwrap(),
        image.manifest_bytes()
    );
    drop(tx);
    cache
        .begin(&image)
        .expect("owned staging retry after transaction drop");
}
