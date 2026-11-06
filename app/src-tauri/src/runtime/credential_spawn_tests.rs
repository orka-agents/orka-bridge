use super::*;
mod descriptor_fixture { include!("spawn_descriptor_fixture.rs"); }

#[cfg(unix)]
#[tokio::test]
async fn capture_deadline_includes_cancellable_wait_for_fd_guard() {
    let held = super::super::spawn_guard::tests::hold();
    let result = run_capture(
        Path::new("/nonexistent/synthetic-gh"),
        Duration::from_millis(20),
    )
    .await;
    drop(held);
    assert_eq!(result.unwrap_err(), CaptureError::TimedOut);
}

#[cfg(unix)]
#[tokio::test]
async fn actual_gh_spawn_cannot_inherit_a_private_descriptor_setup_window() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    let (window, _directory, path) = descriptor_fixture::descriptor_window();
    let mut capture = Box::pin(run_capture(&path, Duration::from_secs(2)));
    assert!(matches!(
        capture
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    drop(window);
    assert_eq!(capture.await.unwrap().as_bytes(), b"FIXTURE_ONLY_clean");
}
