use super::*;
use crate::engine::{
    types::{EngineAction, EngineRequest},
    EngineTransport,
};
#[tokio::test]
async fn stopping_with_an_inflight_engine_client_revokes_its_only_guest_capability() {
    let (t, journal, image) = installation();
    let resources = resource(&serving_peer("engine"));
    let mut session =
        AppleVmSession::launch(resources.path().to_path_buf(), journal, image, 1).unwrap();
    let transport = session.transport.take().unwrap();
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
