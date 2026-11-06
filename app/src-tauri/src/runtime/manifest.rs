//! Strict bridge-image/v2 consumer. Hashes establish identity, not publication approval.
use super::image_types::*;
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{cmp::Ordering, collections::BTreeSet, fmt};

const MAX_MANIFEST: usize = 8 * 1024 * 1024;
const SCHEMA: &[u8] = include_bytes!("../../../../images/manifest.schema.json");
const SCHEMA_SHA256: &str = "0347452bb75e172985c252def74cd9380f59c79655ad2dfb8ace48c5e61f596e";
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn check(ok: bool, why: &'static str) -> Result<(), ImageError> {
    if ok {
        Ok(())
    } else {
        Err(ImageError::Invalid(why))
    }
}

// serde_json::Value alone silently overwrites duplicate keys. Visit every map first,
// comparing decoded keys (so "kind" and "k\\u0069nd" collide too).
pub(crate) struct Unique(pub(crate) Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("unique-key JSON")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Unique, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Unique(Value::Number(n)))
                    .ok_or_else(|| E::custom("nonfinite number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                let mut values = Vec::new();
                while let Some(Unique(v)) = a.next_element()? {
                    values.push(v);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if values.contains_key(&k) {
                        return Err(de::Error::custom("duplicate key"));
                    }
                    let Unique(v) = a.next_value()?;
                    values.insert(k, v);
                }
                Ok(Unique(Value::Object(values)))
            }
        }
        d.deserialize_any(V)
    }
}

macro_rules! record {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Deserialize)]
        #[serde(rename_all="camelCase", deny_unknown_fields)]
        struct $name { $($field:$ty),* }
    };
}
record!(Manifest {
    schema_version: String,
    publication_ready: bool,
    image_id: String,
    build_source_commit: String,
    state_compatibility_id: String,
    target: Target,
    components: Components,
    layout: Value,
    artifact: Artifact,
    default_launch: String,
    direct_boot: Direct,
});
record!(Target {
    platform: String,
    architecture: String,
    format: String,
    minimum_host_version: String,
    minimum_app_version: String
});
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Components {
    input_schema_version: String,
    input_identity: String,
    canonical_lock_sha256: String,
    version_policy_sha256: String,
    orka: Orka,
    // deserialize_with intentionally makes this required even though null is valid.
    #[serde(deserialize_with = "required_nullable")]
    kmx_profile_sha256: Option<String>,
    kind: VersionHash,
    kubernetes: Kubernetes,
    vekil: Vekil,
    tools: Vec<Tool>,
}
fn required_nullable<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::deserialize(d)
}
record!(Orka {
    mode: String,
    identity: String,
    source_commit: String,
    chart_sha256: String
});
record!(VersionHash {
    version: String,
    sha256: String
});
record!(Kubernetes {
    version: String,
    provenance: String
});
record!(Vekil {
    manifest_digest: String,
    config_digest: String
});
record!(Tool {
    name: String,
    version: String,
    sha256: String
});
record!(Artifact { role:String, format:String, compression:String, uncompressed_bytes:u64, uncompressed_sha256:String, compressed_bytes:u64, compressed_sha256:String, chunks:Vec<Chunk> });
record!(Chunk {
    order: u32,
    name: String,
    bytes: u64,
    sha256: String
});
record!(Direct { kernel:Kernel, initrd:Initrd, root_selector:Root, kernel_release:String, command_line:String, required_drivers:Vec<Driver>, provenance:Provenance });
record!(Kernel {
    name: String,
    bytes: u64,
    sha256: String,
    format: String
});
record!(Initrd {
    name: String,
    bytes: u64,
    sha256: String,
    format: String,
    compression: String
});
record!(Root {
    kind: String,
    value: String
});
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Driver {
    name: String,
    // Both built-in and modular evidence are allowed; decoding enforces its type.
    #[serde(rename = "builtIn")]
    _built_in: bool,
}
record!(Extraction {
    name: String,
    version: String,
    executable_sha256: String
});
record!(Provenance {
    disk_sha256:String, boot_receipt_sha256:String, build_receipt_sha256:String, input_sha256:String,
    kernel_config_sha256:String, modules_sha256:String, endpoint_sha256:String, early_hook_sha256:String,
    build_hook_sha256:String, extractor_sha256:String, inspector_sha256:String, extraction_tools:Vec<Extraction>,
});
fn hex(s: &str, n: usize) -> bool {
    s.len() == n
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn id(s: &str) -> bool {
    name(s, 63, false)
}
pub(crate) fn name(s: &str, max: usize, plus: bool) -> bool {
    !s.is_empty()
        && s.len() <= max
        && (s.as_bytes()[0].is_ascii_lowercase() || s.as_bytes()[0].is_ascii_digit())
        && s.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || b".-".contains(&b)
                || (plus && b == b'+')
        })
}
fn version(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}
pub(crate) fn tool_version(s: &str) -> bool {
    let n = s.chars().count();
    (1..=4096).contains(&n)
        && s.chars()
            .all(|c| !matches!(c,'\u{0}'..='\u{8}'|'\u{b}'|'\u{c}'|'\u{e}'..='\u{1f}'|'\u{7f}'))
        && s.chars().any(|c| !c.is_whitespace())
}
fn numeric(s: &str, min: usize, max: usize) -> bool {
    let parts: Vec<_> = s.split('.').collect();
    (min..=max).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}
