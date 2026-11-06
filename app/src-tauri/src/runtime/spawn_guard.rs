//! One guard for application-owned FD setup and child creation; never child I/O.

static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn run<T>(operation: impl FnOnce() -> T) -> T {
    let _guard = GATE.lock().await;
    operation()
}
#[cfg(unix)]
pub(crate) fn run_blocking<T>(operation: impl FnOnce() -> T) -> T {
    // Native backend operations run on the owned blocking worker, not an executor.
    let _guard = GATE.blocking_lock();
    operation()
}
#[cfg(all(test, unix))]
#[path = "spawn_guard_tests.rs"]
pub(crate) mod tests;
