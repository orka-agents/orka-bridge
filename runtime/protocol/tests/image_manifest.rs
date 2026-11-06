// Exercise the same desktop image reader and tests without native UI SDKs.
extern crate self as bridge_engine;
#[path = "../../../app/src-tauri/src/runtime/image_types.rs"]
pub mod image_types;
#[path = "../../../app/src-tauri/src/runtime/manifest.rs"]
pub mod manifest;
pub mod runtime {
    pub use crate::{image_types, manifest};
}
include!("../../../app/src-tauri/tests/image_manifest.rs");
