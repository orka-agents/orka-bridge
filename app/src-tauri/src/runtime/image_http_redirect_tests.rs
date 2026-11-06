use super::*;

#[tokio::test]
async fn declared_short_length_refuses_before_writing_any_body_bytes() {
    let server = Server::start(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\ntin").await;
    let client = server.client(true, "image.invalid");
    let (_sender, closed) = watch::channel(false);
    let mut output = Vec::new();
    assert_eq!(
        client
            .transfer(
                &client.pin.manifest_url,
                &mut output,
                4,
                (Some(4), &hash(b"tiny")),
                closed,
                |_| {}
            )
            .await,
        Err(ImageError::Integrity)
    );
    assert!(output.is_empty());
}

#[tokio::test]
async fn manifest_bound_and_error_redaction_use_the_public_transport() {
    let server = Server::start(b"HTTP/1.1 200 OK\r\nContent-Length: 8388609\r\n\r\n").await;
    let client = server.client(true, "image.invalid");
    let (_sender, closed) = watch::channel(false);
    assert_eq!(
        client.manifest(closed).await,
        Err(ImageError::LimitExceeded)
    );
    let server =
        Server::start(b"HTTP/1.1 503 Busy\r\nContent-Length: 22\r\n\r\nprivate-response-bytes")
            .await;
    let mut client = server.client(true, "image.invalid");
    client.pin.manifest_url.push_str("?secret=fixture-only");
    let (_sender, closed) = watch::channel(false);
    assert_eq!(
        client.manifest(closed).await,
        Err(ImageError::Network("image status"))
    );
}

#[tokio::test]
async fn real_https_redirects_require_an_exact_allowed_origin() {
    let target = Server::start(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ntiny").await;
    let target_origin = format!("https://image.invalid:{}", target.address.port());
    let response = format!(
        "HTTP/1.1 302 Found\r\nLocation: {target_origin}/asset\r\nContent-Length: 0\r\n\r\n"
    );
    let source = Server::start(response.as_bytes()).await;
    let client = source.client(true, "image.invalid");
    assert_eq!(
        receive(&client, 4).await,
        Err(ImageError::Invalid("image redirect origin"))
    );
    assert_eq!(target.calls.load(Ordering::SeqCst), 0);
    let mut pin = client.pin.clone();
    pin.redirect_origins.push(target_origin);
    let mut allowed = ImageHttpClient::new(&pin).unwrap();
    allowed.client = client_builder()
        .resolve("image.invalid", source.address)
        .add_root_certificate(reqwest::Certificate::from_der(&source.root).unwrap())
        .add_root_certificate(reqwest::Certificate::from_der(&target.root).unwrap())
        .build()
        .unwrap();
    assert_eq!(receive(&allowed, 4).await.unwrap(), b"tiny");
    assert_eq!(target.calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn real_https_redirect_limit_and_url_policy_apply_before_following() {
    let server =
        Server::start(b"HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\n\r\n").await;
    assert_eq!(
        receive(&server.client(true, "image.invalid"), 4).await,
        Err(ImageError::Network("image redirect limit"))
    );
    assert_eq!(server.calls.load(Ordering::SeqCst), 6);
    for (location, error) in [
        ("http://image.invalid:{port}/", "image URL policy"),
        ("https://user:pw@image.invalid:{port}/", "image URL policy"),
        ("https://image.invalid:{port}/#fragment", "image URL policy"),
        ("https://127.0.0.1:{port}/", "loopback image origin"),
    ] {
        let response =
            format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n");
        let server = Server::start(response.as_bytes()).await;
        assert_eq!(
            receive(&server.client(true, "image.invalid"), 4).await,
            Err(ImageError::Invalid(error))
        );
        assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn unknown_length_body_cannot_exceed_the_bound() {
    let server = Server::start(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\ntiny!\r\n0\r\n\r\n",
    )
    .await;
    assert_eq!(
        receive(&server.client(true, "image.invalid"), 4).await,
        Err(ImageError::LimitExceeded)
    );
}
#[tokio::test]
async fn idle_body_timeout_and_progress_triggered_cancel_preserve_partial_only() {
    let server = Server::start(b"body-stall").await;
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(2),
            receive(&server.client(true, "image.invalid"), 4)
        )
        .await
        .unwrap(),
        Err(ImageError::Network(_))
    ));
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    let server = Server::start(b"partial-stall").await;
    let client = server.client(true, "image.invalid");
    let (sender, closed) = watch::channel(false);
    let mut output = Vec::new();
    let result = client
        .transfer(
            &client.pin.manifest_url,
            &mut output,
            4,
            (Some(4), &hash(b"tiny")),
            closed,
            |bytes| {
                assert_eq!(bytes, 2);
                sender.send(true).unwrap();
            },
        )
        .await;
    assert_eq!(result, Err(ImageError::Cancelled));
    assert_eq!(output, b"ti");
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn manifest_is_pinned_bounded_and_its_future_is_send() {
    let server = Server::start(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ntiny").await;
    let client = server.client(true, "image.invalid");
    let (_sender, closed) = watch::channel(false);
    fn send<T: Send>(future: T) -> T {
        future
    }
    assert_eq!(send(client.manifest(closed)).await.unwrap(), b"tiny");
    let (sender, closed) = watch::channel(false);
    drop(sender);
    assert_eq!(client.manifest(closed).await, Err(ImageError::Cancelled));
}
