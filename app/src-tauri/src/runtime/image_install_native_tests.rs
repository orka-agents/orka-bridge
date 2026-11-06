use super::*;

#[tokio::test]
async fn mac_uefi_and_windows_direct_handoffs_keep_the_verified_private_pair() {
    for platform in [ImageTarget::MacosArm64, ImageTarget::WindowsAmd64] {
        let temp = tempfile::tempdir().unwrap();
        let image = factory_for(temp.path(), b"private native fixture", platform);
        let (root, lock) = owner(&temp.path().join("install"));
        let target = InstallationImageTarget::from_owned_installation(
            &root,
            &lock,
            Uuid::new_v4(),
            platform,
            &image.image().identity().state_compatibility_id,
            true,
        );
        let (_sender, closed) = watch::channel(false);
        let installed = materialize_image(&image, &target, closed, |_| {})
            .await
            .unwrap();
        assert_eq!(installed.target(), platform);
        assert_eq!(
            installed.launch_mode(),
            if platform == ImageTarget::MacosArm64 {
                LaunchMode::Uefi
            } else {
                LaunchMode::LinuxDirect
            }
        );
        let kernel =
            std::fs::read(temp.path().join("install").join(installed.kernel_path())).unwrap();
        let initrd =
            std::fs::read(temp.path().join("install").join(installed.initrd_path())).unwrap();
        assert_eq!(
            hash(&kernel),
            image
                .image
                .assets()
                .iter()
                .find(|a| a.role() == ImageAssetRole::Kernel)
                .unwrap()
                .sha256()
        );
        assert_eq!(
            hash(&initrd),
            image
                .image
                .assets()
                .iter()
                .find(|a| a.role() == ImageAssetRole::Initrd)
                .unwrap()
                .sha256()
        );
        assert_eq!(
            installed.os_format(),
            if platform == ImageTarget::MacosArm64 {
                "raw"
            } else {
                "qcow2"
            }
        );
        assert_eq!(
            installed.kernel_format(),
            if platform == ImageTarget::MacosArm64 {
                "linux-arm64-image"
            } else {
                "linux-bzimage"
            }
        );
        assert_eq!(installed.initrd_compression(), "cpio-prefix");
        assert_eq!(
            installed.endpoint_sha256(),
            "8c8b050043ed865ac89b79113f8872923e9f6fa18aaed493c34a98c17f3a971e"
        );
        installed.revalidate(&target).unwrap();
    }
}
#[cfg(windows)]
#[tokio::test]
async fn installation_junctions_are_rejected_without_writing_the_outside_directory() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let image = factory(temp.path());
    let path = temp.path().join("install");
    let (root, lock) = owner(&path);
    let from: std::path::PathBuf = path.join("image").components().collect();
    let to: std::path::PathBuf = outside.path().components().collect();
    assert!(std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(from)
        .arg(to)
        .status()
        .unwrap()
        .success());
    let target = InstallationImageTarget::from_owned_installation(
        &root,
        &lock,
        Uuid::new_v4(),
        ImageTarget::WindowsAmd64,
        &image.image.identity().state_compatibility_id,
        true,
    );
    let (_sender, closed) = watch::channel(false);
    assert!(materialize_image(&image, &target, closed, |_| {})
        .await
        .is_err());
    assert!(outside.path().read_dir().unwrap().next().is_none());
}
#[tokio::test]
async fn repeated_materialization_reports_a_complete_existing_private_binding() {
    let scene = super::boundary::Scene::new();
    scene.install().await;
    let events = std::sync::Mutex::new(Vec::new());
    let (_sender, closed) = watch::channel(false);
    materialize_image(&scene.image, &scene.target(), closed, |e| {
        events.lock().unwrap().push(e)
    })
    .await
    .unwrap();
    assert_eq!(
        events.into_inner().unwrap().last(),
        Some(&ImageProgress::Verified)
    );
}
