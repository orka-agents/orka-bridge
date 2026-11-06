// Exercise the actual desktop transport without requiring native UI SDKs.
#[path = "../../../app/src-tauri/src/runtime/identity.rs"]
pub mod identity;
#[path = "../../../app/src-tauri/src/runtime/image_install.rs"]
pub mod image_install;
#[path = "../../../app/src-tauri/src/runtime/manifest.rs"]
pub mod manifest;
#[path = "../../../app/src-tauri/src/runtime/setup_adapter.rs"]
pub mod setup_adapter;
#[allow(dead_code)]
#[path = "../../../app/src-tauri/src/runtime/types.rs"]
pub mod types;

extern crate self as bridge_engine;
#[path = "../../../app/src-tauri/src/runtime/acquire.rs"]
pub mod acquire;
#[path = "../../../app/src-tauri/src/runtime/image_cache.rs"]
pub mod image_cache;
#[path = "../../../app/src-tauri/src/runtime/image_types.rs"]
pub mod image_types;
pub mod runtime {
    pub use crate::{acquire, image_types};
}
include!("../../../app/src-tauri/tests/image_acquire.rs");
