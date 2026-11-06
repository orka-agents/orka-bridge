use super::super::super::{
    image_cache,
    image_types::{ImageAssetRole, ImageProgress},
};
use super::*;
use image_cache::ImageCache;

#[test]
fn dropping_the_writer_keeps_the_transaction_owned_until_queued_io_finishes() {
    use std::{
        pin::Pin,
        task::{Context, Poll, Waker},
    };
    use tokio::io::AsyncWrite;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let (image, _) = image_cache::tests::publication::complete_image();
    let owner = Arc::new(cache.begin(&image).unwrap());
    let file = owner.start_asset(0).unwrap().unwrap();
    let mut writer = asset_writer(file, owner.clone());
    let (release, wait) = std::sync::mpsc::channel();
    let blocker = runtime.spawn_blocking(move || wait.recv().unwrap());
    runtime.block_on(async {
        let result = Pin::new(&mut writer)
            .poll_write(&mut Context::from_waker(Waker::noop()), b"old queued data");
        assert!(!matches!(result, Poll::Ready(Err(_))));
    });
    drop(writer);
    drop(owner);
    let early = cache.begin(&image);
    let busy = matches!(early, Err(ImageError::Busy));
    drop(early);
    release.send(()).unwrap();
    runtime.block_on(blocker).unwrap();
    drop(runtime);
    assert!(
        busy,
        "an uncompleted queued write must still own the cache pin"
    );
    cache
        .begin(&image)
        .expect("write completed and dropped its final ownership");
}

