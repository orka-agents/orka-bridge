// Exercise the actual desktop transport without requiring native UI SDKs.
extern crate self as bridge_engine;
#[path = "../../../app/src-tauri/src/runtime/acquire.rs"]
pub mod acquire;
#[path = "../../../app/src-tauri/src/runtime/image_types.rs"]
pub mod image_types;
pub mod runtime {
    pub use crate::{acquire, image_types};
}
include!("../../../app/src-tauri/tests/image_acquire.rs");
