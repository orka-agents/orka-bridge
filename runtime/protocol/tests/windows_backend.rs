// Exercise the exact desktop implementation without Tauri system libraries.
#[cfg(debug_assertions)]
#[path = "../../../app/src-tauri/src/dev_image.rs"]
pub mod dev_image;
extern crate self as bridge_engine;
#[allow(dead_code)]
pub mod engine {
    pub mod types {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../app/src-tauri/src/engine/types.rs"
        ));
    }
}
#[path = "../../../app/src-tauri/src/runtime/mod.rs"]
pub mod runtime;
include!("../../../app/src-tauri/tests/windows_backend.rs");
