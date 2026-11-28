//! Renewal against a fake clock (Tokio's paused time) and a fake API that expires tokens.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Issues `t2`, `t3`, ... valid for `life`, or refuses as scripted.
struct FakeApi {
    calls: AtomicUsize,
    life: Duration,
    refuse: Mutex<Option<Refused>>,
}
impl FakeApi {
    fn new(life: Duration) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            life,
            refuse: Mutex::new(None),
        })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl Mint for FakeApi {
    fn mint(&self) -> MintFuture<'_> {
        Box::pin(async move {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 2;
            // A real TokenRequest takes a moment; concurrent callers must share it.
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Some(refused) = *self.refuse.lock().unwrap() {
                return Err(refused);
            }
            Ok(Minted {
                token: format!("t{n}"),
                expires: Instant::now() + self.life,
            })
        })
    }
}
fn first(life: Duration) -> Minted {
    Minted {
        token: "t1".into(),
        expires: Instant::now() + life,
    }
}
async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn renews_shortly_before_the_server_expiry_without_any_request() {
    let api = FakeApi::new(Duration::from_secs(600));
    let bearer = Arc::new(Bearer::minted(first(Duration::from_secs(600)), api.clone()));
    keep_renewed(&bearer);
    tokio::time::advance(Duration::from_secs(470)).await;
    settle().await;
    assert_eq!(api.calls(), 0, "not yet due");
    tokio::time::advance(Duration::from_secs(15)).await;
    settle().await;
    tokio::time::advance(Duration::from_millis(60)).await;
    settle().await;
    assert_eq!(api.calls(), 1, "renewed with a fifth of its life left");
    assert_eq!(bearer.peek(), "t2");
    // Long past the first token's expiry, the renewed one keeps working.
    tokio::time::advance(Duration::from_secs(200)).await;
    settle().await;
    assert_eq!(bearer.token().await.unwrap(), "t2");
    assert!(!bearer.lost());
}

#[tokio::test(start_paused = true)]
async fn follows_a_longer_or_shorter_expiry_than_requested() {
    // A cluster may extend the requested 600 s to an hour...
    let api = FakeApi::new(Duration::from_secs(3600));
    let bearer = Bearer::minted(first(Duration::from_secs(3600)), api.clone());
    tokio::time::advance(Duration::from_secs(2800)).await;
    assert_eq!(bearer.token().await.unwrap(), "t1");
    assert_eq!(api.calls(), 0);
    tokio::time::advance(Duration::from_secs(100)).await;
    assert_eq!(bearer.token().await.unwrap(), "t2");
    // ...or cap it at two minutes; renewal still lands before expiry.
    let api = FakeApi::new(Duration::from_secs(120));
    let bearer = Bearer::minted(first(Duration::from_secs(120)), api.clone());
    tokio::time::advance(Duration::from_secs(59)).await;
    assert_eq!(bearer.token().await.unwrap(), "t1");
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(bearer.token().await.unwrap(), "t2");
}

