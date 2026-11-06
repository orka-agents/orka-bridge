extern crate self as bridge_engine;
#[path = "../../../app/src-tauri/src/runtime/client.rs"]
pub mod client;
#[path = "../../../app/src-tauri/src/engine/mod.rs"]
pub mod engine;
#[path = "../../../app/src-tauri/src/runtime/snapshot.rs"]
pub mod snapshot;
#[path = "../../../app/src-tauri/src/runtime/spawn_guard.rs"]
pub mod spawn_guard;
pub mod runtime {
    pub use crate::{client, snapshot, spawn_guard};
}
include!("../../../app/src-tauri/tests/engine.rs");
