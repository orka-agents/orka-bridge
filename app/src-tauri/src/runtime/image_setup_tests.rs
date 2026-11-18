use super::*;

// A bypassed materializer or lost source facts must break this real filesystem handoff.
#[tokio::test]
async fn prepared_cached_source_commits_the_private_boot_set() {
    let temp = tempfile::tempdir().unwrap();
    let image = factory(temp.path());
    let compatibility = image.image().identity().state_compatibility_id.clone();
    let prepared = PreparedImage::from_verified(image).unwrap();
    assert_eq!(prepared.target(), ImageTarget::WindowsAmd64);
    assert_eq!(
        prepared.identity().state_compatibility_id,
        "fixture-state-v1"
    );
    let path = temp.path().join("install");
    let (root, owner) = owner(&path);
    let target = InstallationImageTarget::from_owned_installation(
        &root,
        &owner,
        Uuid::new_v4(),
        ImageTarget::WindowsAmd64,
        &compatibility,
        true,
    );
    let (_sender, closed) = watch::channel(false);
    let installed = prepared
        .materialize(&target, closed, |_| {})
        .await
        .expect("prepared source must commit an installed image, not just verify its cache");
    assert_eq!(
        std::fs::read(path.join(installed.disk_path())).unwrap(),
        b"private synthetic disk"
    );
    assert!(path.join("image-binding.json").is_file());
    installed.revalidate(&target).unwrap();
}
