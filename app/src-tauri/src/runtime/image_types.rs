//! Rust-owned inputs and opaque validated descriptors. None are renderer-deserializable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageTarget {
    MacosArm64,
    WindowsAmd64,
}
#[derive(Clone, Debug)]
pub struct HostTarget {
    pub target: ImageTarget,
    pub host_version: String,
    pub app_version: String,
}
#[derive(Clone, Copy, Debug)]
pub struct TransferLimits {
    pub compressed_bytes: u64,
    pub disk_bytes: u64,
    pub kernel_bytes: u64,
    pub initrd_bytes: u64,
}
/// Supplied only by trusted Rust selection. No shipping catalog or live pin is provided.
#[derive(Clone, Debug)]
pub struct ImagePin {
    pub release_id: String,
    pub manifest_url: String,
    pub manifest_sha256: String,
    pub accepted_schema_sha256: String,
    pub target: ImageTarget,
    pub asset_base: String,
    pub redirect_origins: Vec<String>,
    pub limits: TransferLimits,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Incompatibility {
    Target,
    HostVersion,
    AppVersion,
    UnsupportedVersionPolicy,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    Invalid(&'static str),
    Unsupported,
    Incompatible(Incompatibility),
    Integrity,
    LimitExceeded,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMode {
    Uefi,
    LinuxDirect,
}
#[derive(Clone, Debug)]
pub struct ImageIdentity {
    pub manifest_sha256: String,
    pub image_id: String,
    pub state_compatibility_id: String,
    pub build_source_commit: String,
}
/// Base and validated leaf stay separate; the future HTTP adapter owns URL parsing/policy.
#[derive(Clone, Debug)]
pub struct AssetLocation {
    pub(crate) base: String,
    pub(crate) leaf: String,
}
impl AssetLocation {
    pub fn base(&self) -> &str {
        &self.base
    }
    pub fn leaf(&self) -> &str {
        &self.leaf
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageAssetRole {
    Chunk(u32),
    Kernel,
    Initrd,
}
#[derive(Clone, Debug)]
pub struct ImageAsset {
    pub(crate) location: AssetLocation,
    pub(crate) role: ImageAssetRole,
    pub(crate) bytes: u64,
    pub(crate) sha256: String,
}
impl ImageAsset {
    pub fn location(&self) -> &AssetLocation {
        &self.location
    }
    pub fn role(&self) -> ImageAssetRole {
        self.role
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}
/// Only resolve_image can construct this; asset names and launch policy are validated.
#[derive(Clone, Debug)]
pub struct ResolvedImage {
    pub(crate) identity: ImageIdentity,
    pub(crate) target: ImageTarget,
    pub(crate) launch_mode: LaunchMode,
    pub(crate) command_line: String,
    pub(crate) initrd_compression: String,
    pub(crate) assets: Vec<ImageAsset>,
    pub(crate) limits: TransferLimits,
    pub(crate) compressed_sha256: String,
    pub(crate) compressed_bytes: u64,
    pub(crate) disk_sha256: String,
    pub(crate) disk_bytes: u64,
}
impl ResolvedImage {
    pub fn identity(&self) -> &ImageIdentity {
        &self.identity
    }
    pub fn target(&self) -> ImageTarget {
        self.target
    }
    pub fn launch_mode(&self) -> LaunchMode {
        self.launch_mode
    }
    pub fn command_line(&self) -> &str {
        &self.command_line
    }
    pub fn initrd_compression(&self) -> &str {
        &self.initrd_compression
    }
    pub fn assets(&self) -> &[ImageAsset] {
        &self.assets
    }
    pub fn limits(&self) -> TransferLimits {
        self.limits
    }
    pub fn compressed_sha256(&self) -> &str {
        &self.compressed_sha256
    }
    pub fn compressed_bytes(&self) -> u64 {
        self.compressed_bytes
    }
    pub fn disk_sha256(&self) -> &str {
        &self.disk_sha256
    }
    pub fn disk_bytes(&self) -> u64 {
        self.disk_bytes
    }
}
