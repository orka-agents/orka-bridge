// Small verified factories, not fabricated installed-image capabilities.
use super::fixture_runtime::{image_cache::ImageCache, image_types::*, manifest::resolve_image};
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path};
fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
pub fn verified(path: &Path, target: ImageTarget) -> VerifiedImageSet {
    let disk = b"private synthetic disk";
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(disk).unwrap();
    let compressed = encoder.finish().unwrap();
    let mut kernel = vec![0; 4096];
    kernel[0x1fe..0x200].copy_from_slice(b"\x55\xaa");
    kernel[0x202..0x206].copy_from_slice(b"HdrS");
    kernel[0x38..0x3c].copy_from_slice(b"ARM\x64");
    let initrd = b"070701fixture".to_vec();
    let raw: &[u8] = match target {
        ImageTarget::MacosArm64 => {
            include_bytes!("../../../../images/testdata/v2/producer-macos.json")
        }
        ImageTarget::WindowsAmd64 => {
            include_bytes!("../../../../images/testdata/v2/producer-windows.json")
        }
    };
    let mut value: serde_json::Value = serde_json::from_slice(raw).unwrap();
    value["artifact"]["uncompressedBytes"] = disk.len().into();
    value["artifact"]["uncompressedSha256"] = hash(disk).into();
    value["artifact"]["compressedBytes"] = compressed.len().into();
    value["artifact"]["compressedSha256"] = hash(&compressed).into();
    value["artifact"]["chunks"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    value["artifact"]["chunks"][0]["bytes"] = compressed.len().into();
    value["artifact"]["chunks"][0]["sha256"] = hash(&compressed).into();
    value["directBoot"]["provenance"]["diskSha256"] = hash(disk).into();
    for (name, data) in [("kernel", &kernel), ("initrd", &initrd)] {
        value["directBoot"][name]["bytes"] = data.len().into();
        value["directBoot"][name]["sha256"] = hash(data).into();
    }
    value["directBoot"]["initrd"]["compression"] = "cpio-prefix".into();
    let raw = serde_json::to_vec(&value).unwrap();
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
            target,
            host_version: "13.0".into(),
            app_version: "1.0.0".into(),
        },
    )
    .unwrap();
    let cache = ImageCache::open(&path.join("cache")).unwrap();
    let transaction = cache.begin(&image).unwrap();
    for (index, data) in [compressed, kernel, initrd].iter().enumerate() {
        let mut file = transaction.start_asset(index).unwrap().unwrap();
        file.write_all(data).unwrap();
        drop(file);
        transaction.finish_asset(index).unwrap();
    }
    let (_sender, closed) = tokio::sync::watch::channel(false);
    transaction.commit(&closed).unwrap()
}
