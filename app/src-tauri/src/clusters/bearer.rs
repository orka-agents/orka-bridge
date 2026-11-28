//! The Orka bearer of one confirmed access, shared by every consumer of that access.
//!
//! A ServiceAccount token Bridge minted is renewed before the server's expiry, and once when
//! Orka rejects it, by minting again for the same confirmed account through the same sign-in.
//! A kubeconfig credential is never renewed here. When renewal is impossible the access is
//! lost: it stays lost until an explicit Use replaces it, and every caller is told so.
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::time::Instant;

/// A newly issued token and when the server says it expires.
pub(crate) struct Minted {
    pub token: String,
    pub expires: Instant,
}
/// Why a renewal produced no token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The cluster could not be reached or did not answer; the current token may still work.
    Unreachable,
    /// The sign-in, account, installation or identity no longer matches: renewal is impossible.
    Rejected,
}
pub(crate) type MintFuture<'a> = Pin<Box<dyn Future<Output = Result<Minted, Refused>> + Send + 'a>>;
/// Mints a token for the one account the person confirmed, revalidating it as Use did.
pub(crate) trait Mint: Send + Sync {
    fn mint(&self) -> MintFuture<'_>;
}
/// The access can no longer authenticate to Orka; it needs reconnecting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Lost;

/// Renew once this share of a token's lifetime remains, at least this long before expiry.
const RENEW_SHARE: u32 = 5;
const RENEW_MARGIN: Duration = Duration::from_secs(60);
/// An unreachable cluster is retried at this pace until the token actually expires.
const RETRY: Duration = Duration::from_secs(15);
/// A token minted for a rejection and rejected again this soon means renewal cannot help.
const REJECTED_AGAIN: Duration = Duration::from_secs(30);

struct Current {
    token: String,
    expires: Option<Instant>,
    renew_at: Option<Instant>,
    /// When this token was minted because Orka rejected the previous one.
    after_rejection: Option<Instant>,
    lost: bool,
}
pub(crate) struct Bearer {
    current: Mutex<Current>,
    renewing: tokio::sync::Mutex<()>,
    mint: Option<Arc<dyn Mint>>,
}
fn renew_point(now: Instant, expires: Instant) -> Instant {
    let life = expires.saturating_duration_since(now);
    let margin = (life / RENEW_SHARE).max(RENEW_MARGIN.min(life / 2));
    expires.checked_sub(margin).unwrap_or(now).max(now)
}
impl Bearer {
    /// A kubeconfig credential, or none: used as is until it expires or is rejected.
    pub(crate) fn fixed(token: String, expires: Option<Instant>) -> Self {
        Self {
            current: Mutex::new(Current {
                token,
                expires,
                renew_at: expires,
                after_rejection: None,
                lost: false,
            }),
            renewing: tokio::sync::Mutex::new(()),
            mint: None,
        }
    }
    /// A token Bridge minted, renewed by `mint` for as long as it can be.
    pub(crate) fn minted(first: Minted, mint: Arc<dyn Mint>) -> Self {
        let now = Instant::now();
        Self {
            current: Mutex::new(Current {
                renew_at: Some(renew_point(now, first.expires)),
                token: first.token,
                expires: Some(first.expires),
                after_rejection: None,
                lost: false,
            }),
            renewing: tokio::sync::Mutex::new(()),
            mint: Some(mint),
        }
    }
    /// The current token without renewing; for display-free checks such as echo detection.
    pub(crate) fn peek(&self) -> String {
        self.current.lock().unwrap().token.clone()
    }
    pub(crate) fn renewable(&self) -> bool {
        self.mint.is_some()
    }
    /// True once this access can no longer authenticate: renewal was impossible, or a
    /// credential Bridge cannot renew has expired.
    pub(crate) fn lost(&self) -> bool {
        let current = self.current.lock().unwrap();
        current.lost
            || self.mint.is_none() && current.expires.is_some_and(|at| Instant::now() >= at)
    }
    /// When this bearer next needs attention: its renewal point, or a fixed credential's expiry.
    pub(crate) fn next_check(&self) -> Option<Instant> {
        let current = self.current.lock().unwrap();
        (!current.lost).then_some(current.renew_at).flatten()
    }
    /// The token to send now, renewed first when it is due.
    pub(crate) async fn token(&self) -> Result<String, Lost> {
        {
            let current = self.current.lock().unwrap();
            if current.lost {
                return Err(Lost);
            }
            if current.renew_at.is_none_or(|at| Instant::now() < at) {
                return Ok(current.token.clone());
            }
        }
        self.renew(None).await
    }
    /// Orka rejected `used`. Renews once unless another caller already replaced it; a token
    /// minted for a rejection that is rejected again at once loses the access, so 401s never loop.
    pub(crate) async fn rejected(&self, used: &str) -> Result<String, Lost> {
        self.renew(Some(used)).await
    }
    async fn renew(&self, rejected: Option<&str>) -> Result<String, Lost> {
        let _renewing = self.renewing.lock().await;
        let now = Instant::now();
        let expired = {
            let mut current = self.current.lock().unwrap();
            if current.lost {
                return Err(Lost);
            }
            match rejected {
                Some(used) if current.token != used => return Ok(current.token.clone()),
                Some(_)
                    if self.mint.is_none()
                        || current
                            .after_rejection
                            .is_some_and(|at| now.duration_since(at) < REJECTED_AGAIN) =>
                {
                    current.lost = true;
                    return Err(Lost);
                }
                None if current.renew_at.is_none_or(|at| now < at) => {
                    return Ok(current.token.clone())
                }
                _ => {}
            }
            let expired = current.expires.is_some_and(|at| now >= at);
            if self.mint.is_none() {
                // Only a fixed credential's expiry reaches here.
                if expired {
                    current.lost = true;
                    return Err(Lost);
                }
                current.renew_at = current.expires;
                return Ok(current.token.clone());
            }
            expired
        };
        let minted = self.mint.as_ref().unwrap().mint().await;
        let now = Instant::now();
        let mut current = self.current.lock().unwrap();
        match minted {
            Ok(minted) if !minted.token.is_empty() && minted.expires > now => {
                current.renew_at = Some(renew_point(now, minted.expires));
                current.expires = Some(minted.expires);
                current.token = minted.token;
                current.after_rejection = rejected.is_some().then_some(now);
                Ok(current.token.clone())
            }
            // The current token still works: keep it and try again shortly.
            Err(Refused::Unreachable) if rejected.is_none() && !expired => {
                let retry = now + RETRY;
                current.renew_at = Some(current.expires.map_or(retry, |at| retry.min(at)));
                Ok(current.token.clone())
            }
            _ => {
                current.lost = true;
                Err(Lost)
            }
        }
    }
}
/// Renews `bearer` ahead of each expiry while anything still holds it, so a quiet access is
/// renewed in time and an impossible renewal shows as lost without waiting for a request.
pub(crate) fn keep_renewed(bearer: &Arc<Bearer>) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let weak: Weak<Bearer> = Arc::downgrade(bearer);
    runtime.spawn(async move {
        loop {
            let Some(at) = weak.upgrade().and_then(|b| b.next_check()) else {
                return;
            };
            tokio::time::sleep_until(at).await;
            let Some(bearer) = weak.upgrade() else {
                return;
            };
            if bearer.token().await.is_err() {
                return;
            }
        }
    });
}

#[cfg(test)]
#[path = "bearer_tests.rs"]
mod tests;
