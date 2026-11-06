use super::*;

async fn production_deadline(mode: &[u8], seconds: u64) {
    let server = Server::start(mode).await;
    let client = server.production_client();
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(seconds + 5), receive(&client, 4))
        .await
        .expect("configured transport deadline did not fire");
    assert!(matches!(result, Err(ImageError::Network(_))), "{result:?}");
    assert!(
        start.elapsed() >= Duration::from_secs(seconds - 1),
        "refusal occurred before the configured deadline"
    );
    assert_eq!(
        server.calls.load(Ordering::SeqCst),
        usize::from(mode != b"handshake-stall")
    );
}
#[tokio::test]
async fn production_connect_timeout_includes_a_stalled_tls_handshake() {
    production_deadline(b"handshake-stall", 10).await;
}
#[tokio::test]
async fn production_read_timeout_bounds_stalled_response_headers() {
    production_deadline(b"stall", 30).await;
}
#[tokio::test]
async fn production_read_timeout_bounds_an_idle_body() {
    production_deadline(b"body-stall", 30).await;
}
#[tokio::test]
async fn cancellation_while_waiting_for_headers_does_not_wait_for_idle_timeout() {
    let server = Server::start(b"stall").await;
    let client = server.production_client();
    let (sender, closed) = watch::channel(false);
    let cancel = async {
        while server.calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        sender.send(true).unwrap();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(client.manifest(closed), cancel)
    })
    .await
    .expect("cancellation did not interrupt the request");
    assert_eq!(result, Err(ImageError::Cancelled));
}
