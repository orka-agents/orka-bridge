// Native process tests use the desktop implementation, without the Linux UI SDK.
#[cfg(unix)]
#[path = "../../../app/src-tauri/src/runtime/spawn_guard.rs"]
pub mod spawn_guard;
#[cfg(unix)]
pub mod runtime {
    pub use crate::spawn_guard;
    pub mod credentials {
        include!("../../../app/src-tauri/src/runtime/credentials.rs");
        mod spawn_tests {
            include!("../../../app/src-tauri/src/runtime/credential_spawn_tests.rs");
        }
    }
    pub mod apple {
        include!("../../../app/src-tauri/src/runtime/apple.rs");
        mod tests {
            include!("../../../app/src-tauri/src/runtime/apple_tests.rs");
        }
    }
}