fn app_grammar(s: &str) -> bool {
    let (main, pre) = s.split_once('-').map_or((s, None), |(a, b)| (a, Some(b)));
    numeric(main, 3, 3)
        && pre.is_none_or(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
        })
}
fn host_cmp(a: &str, b: &str) -> Ordering {
    // Decimal-string comparison preserves arbitrarily large numeric components.
    let mut aa = a.split('.');
    let mut bb = b.split('.');
    for _ in 0..3 {
        let x = aa.next().unwrap_or("0").trim_start_matches('0');
        let y = bb.next().unwrap_or("0").trim_start_matches('0');
        let c = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
        if c != Ordering::Equal {
            return c;
        }
    }
    Ordering::Equal
}
fn validate_components(c: &Components) -> Result<(), ImageError> {
    check(
        c.input_schema_version == "bridge-guest-inputs/v1",
        "component schema",
    )?;
    check(
        [
            &c.input_identity,
            &c.canonical_lock_sha256,
            &c.version_policy_sha256,
        ]
        .iter()
        .all(|s| hex(s, 64))
            && c.kmx_profile_sha256.as_ref().is_none_or(|s| hex(s, 64)),
        "component hashes",
    )?;
    let o = &c.orka;
    check(
        matches!(o.mode.as_str(), "source" | "release")
            && version(&o.identity)
            && hex(&o.source_commit, 40)
            && hex(&o.chart_sha256, 64)
            && (o.mode != "source" || o.identity == o.source_commit),
        "Orka identity",
    )?;
    check(
        version(&c.kubernetes.version) && c.kubernetes.provenance == "policy-declared",
        "Kubernetes policy",
    )?;
    check(
        [&c.vekil.manifest_digest, &c.vekil.config_digest]
            .iter()
            .all(|s| s.strip_prefix("sha256:").is_some_and(|h| hex(h, 64))),
        "Vekil identity",
    )?;
    check(!c.tools.is_empty() && c.tools.len() <= 128, "tool count")?;
    let mut previous: Option<&str> = None;
    for t in &c.tools {
        check(
            name(&t.name, 96, true) && tool_version(&t.version) && hex(&t.sha256, 64),
            "tool identity",
        )?;
        check(
            previous.is_none_or(|p| p < t.name.as_str()),
            "tool order or duplicate",
        )?;
        previous = Some(&t.name);
    }
    let kind = c
        .tools
        .iter()
        .find(|t| t.name == "kind")
        .ok_or(ImageError::Invalid("missing kind"))?;
    check(
        version(&kind.version) && c.kind.version == kind.version && c.kind.sha256 == kind.sha256,
        "kind identity",
    )
}
fn layout(fmt: &str) -> Value {
    json!({"contractVersion":"H1/v1",
        "osDisk":{"role":"os-disk","format":fmt,"virtualCapacityBytes":17179869184u64,"blockSerial":"bridge-os","partitionTable":"gpt","boot":"uefi","privateWritableCopy":true},
        "stateDisk":{"role":"state-disk","operation":"create-empty","format":fmt,"virtualCapacityBytes":34359738368u64,"blockSerial":"bridge-state","filesystem":"ext4","partitioning":"whole-device","formatPolicy":"new-only"},
        "devices":{"contractVersion":"H1/v1","diskBus":"virtio","network":"nat","entropy":true,"clock":"utc","controlPort":"ai.orka.bridge.control","diagnostics":"separate"},
        "transport":{"protocolMajor":1,"framing":"json-lines"},"testResources":{"vcpus":4,"memoryBytes":6442450944u64}})
}
fn validate_boot(m: &Manifest, stem: &str) -> Result<(), ImageError> {
    let d = &m.direct_boot;
    let p = &d.provenance;
    let arm = m.target.architecture == "arm64";
    check(
        d.kernel.name == format!("{stem}.kernel")
            && d.initrd.name == format!("{stem}.initrd")
            && d.kernel.bytes > 0
            && d.initrd.bytes > 0
            && hex(&d.kernel.sha256, 64)
            && hex(&d.initrd.sha256, 64),
        "boot identity",
    )?;
    check(
        d.kernel.format
            == if arm {
                "linux-arm64-image"
            } else {
                "linux-bzimage"
            }
            && d.initrd.format == "initramfs"
            && matches!(
                d.initrd.compression.as_str(),
                "gzip" | "xz" | "zstd" | "cpio-prefix"
            ),
        "boot format",
    )?;
    let suffix = format!("-{}", m.target.architecture);
    check(
        d.kernel_release.strip_suffix(&suffix).is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 96
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        }),
        "kernel release",
    )?;
    let uuid = &d.root_selector.value;
    check(
        d.root_selector.kind == "filesystem-uuid"
            && uuid.len() == 36
            && uuid.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
                }
            }),
        "root selector",
    )?;
    let console = if arm { "hvc0" } else { "ttyS0,115200n8" };
    check(
        d.command_line == format!("root=UUID={uuid} ro console={console} noresume"),
        "root command line",
    )?;
    let drivers: BTreeSet<_> = d.required_drivers.iter().map(|r| r.name.as_str()).collect();
    check(
        d.required_drivers.len() == 6
            && drivers
                == BTreeSet::from([
                    "virtio_pci",
                    "virtio_blk",
                    "virtio_console",
                    "virtio_rng",
                    "virtio_net",
                    "ext4",
                ]),
        "required drivers",
    )?;
    check(
        [
            &p.disk_sha256,
            &p.boot_receipt_sha256,
            &p.build_receipt_sha256,
            &p.input_sha256,
            &p.kernel_config_sha256,
            &p.modules_sha256,
            &p.endpoint_sha256,
            &p.early_hook_sha256,
            &p.build_hook_sha256,
            &p.extractor_sha256,
            &p.inspector_sha256,
        ]
        .iter()
        .all(|s| hex(s, 64)),
        "boot provenance",
    )?;
    let tools = &m.components.tools;
    for n in ["bridge-guest", "bridge-guest-provenance"] {
        let t = tools
            .iter()
            .find(|t| t.name == n)
            .ok_or(ImageError::Invalid("missing endpoint source"))?;
        check(t.version == m.build_source_commit, "endpoint source")?;
        if n == "bridge-guest" {
            check(t.sha256 == p.endpoint_sha256, "endpoint identity")?;
        }
    }
    let names: BTreeSet<_> = p.extraction_tools.iter().map(|t| t.name.as_str()).collect();
    check(
        p.extraction_tools.len() == 4
            && names == BTreeSet::from(["guestfish", "unmkinitramfs", "readelf", "dpkg-query"]),
        "extraction roles",
    )?;
    for t in &p.extraction_tools {
        check(
            tools.iter().any(|c| {
                c.name == t.name && c.version == t.version && c.sha256 == t.executable_sha256
            }) && tool_version(&t.version),
            "extraction identity",
        )?;
    }
    check(
        p.disk_sha256 == m.artifact.uncompressed_sha256,
        "boot/disk identity",
    )
}

