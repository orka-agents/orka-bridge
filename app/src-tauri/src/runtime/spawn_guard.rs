//! One guard for application-owned FD setup and child creation; never child I/O.

static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn run<T>(operation: impl FnOnce() -> T) -> T {
    let _guard = GATE.lock().await;
    operation()
}
// Native backend operations run on an owned blocking worker, never an executor.
pub(crate) fn blocking_guard() -> tokio::sync::MutexGuard<'static, ()> {
    GATE.blocking_lock()
}
#[cfg(unix)]
pub(crate) fn run_blocking<T>(operation: impl FnOnce() -> T) -> T {
    let _guard = blocking_guard();
    operation()
}
#[cfg(all(test, unix))]
#[path = "spawn_guard_tests.rs"]
pub(crate) mod tests;
