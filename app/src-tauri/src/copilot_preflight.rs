//! Host-only GitHub Copilot preflight, run before any workspace mutation.
//!
//! The workspace's relay accepts a Copilot sign-in only after the same model
//! list request succeeds, and Connect then needs the selected model in that list.
//! Asking first on this computer turns those failures into typed outcomes before
//! Bridge writes a pending connection or delivers a sign-in.
//!
//! Self-contained on purpose: std, reqwest, serde, serde_json, sha2 and tokio only,
//! so it compiles and tests outside the Tauri crate.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// The model Bridge asks for when the person has not chosen one.
pub const DEFAULT_MODEL: &str = "gpt-5.6-terra";
/// A preference order, not a guarantee. When the configured model is not in an
/// account's catalog, Bridge uses the first of these the catalog offers with tool
/// calling (the example agent depends on web_fetch). Each entry must also be
/// accepted by the bundled example (`agent_folder::example_for_model`).
pub const PREFERRED_MODELS: [&str; 5] = [
    DEFAULT_MODEL,
    "gpt-6.1-terra",
    "gpt-5.6-sol",
    "gpt-5.6-luna",
    "gpt-4.1",
];
/// The same request the workspace's relay makes for readiness.
pub const MODELS_URL: &str = "https://api.githubcopilot.com/models";
const HEADERS: [(&str, &str); 5] = [
    ("copilot-integration-id", "copilot-developer-cli"),
    ("editor-version", "vscode/1.95.0"),
    ("editor-plugin-version", "copilot-chat/0.26.7"),
    ("user-agent", "GitHubCopilotChat/0.26.7"),
    ("x-github-api-version", "2026-08-20"),
];
const MAX_BODY: usize = 1024 * 1024;
const MAX_MODELS: usize = 512;
const MAX_ID: usize = 100;
/// How many offered ids a ModelNotOffered failure carries for the AI card.
pub const MAX_SHOWN: usize = 8;
/// A successful answer is reused for the same sign-in for this long.
pub const FRESH_FOR: Duration = Duration::from_secs(5 * 60);

