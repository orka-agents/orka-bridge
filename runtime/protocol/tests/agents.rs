// Exercise native registration without Tauri system libraries.
extern crate self as bridge_engine;
#[path = "../../../app/src-tauri/src/agent_folder.rs"]
pub mod agent_folder;
#[path = "../../../app/src-tauri/src/agents.rs"]
pub mod agents;
#[path = "../../../app/src-tauri/src/runtime/snapshot.rs"]
pub mod snapshot;
pub mod runtime {
    pub use crate::snapshot;
}
include!("../../../app/src-tauri/tests/agents.rs");