pub fn resolve_image(
    pin: &ImagePin,
    bytes: &[u8],
    host: &HostTarget,
) -> Result<ResolvedImage, ImageError> {
    if bytes.len() > MAX_MANIFEST {
        return Err(ImageError::LimitExceeded);
    }
    if hash(bytes) != pin.manifest_sha256 {
        return Err(ImageError::Integrity);
    }
    if pin.accepted_schema_sha256 != SCHEMA_SHA256 || hash(SCHEMA) != SCHEMA_SHA256 {
        return Err(ImageError::Unsupported);
    }
    let Unique(value) =
        serde_json::from_slice(bytes).map_err(|_| ImageError::Invalid("JSON or duplicate key"))?;
    let m: Manifest = serde_json::from_value(value)
        .map_err(|_| ImageError::Invalid("manifest fields or types"))?;
    if m.schema_version != "bridge-image/v2" {
        return Err(ImageError::Unsupported);
    }
    check(
        !m.publication_ready
            && id(&m.image_id)
            && !m.image_id.contains("..")
            && id(&m.state_compatibility_id)
            && hex(&m.build_source_commit, 40),
        "manifest identity",
    )?;
    let target = match (
        m.target.platform.as_str(),
        m.target.architecture.as_str(),
        m.target.format.as_str(),
    ) {
        ("macos", "arm64", "raw") => ImageTarget::MacosArm64,
        ("windows", "amd64", "qcow2") => ImageTarget::WindowsAmd64,
        _ => return Err(ImageError::Invalid("target tuple")),
    };
    check(
        numeric(&m.target.minimum_host_version, 2, 3) && app_grammar(&m.target.minimum_app_version),
        "version grammar",
    )?;
    validate_components(&m.components)?;
    check(m.layout == layout(&m.target.format), "H1 layout")?;
    let launch = if target == ImageTarget::MacosArm64 {
        LaunchMode::Uefi
    } else {
        LaunchMode::LinuxDirect
    };
    check(
        m.default_launch
            == if launch == LaunchMode::Uefi {
                "uefi"
            } else {
                "linux-direct"
            },
        "launch mode",
    )?;
    let stem = format!(
        "{}-{}-{}",
        m.image_id, m.target.platform, m.target.architecture
    );
    let a = &m.artifact;
    check(
        a.role == "os-disk"
            && a.format == m.target.format
            && a.compression == "gzip"
            && a.uncompressed_bytes > 0
            && a.compressed_bytes > 0
            && hex(&a.uncompressed_sha256, 64)
            && hex(&a.compressed_sha256, 64),
        "artifact",
    )?;
    check(
        !a.chunks.is_empty() && a.chunks.len() <= 50000,
        "chunk count",
    )?;
    let mut total = 0u64;
    let first = a.chunks[0].bytes;
    for (i, c) in a.chunks.iter().enumerate() {
        check(
            c.order as usize == i
                && c.name == format!("{stem}.part-{i:06}")
                && c.bytes > 0
                && c.bytes <= 1073741824
                && hex(&c.sha256, 64)
                && (i == a.chunks.len() - 1 || c.bytes == first)
                && c.bytes <= first,
            "chunk geometry or identity",
        )?;
        total = total
            .checked_add(c.bytes)
            .ok_or(ImageError::Invalid("chunk total overflow"))?;
    }
    check(total == a.compressed_bytes, "compressed size")?;
    validate_boot(&m, &stem)?;
    if target != pin.target || target != host.target {
        return Err(ImageError::Incompatible(Incompatibility::Target));
    }
    let min = semver::Version::parse(&m.target.minimum_app_version)
        .map_err(|_| ImageError::Incompatible(Incompatibility::UnsupportedVersionPolicy))?;
    let app = semver::Version::parse(&host.app_version)
        .map_err(|_| ImageError::Incompatible(Incompatibility::UnsupportedVersionPolicy))?;
    if !numeric(&host.host_version, 2, 3) {
        return Err(ImageError::Incompatible(
            Incompatibility::UnsupportedVersionPolicy,
        ));
    }
    if host_cmp(&host.host_version, &m.target.minimum_host_version) == Ordering::Less {
        return Err(ImageError::Incompatible(Incompatibility::HostVersion));
    }
    if app < min {
        return Err(ImageError::Incompatible(Incompatibility::AppVersion));
    }
    let d = &m.direct_boot;
    let limits = pin.limits;
    if a.compressed_bytes > limits.compressed_bytes
        || a.uncompressed_bytes > limits.disk_bytes
        || d.kernel.bytes > limits.kernel_bytes
        || d.initrd.bytes > limits.initrd_bytes
    {
        return Err(ImageError::LimitExceeded);
    }
    // Budget checks precede allocating the output asset set; no URI interpolation.
    let mut assets = Vec::with_capacity(a.chunks.len() + 2);
    let mut push = |leaf: String, role, bytes, sha256: String| {
        assets.push(ImageAsset {
            location: AssetLocation {
                base: pin.asset_base.clone(),
                leaf,
            },
            role,
            bytes,
            sha256,
        })
    };
    for c in &a.chunks {
        push(
            c.name.clone(),
            ImageAssetRole::Chunk(c.order),
            c.bytes,
            c.sha256.clone(),
        );
    }
    push(
        d.kernel.name.clone(),
        ImageAssetRole::Kernel,
        d.kernel.bytes,
        d.kernel.sha256.clone(),
    );
    push(
        d.initrd.name.clone(),
        ImageAssetRole::Initrd,
        d.initrd.bytes,
        d.initrd.sha256.clone(),
    );
    Ok(ResolvedImage {
        identity: ImageIdentity {
            manifest_sha256: pin.manifest_sha256.clone(),
            image_id: m.image_id,
            state_compatibility_id: m.state_compatibility_id,
            build_source_commit: m.build_source_commit,
        },
        target,
        launch_mode: launch,
        command_line: m.direct_boot.command_line,
        initrd_compression: m.direct_boot.initrd.compression,
        assets,
        limits,
        compressed_sha256: a.compressed_sha256.clone(),
        compressed_bytes: a.compressed_bytes,
        disk_sha256: a.uncompressed_sha256.clone(),
        disk_bytes: a.uncompressed_bytes,
        manifest_bytes: bytes.into(),
    })
}
