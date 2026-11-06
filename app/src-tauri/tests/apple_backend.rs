// Exercise actual source with private shorter budgets, isolated from journal
// reopen tests: other threads' fork-before-exec copies can retain flock briefly.
#[cfg(unix)]
#[path = "../src/runtime/spawn_guard.rs"]
pub mod spawn_guard;
#[cfg(unix)]
mod runtime {
    pub use crate::spawn_guard;
    pub mod credentials {
        include!("../src/runtime/credentials.rs");
        mod spawn_tests {
            include!("../src/runtime/credential_spawn_tests.rs");
        }
    }
    pub mod apple {
        include!("../src/runtime/apple.rs");
        mod tests {
            include!("../src/runtime/apple_tests.rs");
        }
    }
}