#[tokio::test]
async fn ready_publication_between_cache_check_and_lock_is_reconciled_without_http() {
    use std::{
        future::Future,
        task::{Context, Poll, Wake, Waker},
    };
    struct Signal(Arc<tokio::sync::Notify>);
    impl Wake for Signal {
        fn wake(self: Arc<Self>) {
            self.0.notify_one();
        }
    }
    let (image, files) = image_cache::tests::publication::complete_image();
    let pin = ImagePin {
        release_id: "fixture".into(),
        manifest_url: "https://fixture.invalid/m".into(),
        manifest_sha256: image.identity().manifest_sha256.clone(),
        accepted_schema_sha256: "a".repeat(64),
        target: image.target(),
        asset_base: "https://fixture.invalid/".into(),
        redirect_origins: vec![],
        limits: image.limits(),
    };
    let client = ImageHttpClient::new(&pin).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let tx = cache.begin(&image).unwrap();
    for (i, data) in files.iter().enumerate() {
        let mut f = tx.start_asset(i).unwrap().unwrap();
        std::io::Write::write_all(&mut f, data).unwrap();
        drop(f);
        tx.finish_asset(i).unwrap();
    }
    let (_sender, closed) = watch::channel(false);
    let mut acquire = Box::pin(acquire_image(
        &image,
        &cache,
        &client,
        closed.clone(),
        |_| {},
    ));
    let notified = Arc::new(tokio::sync::Notify::new());
    let waker = Waker::from(Arc::new(Signal(notified.clone())));
    assert!(matches!(
        acquire.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    tokio::time::timeout(Duration::from_secs(5), notified.notified())
        .await
        .unwrap();
    tx.commit(&closed).unwrap();
    acquire
        .await
        .expect("a concurrent complete commit is a cache hit, not a transfer error");
}

#[tokio::test]
async fn interruption_retains_complete_chunks_and_retry_fetches_only_missing_assets() {
    let (mut image, files) = image_cache::tests::publication::complete_image();
    let server = Server::start(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\n\r\n").await;
    let mut client = server.client(true, "image.invalid");
    client.pin.manifest_sha256 = image.identity().manifest_sha256.clone();
    for (asset, data) in image.assets.iter_mut().zip(&files) {
        asset.location.base = client.pin.asset_base.clone();
        let mut response =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", data.len()).into_bytes();
        response.extend(data);
        server
            .routes
            .lock()
            .unwrap()
            .insert(format!("/{}", asset.location.leaf), response);
    }
    let temp = tempfile::tempdir().unwrap();
    let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let (sender, closed) = watch::channel(false);
    let result = acquire_image(&image, &cache, &client, closed, |event| {
        if matches!(event,ImageProgress::Download{bytes,..} if bytes>files[0].len() as u64) {
            sender.send(true).unwrap();
        }
    })
    .await;
    assert!(matches!(result, Err(ImageError::Cancelled)));
    assert!(cache.verified(&image).unwrap().is_none());
    assert_eq!(server.calls.load(Ordering::SeqCst), 2);
    let tx = cache.begin(&image).unwrap();
    assert!(tx.start_asset(0).unwrap().is_none());
    drop(tx);
    let (_sender, closed) = watch::channel(false);
    acquire_image(&image, &cache, &client, closed.clone(), |_| {})
        .await
        .unwrap();
    assert_eq!(server.calls.load(Ordering::SeqCst), 4);
    let tx_cache = ImageCache::open(&temp.path().join("busy-cache")).unwrap();
    let _owner = tx_cache.begin(&image).unwrap();
    assert!(matches!(
        acquire_image(&image, &tx_cache, &client, closed.clone(), |_| {}).await,
        Err(ImageError::Busy)
    ));
    assert_eq!(server.calls.load(Ordering::SeqCst), 4);
    let (_sender, stopped) = watch::channel(true);
    assert!(matches!(
        acquire_image(&image, &cache, &client, stopped, |_| {}).await,
        Err(ImageError::Cancelled)
    ));
    assert_eq!(server.calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn acquisition_streams_assets_reports_progress_and_reuses_without_http() {
    let (mut image, files) = image_cache::tests::publication::complete_image();
    let server = Server::start(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\n\r\n").await;
    let mut client = server.client(true, "image.invalid");
    client.pin.manifest_sha256 = image.identity().manifest_sha256.clone();
    for (asset, data) in image.assets.iter_mut().zip(&files) {
        asset.location.base = client.pin.asset_base.clone();
        let mut response =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", data.len()).into_bytes();
        response.extend(data);
        server
            .routes
            .lock()
            .unwrap()
            .insert(format!("/{}", asset.location.leaf), response);
    }
    let temp = tempfile::tempdir().unwrap();
    let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    let (_sender, closed) = watch::channel(false);
    let events = std::sync::Mutex::new(Vec::new());
    fn send<T: Send>(future: T) -> T {
        future
    }
    let set = send(acquire_image(
        &image,
        &cache,
        &client,
        closed.clone(),
        |event| events.lock().unwrap().push(event),
    ))
    .await
    .unwrap();
    assert_eq!(server.calls.load(Ordering::SeqCst), files.len());
    assert_eq!(set.image().disk_sha256(), image.disk_sha256());
    assert!(set.open_boot(ImageAssetRole::Kernel).is_ok());
    let total: u64 = files.iter().map(|b| b.len() as u64).sum();
    let events = events.into_inner().unwrap();
    let downloaded: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let ImageProgress::Download {
                bytes,
                total: bound,
            } = e
            {
                assert_eq!(*bound, total);
                Some(*bytes)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(downloaded.first(), Some(&0));
    assert_eq!(downloaded.last(), Some(&total));
    assert!(downloaded.windows(2).all(|p| p[0] <= p[1]));
    assert_eq!(events.last(), Some(&ImageProgress::Verified));
    assert_eq!(
        cache.manifest(&client.pin).unwrap().as_deref(),
        Some(image.manifest_bytes())
    );
    drop(set);
    let cache = ImageCache::open(&temp.path().join("cache")).unwrap();
    // No ephemeral root or DNS override: any accidental HTTP request would fail.
    let offline = ImageHttpClient::new(&client.pin).unwrap();
    let reused_events = std::sync::Mutex::new(Vec::new());
    acquire_image(&image, &cache, &offline, closed.clone(), |event| {
        reused_events.lock().unwrap().push(event)
    })
    .await
    .unwrap();
    assert!(reused_events
        .into_inner()
        .unwrap()
        .contains(&ImageProgress::Download {
            bytes: total,
            total
        }));
    assert_eq!(server.calls.load(Ordering::SeqCst), files.len());
    let root = temp
        .path()
        .join("cache/images/v2")
        .join(&image.identity().manifest_sha256)
        .join("generation-1");
    std::fs::write(root.join("os.disk"), b"corrupt").unwrap();
    assert!(matches!(
        acquire_image(&image, &cache, &client, closed, |_| {}).await,
        Err(ImageError::Integrity)
    ));
    assert_eq!(server.calls.load(Ordering::SeqCst), files.len());
}
