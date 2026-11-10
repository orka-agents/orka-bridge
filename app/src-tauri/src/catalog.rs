use bridge_runtime_protocol::WorkspaceCatalog;
use serde::Serialize;

// Current pinned images predate the catalog action and reject unknown protocol
// variants by closing the channel. Enable only with the new image/compatibility pin.
pub const IMAGE_AVAILABLE: bool = false;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogReply {
    pub default_model: String,
    #[serde(flatten)]
    pub catalog: WorkspaceCatalog,
}
