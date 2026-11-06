use super::*;
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll, Waker},
    time::Duration,
};

pub(crate) struct HeldGate {
    pub(crate) release: Option<std::sync::mpsc::Sender<()>>,
    pub(crate) thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for HeldGate {
    fn drop(&mut self) {
        let _ = self.release.take().unwrap().send(());
        self.thread.take().unwrap().join().unwrap();
    }
}
pub(crate) fn hold() -> HeldGate {
    let (ready, entered) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        run_blocking(|| {
            ready.send(()).unwrap();
            let _ = wait.recv();
        })
    });
    entered.recv().unwrap();
    HeldGate {
        release: Some(release),
        thread: Some(thread),
    }
}

#[tokio::test]
async fn descriptor_setup_excludes_other_app_spawns() {
    // Removing the shared lock lets this closure run inside an unsafe FD window.
    let held = hold();
    let mut launch = Box::pin(run(|| true));
    let poll = launch
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()));
    assert!(
        matches!(poll, Poll::Pending),
        "spawn entered the descriptor-setup window"
    );
    drop(held);
    assert!(launch.await);
}

#[tokio::test]
async fn cancelled_guard_wait_never_executes_its_spawn_closure() {
    let held = hold();
    let spawned = Arc::new(AtomicBool::new(false));
    let result = tokio::time::timeout(
        Duration::from_millis(20),
        run(|| spawned.store(true, Ordering::SeqCst)),
    )
    .await;
    assert!(result.is_err());
    drop(held);
    assert!(!spawned.load(Ordering::SeqCst));
}