#[tokio::test(start_paused = true)]
async fn concurrent_requests_share_one_renewal() {
    let api = FakeApi::new(Duration::from_secs(600));
    let bearer = Arc::new(Bearer::minted(first(Duration::from_secs(600)), api.clone()));
    tokio::time::advance(Duration::from_secs(590)).await;
    let tasks: Vec<_> = (0..4)
        .map(|_| {
            let bearer = bearer.clone();
            tokio::spawn(async move { bearer.token().await })
        })
        .collect();
    for task in tasks {
        assert_eq!(task.await.unwrap().unwrap(), "t2");
    }
    assert_eq!(api.calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_rejected_token_is_renewed_once_and_never_loops() {
    let api = FakeApi::new(Duration::from_secs(600));
    let bearer = Bearer::minted(first(Duration::from_secs(600)), api.clone());
    assert_eq!(bearer.rejected("t1").await.unwrap(), "t2");
    // A late rejection of the replaced token reuses the renewal.
    assert_eq!(bearer.rejected("t1").await.unwrap(), "t2");
    assert_eq!(api.calls(), 1);
    // The token minted for a rejection is rejected too: the access is lost, not retried.
    assert_eq!(bearer.rejected("t2").await, Err(Lost));
    assert_eq!(bearer.token().await, Err(Lost));
    assert_eq!(bearer.rejected("t2").await, Err(Lost));
    assert_eq!(api.calls(), 1);
    assert!(bearer.lost());
}

#[tokio::test(start_paused = true)]
async fn a_renewed_token_rejected_much_later_is_renewed_again() {
    let api = FakeApi::new(Duration::from_secs(600));
    let bearer = Bearer::minted(first(Duration::from_secs(600)), api.clone());
    assert_eq!(bearer.rejected("t1").await.unwrap(), "t2");
    // Revoked minutes later, not refused at once: one more renewal is worth trying.
    tokio::time::advance(Duration::from_secs(120)).await;
    assert_eq!(bearer.rejected("t2").await.unwrap(), "t3");
    assert_eq!(api.calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn an_impossible_renewal_loses_the_access() {
    let api = FakeApi::new(Duration::from_secs(600));
    *api.refuse.lock().unwrap() = Some(Refused::Rejected);
    let bearer = Arc::new(Bearer::minted(first(Duration::from_secs(600)), api.clone()));
    keep_renewed(&bearer);
    tokio::time::advance(Duration::from_secs(481)).await;
    settle().await;
    tokio::time::advance(Duration::from_millis(60)).await;
    settle().await;
    assert!(bearer.lost(), "shown as lost before any request is made");
    assert_eq!(bearer.token().await, Err(Lost));
    assert_eq!(api.calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_unreachable_cluster_keeps_the_token_until_it_expires() {
    let api = FakeApi::new(Duration::from_secs(600));
    *api.refuse.lock().unwrap() = Some(Refused::Unreachable);
    let bearer = Bearer::minted(first(Duration::from_secs(600)), api.clone());
    tokio::time::advance(Duration::from_secs(500)).await;
    assert_eq!(bearer.token().await.unwrap(), "t1");
    let calls = api.calls();
    // Retried at a bounded pace, not on every request.
    assert_eq!(bearer.token().await.unwrap(), "t1");
    assert_eq!(api.calls(), calls);
    tokio::time::advance(Duration::from_secs(20)).await;
    assert_eq!(bearer.token().await.unwrap(), "t1");
    assert_eq!(api.calls(), calls + 1);
    // Reachable again in time: renewed.
    *api.refuse.lock().unwrap() = None;
    tokio::time::advance(Duration::from_secs(20)).await;
    assert!(bearer.token().await.unwrap().starts_with('t'));
    assert_ne!(bearer.peek(), "t1");
    // Unreachable past expiry: lost.
    *api.refuse.lock().unwrap() = Some(Refused::Unreachable);
    tokio::time::advance(Duration::from_secs(601)).await;
    assert_eq!(bearer.token().await, Err(Lost));
}

#[tokio::test(start_paused = true)]
async fn a_kubeconfig_bearer_is_never_renewed_and_reports_its_own_expiry() {
    let bearer = Arc::new(Bearer::fixed(
        "kubeconfig".into(),
        Some(Instant::now() + Duration::from_secs(60)),
    ));
    keep_renewed(&bearer);
    assert_eq!(bearer.token().await.unwrap(), "kubeconfig");
    tokio::time::advance(Duration::from_secs(61)).await;
    settle().await;
    assert!(bearer.lost());
    assert_eq!(bearer.token().await, Err(Lost));
    // Without a known expiry it works until Orka rejects it, once.
    let bearer = Bearer::fixed("static".into(), None);
    assert_eq!(bearer.token().await.unwrap(), "static");
    assert!(!bearer.lost());
    assert_eq!(bearer.rejected("static").await, Err(Lost));
    assert!(bearer.lost());
}

#[tokio::test(start_paused = true)]
async fn renewal_stops_once_the_access_is_dropped() {
    let api = FakeApi::new(Duration::from_secs(600));
    let bearer = Arc::new(Bearer::minted(first(Duration::from_secs(600)), api.clone()));
    keep_renewed(&bearer);
    drop(bearer);
    tokio::time::advance(Duration::from_secs(1200)).await;
    settle().await;
    assert_eq!(api.calls(), 0);
}
