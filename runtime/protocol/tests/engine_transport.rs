extern crate self as bridge_engine;
#[path = "../../../app/src-tauri/src/engine/mod.rs"]
pub mod engine;
#[path = "../../../app/src-tauri/src/runtime/mod.rs"]
pub mod runtime;
pub use runtime::{client, snapshot};
include!("../../../app/src-tauri/tests/engine.rs");
