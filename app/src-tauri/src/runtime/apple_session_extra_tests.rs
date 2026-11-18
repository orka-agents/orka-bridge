use super::*;
use crate::engine::{
    types::{EngineAction, EngineRequest},
    EngineTransport,
};
#[test]
fn stopping_with_an_inflight_engine_client_revokes_its_only_guest_capability() {
    let (t, journal, image) = installation();
    let resources = resource(&serving_peer("engine"));
    let mut session =
        AppleVmSession::launch(resources.path().to_path_buf(), journal, image, 1).unwrap();
    let transport = session.transport.take().unwrap();
    // Materialization above owns its runtime; enter the engine runtime afterward.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let task = tokio::spawn(async move {
                transport
                    .execute(EngineRequest {
                        action: EngineAction::Version {},
                    })
                    .await
            });
            let seen = t.path().join("install/request-seen");
            let end = Instant::now() + Duration::from_secs(2);
            while !seen.exists() {
                assert!(Instant::now() < end, "engine request did not reach peer");
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            session.stop().unwrap();
            assert!(tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .is_err());
            assert!(session.transport().is_none());
        });
}
#[test]
fn close_interrupts_inflight_io_before_acquiring_session_and_preserves_forced_outcome() {
    let (t, journal, image) = installation();
    let resources = resource(&serving_peer("engine"));
    let session =
        AppleVmSession::launch(resources.path().to_path_buf(), journal, image, 1).unwrap();
    let slot = session
        .worker
        .lease()
        .unwrap()
        .handle
        .get::<AppleBackendHandle>()
        .unwrap()
        .guest
        .clone();
    let (closed, close) = tokio::sync::watch::channel(false);
    slot.bind_close(close);
    crate::runtime::apple_channel::tests::short_close(&slot);
    let session = Arc::new(tokio::sync::Mutex::new(session));
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let executing = session.clone();
            let task = tokio::spawn(async move {
                let session = executing.lock().await;
                session
                    .transport()
                    .unwrap()
                    .execute(EngineRequest {
                        action: EngineAction::Version {},
                    })
                    .await
            });
            let seen = t.path().join("install/request-seen");
            let end = Instant::now() + Duration::from_secs(2);
            while !seen.exists() {
                assert!(Instant::now() < end);
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            // Same order as the facade: publish close BEFORE awaiting its session mutex.
            closed.send_replace(true);
            let stopped = tokio::time::timeout(Duration::from_secs(2), async {
                session.lock().await.stop()
            })
            .await
            .expect("active request prevented native stop");
            assert!(
                matches!(stopped, Err(RuntimeError::ShutdownForced)),
                "{stopped:?}"
            );
            assert!(task.await.unwrap().is_err());
            let mut session = session.lock().await;
            assert!(session.worker.lease().is_none());
            assert!(matches!(session.stop(), Err(RuntimeError::ShutdownForced)));
        });
    assert!(t.path().join("install/hard-stop").exists());
}
#[test]
fn helper_ack_alone_never_establishes_guest_transport() {
    let (t, journal, image) = installation();
    let resources = resource(&PEER.replace(
        "s.sendall(b'BRVM\\x01')",
        "s.sendall(b'BRVM\\x01')\ng.shutdown(socket.SHUT_WR)",
    ));
    assert!(AppleVmSession::launch(resources.path().to_path_buf(), journal, image, 1).is_err());
    let reopened = Journal::open(&t.path().join("install")).unwrap();
    assert_eq!(
        reopened.record().phase,
        crate::runtime::journal::StatePhase::Prepared
    );
}
