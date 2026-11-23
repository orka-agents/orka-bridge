// Exercise the actual desktop cache without requiring native UI SDKs.
extern crate self as bridge_engine;
#[path = "../../../app/src-tauri/src/runtime/image_cache.rs"]
pub mod image_cache;
#[path = "../../../app/src-tauri/src/runtime/image_types.rs"]
pub mod image_types;
