use bridge_runtime_protocol::WorkspaceCatalog;
use serde::Serialize;

// Only the new Windows development image carries the catalog guest and Role.
// Mac's unchanged guest rejects unknown operations; release has no approved image.
pub const IMAGE_AVAILABLE: bool = cfg!(all(
    debug_assertions,
    target_os = "windows",
    target_arch = "x86_64"
));

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogReply {
    pub default_model: String,
    #[serde(flatten)]
    pub catalog: WorkspaceCatalog,
}
