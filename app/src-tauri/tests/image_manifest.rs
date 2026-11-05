use bridge_engine::runtime::{image_types::*, manifest::resolve_image};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

const SCHEMA: &[u8] = include_bytes!("../../../images/manifest.schema.json");
const INDEX: &str = include_str!("../../../images/testdata/v2/vectors.json");
const WINDOWS: &[u8] = include_bytes!("../../../images/testdata/v2/producer-windows.json");
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn pin(bytes: &[u8], target: ImageTarget) -> ImagePin {
    ImagePin {
        release_id: "synthetic-not-a-release".into(),
        manifest_url: "https://fixtures.invalid/fixed/manifest.json".into(),
        manifest_sha256: hash(bytes),
        accepted_schema_sha256: hash(SCHEMA),
        target,
        asset_base: "https://fixtures.invalid/fixed/".into(),
        redirect_origins: vec![],
        limits: TransferLimits {
            compressed_bytes: 4096,
            disk_bytes: 4096,
            kernel_bytes: 8192,
            initrd_bytes: 4096,
        },
    }
}
fn host(target: ImageTarget) -> HostTarget {
    HostTarget {
        target,
        host_version: "13.0".into(),
        app_version: "1.0.0".into(),
    }
}
fn verdict(result: &Result<ResolvedImage, ImageError>) -> String {
    match result {
        Ok(_) => "compatible".into(),
        Err(ImageError::Incompatible(reason)) => format!("{reason:?}"),
        Err(e) => format!("{e:?}"),
    }
}
#[test]
fn shared_raw_vectors_enforce_structure_and_separate_compatibility() {
    let rows: Vec<Value> = serde_json::from_str(INDEX).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|p| p.join("images/testdata/v2"))
        .find(|p| p.is_dir())
        .unwrap();
    for row in rows {
        let raw = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&raw), row["sha256"].as_str().unwrap());
        let target = if row["platform"] == "macos" {
            ImageTarget::MacosArm64
        } else {
            ImageTarget::WindowsAmd64
        };
        let mut h = host(target);
        if let Some(v) = row["hostVersion"].as_str() {
            h.host_version = v.into();
        }
        if let Some(v) = row["appVersion"].as_str() {
            h.app_version = v.into();
        }
        let result = resolve_image(&pin(&raw, target), &raw, &h);
        if row["structure"] == "invalid" {
            assert!(
                matches!(
                    result,
                    Err(ImageError::Invalid(_)) | Err(ImageError::Unsupported)
                ),
                "{}: {:?}",
                row["invariant"],
                result
            );
        } else {
            assert_eq!(
                verdict(&result),
                row["compatibility"].as_str().unwrap(),
                "{}",
                row["invariant"]
            );
        }
    }
}
#[test]
fn schema_pin_raw_hash_and_size_precede_json_decoding() {
    let mut p = pin(WINDOWS, ImageTarget::WindowsAmd64);
    p.manifest_sha256 = hash(b"{}");
    assert_eq!(
        resolve_image(&p, b"not JSON", &host(p.target)).unwrap_err(),
        ImageError::Integrity
    );
    let mut at_cap = WINDOWS.to_vec();
    at_cap.resize(8 * 1024 * 1024, b' ');
    assert!(resolve_image(&pin(&at_cap, p.target), &at_cap, &host(p.target)).is_ok());
    let huge = vec![b' '; 8 * 1024 * 1024 + 1];
    p.manifest_sha256 = hash(&huge);
    assert_eq!(
        resolve_image(&p, &huge, &host(p.target)).unwrap_err(),
        ImageError::LimitExceeded
    );
    p = pin(WINDOWS, p.target);
    p.accepted_schema_sha256 = "0".repeat(64);
    assert_eq!(
        resolve_image(&p, WINDOWS, &host(p.target)).unwrap_err(),
        ImageError::Unsupported
    );
    // Same decoded JSON, different raw bytes: must not canonicalize before hashing.
    let mut raw = WINDOWS.to_vec();
    raw.push(b' ');
    assert_eq!(
        resolve_image(&pin(WINDOWS, p.target), &raw, &host(p.target)).unwrap_err(),
        ImageError::Integrity
    );
}
#[test]
fn target_and_all_measured_ceilings_are_trusted_not_manifest_overrides() {
    let p = pin(WINDOWS, ImageTarget::WindowsAmd64);
    let h = host(p.target);
    let mut wrong = p.clone();
    wrong.target = ImageTarget::MacosArm64;
    assert_eq!(
        resolve_image(&wrong, WINDOWS, &h).unwrap_err(),
        ImageError::Incompatible(Incompatibility::Target)
    );
    assert_eq!(
        resolve_image(&p, WINDOWS, &host(ImageTarget::MacosArm64)).unwrap_err(),
        ImageError::Incompatible(Incompatibility::Target)
    );
    let mut exact = p.clone();
    exact.limits = TransferLimits {
        compressed_bytes: 61,
        disk_bytes: 43,
        kernel_bytes: 4096,
        initrd_bytes: 21,
    };
    assert!(resolve_image(&exact, WINDOWS, &h).is_ok());
    for i in 0..4 {
        let mut tiny = p.clone();
        match i {
            0 => tiny.limits.compressed_bytes = 1,
            1 => tiny.limits.disk_bytes = 1,
            2 => tiny.limits.kernel_bytes = 1,
            _ => tiny.limits.initrd_bytes = 1,
        };
        assert_eq!(
            resolve_image(&tiny, WINDOWS, &h).unwrap_err(),
            ImageError::LimitExceeded
        );
    }
}
#[test]
fn resolved_output_retains_declared_initrd_compression() {
    for compression in ["gzip", "xz", "zstd", "cpio-prefix"] {
        let mut descriptor: Value = serde_json::from_slice(WINDOWS).unwrap();
        descriptor["directBoot"]["initrd"]["compression"] = compression.into();
        let raw = serde_json::to_vec(&descriptor).unwrap();
        let p = pin(&raw, ImageTarget::WindowsAmd64);
        let image = resolve_image(&p, &raw, &host(p.target)).unwrap();
        assert_eq!(image.initrd_compression(), compression);
    }
}
#[test]
fn resolved_output_carries_only_validated_locations_and_identity() {
    let p = pin(WINDOWS, ImageTarget::WindowsAmd64);
    let image = resolve_image(&p, WINDOWS, &host(p.target)).unwrap();
    assert_eq!(image.identity().manifest_sha256, hash(WINDOWS));
    assert_eq!(image.identity().build_source_commit, "a".repeat(40));
    assert_eq!(image.identity().state_compatibility_id, "fixture-state-v1");
    assert_eq!(image.target(), ImageTarget::WindowsAmd64);
    assert_eq!(image.launch_mode(), LaunchMode::LinuxDirect);
    assert!(image
        .command_line()
        .ends_with("ro console=ttyS0,115200n8 noresume"));
    assert_eq!(image.assets().len(), 6);
    for asset in image.assets() {
        assert_eq!(asset.location().base(), "https://fixtures.invalid/fixed/");
        assert!(asset
            .location()
            .leaf()
            .starts_with("synthetic-not-bootable-windows-amd64."));
        assert!(!asset.location().leaf().contains('/'));
        assert!(asset.bytes() > 0);
    }
}
