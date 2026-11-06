// Compile and exercise the same installation source without desktop SDKs.
#[path = "../../../app/src-tauri/src/runtime/acquire.rs"]
pub mod acquire;
#[path = "../../../app/src-tauri/src/runtime/setup_adapter.rs"]
pub mod setup_adapter;

#[path = "../../../app/src-tauri/src/runtime/identity.rs"]
pub mod identity;
#[path = "../../../app/src-tauri/src/runtime/image_cache.rs"]
pub mod image_cache;
#[path = "../../../app/src-tauri/src/runtime/image_install.rs"]
pub mod image_install;
#[path = "../../../app/src-tauri/src/runtime/image_types.rs"]
pub mod image_types;
#[path = "../../../app/src-tauri/src/runtime/manifest.rs"]
pub mod manifest;
// The portable image-only harness intentionally does not instantiate VM leases.
#[allow(dead_code)]
#[path = "../../../app/src-tauri/src/runtime/types.rs"]
pub mod types;