/// Closed outcomes. None of them carries the token or any reply text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreflightFailure {
    /// Not an OAuth, GitHub App user or fine-grained token; the relay refuses classic PATs.
    TokenType,
    /// 401: GitHub did not accept the sign-in.
    SignInRejected,
    /// 403: no Copilot access, or the organisation blocks Copilot in the command line.
    AccessDenied,
    /// Network failure, timeout or a 5xx/429 reply.
    Unreachable,
    /// Any other reply, including redirects, oversized or wrongly shaped bodies.
    Malformed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Offered {
    id: String,
    /// False when the reply says the model is not a chat model or cannot call tools.
    usable: bool,
}
/// Model ids from GitHub's reply that passed the id pattern, in GitHub's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog(Vec<Offered>);
impl Catalog {
    pub fn usable(&self, id: &str) -> bool {
        self.0.iter().any(|m| m.usable && m.id == id)
    }
    /// A few usable ids for the AI card, in GitHub's order.
    pub fn shown(&self) -> Vec<String> {
        self.0
            .iter()
            .filter(|m| m.usable)
            .take(MAX_SHOWN)
            .map(|m| m.id.clone())
            .collect()
    }
    pub fn offering(ids: &[&str]) -> Self {
        Self(
            ids.iter()
                .filter(|id| valid_id(id))
                .map(|id| Offered {
                    id: (*id).into(),
                    usable: true,
                })
                .collect(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    /// The model Bridge connects with.
    pub model: String,
    /// The model Bridge asked for; differs from `model` after a fallback.
    pub wanted: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckError {
    Failure(PreflightFailure),
    /// Neither the wanted model nor any preferred one is offered.
    NotOffered {
        wanted: String,
        offered: Vec<String>,
    },
}

/// Conservative model id shape: what Bridge will put in a selection and an agent file.
pub fn valid_id(id: &str) -> bool {
    (1..=MAX_ID).contains(&id.len())
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        && !id.eq_ignore_ascii_case("auto")
}

/// Token type from its prefix only; no network.
pub fn token_type_ok(token: &[u8]) -> bool {
    ["gho_", "ghu_", "github_pat_"].iter().any(|prefix| {
        token.len() > prefix.len()
            && token.len() <= 1024
            && token.starts_with(prefix.as_bytes())
            && token[prefix.len()..]
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
    })
}

/// Configured model if usable, else the first usable preferred model.
pub fn choose(wanted: &str, catalog: &Catalog) -> Result<ModelChoice, CheckError> {
    let pick = |model: &str| ModelChoice {
        model: model.into(),
        wanted: wanted.into(),
    };
    if catalog.usable(wanted) {
        return Ok(pick(wanted));
    }
    PREFERRED_MODELS
        .iter()
        .find(|id| catalog.usable(id))
        .map(|id| pick(id))
        .ok_or_else(|| CheckError::NotOffered {
            wanted: wanted.into(),
            offered: catalog.shown(),
        })
}

#[derive(Deserialize)]
struct Reply {
    data: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    id: String,
    #[serde(default)]
    capabilities: Option<serde_json::Value>,
}
/// Strict on the shape Bridge relies on, tolerant of GitHub's other fields.
pub fn parse(body: &[u8]) -> Result<Catalog, PreflightFailure> {
    if body.len() > MAX_BODY {
        return Err(PreflightFailure::Malformed);
    }
    let reply: Reply = serde_json::from_slice(body).map_err(|_| PreflightFailure::Malformed)?;
    if reply.data.len() > MAX_MODELS {
        return Err(PreflightFailure::Malformed);
    }
    let mut models: Vec<Offered> = Vec::new();
    for entry in reply.data {
        if !valid_id(&entry.id) || models.iter().any(|m| m.id == entry.id) {
            continue;
        }
        let capabilities = entry.capabilities.as_ref();
        let kind = capabilities.and_then(|c| c.get("type"));
        let tools = capabilities
            .and_then(|c| c.get("supports"))
            .and_then(|s| s.get("tool_calls"));
        let usable = kind.is_none_or(|t| t.as_str() == Some("chat"))
            && tools.is_none_or(|t| t.as_bool() != Some(false));
        models.push(Offered {
            id: entry.id,
            usable,
        });
    }
    Ok(Catalog(models))
}

#[derive(Clone, Copy)]
pub struct Limits {
    pub connect: Duration,
    pub total: Duration,
    pub https_only: bool,
}
impl Limits {
    pub const NETWORK: Self = Self {
        connect: Duration::from_secs(10),
        total: Duration::from_secs(20),
        https_only: true,
    };
}

/// One GET, at most one retry after a connection failure or timeout.
pub async fn fetch_from(
    url: &str,
    token: &[u8],
    limits: Limits,
) -> Result<Catalog, PreflightFailure> {
    use reqwest::header::{HeaderValue, AUTHORIZATION};
    if !token_type_ok(token) {
        return Err(PreflightFailure::TokenType);
    }
    // The workspace's relay gets no proxy settings, so a request that only works
    // through this computer's proxy would pass here and still fail there.
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .https_only(limits.https_only)
        .connect_timeout(limits.connect)
        .timeout(limits.total)
        .build()
        .map_err(|_| PreflightFailure::Unreachable)?;
    let mut bearer = b"Bearer ".to_vec();
    bearer.extend_from_slice(token);
    let mut authorization =
        HeaderValue::from_bytes(&bearer).map_err(|_| PreflightFailure::TokenType)?;
    authorization.set_sensitive(true);
    let mut attempt = 0;
    let mut response = loop {
        attempt += 1;
        let mut request = client
            .get(url)
            .header(AUTHORIZATION, authorization.clone())
            .header(reqwest::header::ACCEPT_ENCODING, "identity");
        for (name, value) in HEADERS {
            request = request.header(name, value);
        }
        // Never format the error: only its class decides the outcome.
        match request.send().await {
            Ok(response) => break response,
            Err(error) if attempt == 1 && (error.is_connect() || error.is_timeout()) => continue,
            Err(_) => return Err(PreflightFailure::Unreachable),
        }
    };
    let status = response.status().as_u16();
    match status {
        200 => {}
        401 => return Err(PreflightFailure::SignInRejected),
        403 => return Err(PreflightFailure::AccessDenied),
        429 | 500..=599 => return Err(PreflightFailure::Unreachable),
        _ => return Err(PreflightFailure::Malformed),
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err(PreflightFailure::Malformed);
    }
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > MAX_BODY {
                    return Err(PreflightFailure::Malformed);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) => return Err(PreflightFailure::Unreachable),
        }
    }
    parse(&body)
}

enum Source {
    Network,
    Fixed(Result<Catalog, PreflightFailure>),
    /// Unit tests of the setup state machine only: every wanted model is offered,
    /// whatever the token, without any request.
    AnyWanted,
}
struct Cached {
    at: Instant,
    digest: [u8; 32],
    catalog: Catalog,
}
/// Owns the answer source and the last successful answer, keyed by a digest of the
/// sign-in so a changed sign-in always asks again. Holds no token.
pub struct Preflight {
    source: Source,
    cache: Mutex<Option<Cached>>,
}
impl std::fmt::Debug for Preflight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Preflight")
    }
}
impl Preflight {
    pub fn network() -> Self {
        Self::with(Source::Network)
    }
    /// A fixed answer, for fixtures and tests; no request is made.
    pub fn fixed(answer: Result<Catalog, PreflightFailure>) -> Self {
        Self::with(Source::Fixed(answer))
    }
    pub fn any_wanted() -> Self {
        Self::with(Source::AnyWanted)
    }
    fn with(source: Source) -> Self {
        Self {
            source,
            cache: Mutex::new(None),
        }
    }
    /// Blocking: call from a native worker thread, never an async task.
    pub fn check(&self, token: &[u8], wanted: &str) -> Result<ModelChoice, CheckError> {
        self.check_at(token, wanted, Instant::now())
    }
    fn check_at(
        &self,
        token: &[u8],
        wanted: &str,
        now: Instant,
    ) -> Result<ModelChoice, CheckError> {
        if matches!(self.source, Source::AnyWanted) {
            return Ok(ModelChoice {
                model: wanted.into(),
                wanted: wanted.into(),
            });
        }
        if !token_type_ok(token) {
            return Err(CheckError::Failure(PreflightFailure::TokenType));
        }
        let digest: [u8; 32] = Sha256::digest(token).into();
        let reused = self.cache.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|c| c.digest == digest && now.saturating_duration_since(c.at) < FRESH_FOR)
                .map(|c| c.catalog.clone())
        });
        let catalog = match reused {
            Some(catalog) => catalog,
            None => {
                let answer = match &self.source {
                    Source::Network => fetch_blocking(token),
                    Source::Fixed(answer) => answer.clone(),
                    Source::AnyWanted => unreachable!(),
                };
                if let Ok(mut cache) = self.cache.lock() {
                    *cache = answer.as_ref().ok().map(|catalog| Cached {
                        at: now,
                        digest,
                        catalog: catalog.clone(),
                    });
                }
                answer.map_err(CheckError::Failure)?
            }
        };
        choose(wanted, &catalog)
    }
    /// Drop a remembered answer, e.g. after the sign-in is removed.
    pub fn forget(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            *cache = None;
        }
    }
}

fn fetch_blocking(token: &[u8]) -> Result<Catalog, PreflightFailure> {
    let future = fetch_from(MODELS_URL, token, Limits::NETWORK);
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(future),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| PreflightFailure::Unreachable)?
            .block_on(future),
    }
}

#[cfg(test)]
#[path = "copilot_preflight_tests.rs"]
mod tests;
