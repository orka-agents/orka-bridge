use super::super::{image_cache::ImageCache, manifest::resolve_image};
use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;

#[cfg(windows)]
#[test]
fn original_deny_sharing_owner_is_validated_without_reopening_its_data() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("install");
    let journal = crate::runtime::journal::Journal::create(
        &root,
        crate::runtime::types::BackendKind::Qemu,
        vec![1],
    )
    .unwrap();
    let target = journal.image_target("fixture-owner").unwrap();
    validate_target(target.root, target.owner, target.id).unwrap();
    assert_eq!(
        File::open(root.join("owner.lock"))
            .unwrap_err()
            .raw_os_error(),
        Some(32),
        "identity validation must not weaken the original deny-sharing guard"
    );
}
fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn factory(path: &std::path::Path) -> VerifiedImageSet {
    factory_with_disk(path, b"private synthetic disk")
}
fn factory_with_disk(path: &std::path::Path, disk: &[u8]) -> VerifiedImageSet {
    factory_for(path, disk, ImageTarget::WindowsAmd64)
}
fn factory_for(path: &std::path::Path, disk: &[u8], target: ImageTarget) -> VerifiedImageSet {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(disk).unwrap();
    let compressed = encoder.finish().unwrap();
    let mut kernel = vec![0; 4096];
    kernel[0x1fe..0x200].copy_from_slice(b"\x55\xaa");
    kernel[0x202..0x206].copy_from_slice(b"HdrS");
    kernel[0x38..0x3c].copy_from_slice(b"ARM\x64");
    let initrd = b"070701fixture".to_vec();
    let raw: &[u8] = if target == ImageTarget::MacosArm64 {
        include_bytes!("../../../../images/testdata/v2/producer-macos.json")
    } else {
        include_bytes!("../../../../images/testdata/v2/producer-windows.json")
    };
    let mut v: serde_json::Value = serde_json::from_slice(raw).unwrap();
    v["artifact"]["uncompressedBytes"] = disk.len().into();
    v["artifact"]["uncompressedSha256"] = hash(disk).into();
    v["artifact"]["compressedBytes"] = compressed.len().into();
    v["artifact"]["compressedSha256"] = hash(&compressed).into();
    v["artifact"]["chunks"].as_array_mut().unwrap().truncate(1);
    v["artifact"]["chunks"][0]["bytes"] = compressed.len().into();
    v["artifact"]["chunks"][0]["sha256"] = hash(&compressed).into();
    v["directBoot"]["provenance"]["diskSha256"] = hash(disk).into();
    for (name, data) in [("kernel", &kernel), ("initrd", &initrd)] {
        v["directBoot"][name]["bytes"] = data.len().into();
        v["directBoot"][name]["sha256"] = hash(data).into();
    }
    v["directBoot"]["initrd"]["compression"] = "cpio-prefix".into();
    let raw = serde_json::to_vec(&v).unwrap();
    let pin = ImagePin {
        release_id: "synthetic".into(),
        manifest_url: "https://fixture.invalid/m".into(),
        manifest_sha256: hash(&raw),
        accepted_schema_sha256: hash(include_bytes!("../../../../images/manifest.schema.json")),
        target,
        asset_base: "https://fixture.invalid/".into(),
        redirect_origins: vec![],
        limits: TransferLimits {
            compressed_bytes: 8192,
            disk_bytes: 1024 * 1024,
            kernel_bytes: 8192,
            initrd_bytes: 8192,
        },
    };
    let image = resolve_image(
        &pin,
        &raw,
        &HostTarget {
            target: pin.target,
            host_version: "13.0".into(),
            app_version: "1.0.0".into(),
        },
    )
    .unwrap();
    let cache = ImageCache::open(&path.join("cache")).unwrap();
    let tx = cache.begin(&image).unwrap();
    for (i, data) in [compressed, kernel, initrd].iter().enumerate() {
        let mut f = tx.start_asset(i).unwrap().unwrap();
        f.write_all(data).unwrap();
        drop(f);
        tx.finish_asset(i).unwrap();
    }
    let (_sender, closed) = tokio::sync::watch::channel(false);
    tx.commit(&closed).unwrap()
}
fn owner(path: &std::path::Path) -> (Dir, Arc<File>) {
    std::fs::create_dir(path).unwrap();
    let root = Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap();
    let f = File::options()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path.join("owner.lock"))
        .unwrap();
    f.try_lock().unwrap();
    (root, Arc::new(f))
}
#[tokio::test]
async fn materialization_copies_private_bytes_and_reopens_a_committed_binding() {
    let temp = tempfile::tempdir().unwrap();
    let image = factory(temp.path());
    let path = temp.path().join("install");
    let (root, lock) = owner(&path);
    let id = Uuid::parse_str("12345678-1234-4234-8234-123456789abc").unwrap();
    let target = InstallationImageTarget::from_owned_installation(
        &root,
        &lock,
        id,
        ImageTarget::WindowsAmd64,
        &image.image().identity().state_compatibility_id,
        true,
    );
    let (_sender, closed) = tokio::sync::watch::channel(false);
    let installed = materialize_image(&image, &target, closed, |_| {})
        .await
        .expect("verified private copy must commit the installed binding");
    assert_eq!(installed.installation_id(), id);
    assert_eq!(
        std::fs::read(path.join(installed.disk_path())).unwrap(),
        b"private synthetic disk"
    );
    assert!(path.join("image-binding.json").is_file());
    open_installed_image(&target).unwrap();
    std::fs::write(path.join(installed.disk_path()), b"mutable guest data").unwrap();
    open_installed_image(&target)
        .expect("initial OS hash is provenance after boot, not immutable content");
}
#[path = "image_install_boundary_tests.rs"]
mod boundary;
#[path = "image_install_native_tests.rs"]
mod native;
#[path = "image_setup_tests.rs"]
mod setup;
