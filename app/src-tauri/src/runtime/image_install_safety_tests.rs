use super::*;

#[tokio::test]
async fn persisted_launch_and_provenance_facts_have_semantic_validation() {
    let scene = Scene::new();
    scene.install().await;
    let path = scene.path().join("image-binding.json");
    let original = std::fs::read(&path).unwrap();
    let valid: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let mut cases = Vec::new();
    let mut v = valid.clone();
    v["identity"]["buildSourceCommit"] = "z".repeat(40).into();
    cases.push(v);
    let mut v = valid.clone();
    v["identity"]["imageId"] = "not an image id".into();
    cases.push(v);
    let mut v = valid.clone();
    v["boot"]["kernelRelease"] = "wrong-architecture-arm64".into();
    cases.push(v);
    let mut v = valid.clone();
    v["boot"]["provenance"]["endpointSha256"] = "invalid".into();
    cases.push(v);
    let mut v = valid.clone();
    v["boot"]["provenance"]["extractionTools"][0]["version"] = "\u{0}".into();
    cases.push(v);
    let mut v = valid.clone();
    v["boot"]["requiredDrivers"].as_array_mut().unwrap().clear();
    cases.push(v);
    let mut v = valid.clone();
    v["os"]["initialBytes"] = u64::MAX.into();
    cases.push(v);
    let mut v = valid.clone();
    v["identity"]["imageId"] = "x..y".into();
    cases.push(v);
    let mut accepted = 0;
    for value in cases {
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        if open_installed_image(&scene.target()).is_ok() {
            accepted += 1;
        }
    }
    std::fs::write(path, original).unwrap();
    assert_eq!(
        accepted, 0,
        "malformed launch/provenance facts must not become installed capabilities"
    );
}
#[cfg(unix)]
#[tokio::test]
async fn installation_symlinks_hardlinks_and_fifos_do_not_escape_the_owned_root() {
    use std::os::unix::fs::symlink;
    for kind in ["directory", "file", "hardlink", "fifo"] {
        let scene = Scene::new();
        let outside = tempfile::tempdir().unwrap();
        let keep = outside.path().join("keep");
        std::fs::write(&keep, b"outside retained bytes").unwrap();
        if kind == "directory" {
            symlink(outside.path(), scene.path().join("image")).unwrap();
        } else {
            let images = directory(&scene.root, std::ffi::OsStr::new("image")).unwrap();
            let stage = directory(&images, std::ffi::OsStr::new(".pending-1")).unwrap();
            // Obtain the exact owned intent from an interrupted, genuine copy instead of fabricating authority.
            drop(stage);
            drop(images);
            std::fs::remove_dir(scene.path().join("image/.pending-1")).unwrap();
            let mut target = scene.target();
            target.point = Some(Arc::new(|p| {
                if p == "copy-chunk" {
                    Err(ImageError::Storage(std::io::ErrorKind::Interrupted))
                } else {
                    Ok(())
                }
            }));
            let (_sender, closed) = watch::channel(false);
            materialize_image(&scene.image, &target, closed, |_| {})
                .await
                .unwrap_err();
            let pending = scene.path().join("image/.pending-1/os.disk");
            std::fs::remove_file(&pending).unwrap();
            if kind == "file" {
                symlink(&keep, pending).unwrap();
            } else if kind == "hardlink" {
                std::fs::hard_link(&keep, pending).unwrap();
            } else {
                let p = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(
                    pending.as_os_str(),
                ))
                .unwrap();
                assert_eq!(unsafe { libc::mkfifo(p.as_ptr(), 0o600) }, 0);
            }
        }
        let (_sender, closed) = watch::channel(false);
        assert!(
            materialize_image(&scene.image, &scene.target(), closed, |_| {})
                .await
                .is_err(),
            "{kind}"
        );
        assert_eq!(std::fs::read(keep).unwrap(), b"outside retained bytes");
        assert!(!scene.path().join("image-binding.json").exists());
    }
}
#[test]
fn dropping_a_queued_materialization_future_keeps_real_ownership_until_job_quiescence() {
    use std::{
        future::Future,
        pin::Pin,
        task::{Context as TaskContext, Poll, Waker},
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let scene = Scene::new();
        let (release, wait) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || wait.recv().unwrap());
        let target = scene.target();
        let (_sender, closed) = watch::channel(false);
        let mut future = Box::pin(materialize_image(&scene.image, &target, closed, |_| {}));
        let waker = Waker::noop();
        assert!(matches!(
            Pin::new(&mut future).poll(&mut TaskContext::from_waker(waker)),
            Poll::Pending
        ));
        drop(future);
        drop(target);
        let Scene {
            temp,
            image,
            root,
            lock,
            id: _,
        } = scene;
        drop(lock);
        let contender = File::options()
            .read(true)
            .write(true)
            .open(temp.path().join("install/owner.lock"))
            .unwrap();
        let busy = matches!(contender.try_lock(), Err(std::fs::TryLockError::WouldBlock));
        release.send(()).unwrap();
        blocker.await.unwrap();
        tokio::task::spawn_blocking(|| {}).await.unwrap();
        assert!(
            busy,
            "the queued actual worker must retain the provided ownership file description"
        );
        contender
            .try_lock()
            .expect("quiescence must release the temporary ownership clone");
        assert!(!temp.path().join("install/image-binding.json").exists());
        drop(root);
        drop(image);
    });
}
#[tokio::test]
async fn an_existing_foreign_generation_or_stage_is_not_adopted() {
    for name in ["1", ".pending-1"] {
        let scene = Scene::new();
        std::fs::create_dir_all(scene.path().join("image").join(name)).unwrap();
        std::fs::write(
            scene.path().join("image").join(name).join("foreign"),
            b"leave this",
        )
        .unwrap();
        let (_sender, closed) = watch::channel(false);
        assert!(
            materialize_image(&scene.image, &scene.target(), closed, |_| {})
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(scene.path().join("image").join(name).join("foreign")).unwrap(),
            b"leave this"
        );
        assert!(!scene.path().join("image-binding.json").exists());
    }
}
#[tokio::test]
async fn materialization_reports_image_bytes_without_conveying_lifecycle_authority() {
    fn send<T: Send>(_: &T) {}
    fn shared<T: Send + Sync>() {}
    shared::<InstalledImageSet>();
    let scene = Scene::new();
    let target = scene.target();
    let (_sender, closed) = watch::channel(false);
    let events = std::sync::Mutex::new(Vec::new());
    let future = materialize_image(&scene.image, &target, closed, |e| {
        events.lock().unwrap().push(e)
    });
    send(&future);
    let installed = future.await.unwrap();
    let total = scene.image.image.disk_bytes()
        + scene
            .image
            .image
            .assets()
            .iter()
            .filter(|a| !matches!(a.role(), ImageAssetRole::Chunk(_)))
            .map(|a| a.bytes())
            .sum::<u64>();
    let events = events.into_inner().unwrap();
    assert!(events.contains(&ImageProgress::Materializing {
        bytes: total,
        total
    }));
    assert_eq!(events.last(), Some(&ImageProgress::Verified));
    assert_eq!(installed.target(), ImageTarget::WindowsAmd64);
    assert_eq!(installed.launch_mode(), LaunchMode::LinuxDirect);
    assert_eq!(
        installed.root_selector(),
        "11111111-2222-4333-8444-555555555555"
    );
    assert_eq!(installed.kernel_release(), "6.1.0-fixture-amd64");
    assert_eq!(
        installed.identity().manifest_sha256,
        scene.image.image.identity().manifest_sha256
    );
    installed.revalidate(&target).unwrap();
}
