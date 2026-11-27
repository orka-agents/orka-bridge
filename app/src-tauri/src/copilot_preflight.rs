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

/// A preference order, not a guarantee, and never a choice on its own: it only picks
/// which offered model the AI card preselects when the person has not chosen one.
/// The person confirms the model before Bridge connects with it.
pub const PREFERRED_MODELS: [&str; 5] = [
    "gpt-5.6-terra",
    "gpt-6.1-terra",
    "gpt-5.6-sol",
    "gpt-5.6-luna",
    "gpt-4.1",
];
/// The same request the workspace's relay makes for readiness. Some accounts report a
/// different Copilot API host (an enterprise one); the relay still asks this host, so
/// the preflight asks it too and never follows or calls another.
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
const MAX_NAME: usize = 80;
/// How many offered models the AI card's picker lists.
pub const MAX_SHOWN: usize = 50;
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

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Offered {
    pub id: String,
    /// GitHub's display name, when it sent a plain one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// See `parse` for the fields this rests on.
    #[serde(skip)]
    usable: bool,
}
/// Model ids from GitHub's reply that passed the id pattern, in GitHub's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog(Vec<Offered>);
impl Catalog {
    pub fn usable(&self, id: &str) -> bool {
        self.0.iter().any(|m| m.usable && m.id == id)
    }
    /// The usable models for the AI card's picker, in GitHub's order.
    pub fn models(&self) -> Vec<Offered> {
        self.0
            .iter()
            .filter(|m| m.usable)
            .take(MAX_SHOWN)
            .cloned()
            .collect()
    }
    /// A few usable ids, for a refusal's details.
    pub fn shown(&self) -> Vec<String> {
        self.models().into_iter().take(8).map(|m| m.id).collect()
    }
    pub fn offering(ids: &[&str]) -> Self {
        Self(
            ids.iter()
                .filter(|id| valid_id(id))
                .map(|id| Offered {
                    id: (*id).into(),
                    name: None,
                    usable: true,
                })
                .collect(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckError {
    Failure(PreflightFailure),
    /// The chosen model is not among the account's usable models.
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

/// Which row the picker starts on: the previous choice if still usable, else the first
/// usable preferred model, else the first usable model. Never connects by itself.
pub fn preselect(previous: Option<&str>, catalog: &Catalog) -> Option<String> {
    previous
        .filter(|id| catalog.usable(id))
        .or_else(|| {
            PREFERRED_MODELS
                .iter()
                .copied()
                .find(|id| catalog.usable(id))
        })
        .map(String::from)
        .or_else(|| catalog.models().into_iter().next().map(|m| m.id))
}
/// The person's chosen model must still be usable when Connect runs.
pub fn require(model: &str, catalog: &Catalog) -> Result<(), CheckError> {
    if catalog.usable(model) {
        Ok(())
    } else {
        Err(CheckError::NotOffered {
            wanted: model.into(),
            offered: catalog.shown(),
        })
    }
}

#[derive(Deserialize)]
struct Reply {
    data: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    id: String,
    #[serde(default)]
    name: Option<serde_json::Value>,
    #[serde(default)]
    capabilities: Option<serde_json::Value>,
    #[serde(default)]
    model_picker_enabled: Option<serde_json::Value>,
    #[serde(default)]
    policy: Option<serde_json::Value>,
}
/// Strict on the shape Bridge relies on, tolerant of GitHub's other fields.
/// A model is usable for agents only when the reply itself says so:
/// `capabilities.type` is `"chat"`, `capabilities.supports.tool_calls` is `true`,
/// `model_picker_enabled` is not `false`, and `policy.state`, when sent, is `"enabled"`.
/// A reply that does not say a model can call tools does not get it listed.
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
        let chat = capabilities
            .and_then(|c| c.get("type"))
            .and_then(|t| t.as_str())
            == Some("chat");
        let tools = capabilities
            .and_then(|c| c.get("supports"))
            .and_then(|s| s.get("tool_calls"))
            .and_then(|t| t.as_bool())
            == Some(true);
        let picker = entry
            .model_picker_enabled
            .as_ref()
            .is_none_or(|p| p.as_bool() != Some(false));
        let enabled = entry
            .policy
            .as_ref()
            .and_then(|p| p.get("state"))
            .is_none_or(|state| state.as_str() == Some("enabled"));
        let name = entry
            .name
            .as_ref()
            .and_then(|n| n.as_str())
            .filter(|n| {
                (1..=MAX_NAME).contains(&n.len())
                    && n.chars().all(|c| c.is_ascii_graphic() || c == ' ')
            })
            .map(String::from);
        models.push(Offered {
            id: entry.id,
            name,
            usable: chat && tools && picker && enabled,
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
    /// Unit tests of the setup state machine only: synthetic sign-ins, no token check.
    Unchecked(Catalog),
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
    /// Unit tests of the setup state machine: any token, these models, no request.
    pub fn offline(ids: &[&str]) -> Self {
        Self::with(Source::Unchecked(Catalog::offering(ids)))
    }
    fn with(source: Source) -> Self {
        Self {
            source,
            cache: Mutex::new(None),
        }
    }
    /// Blocking: call from a native worker thread, never an async task.
    pub fn catalog(&self, token: &[u8]) -> Result<Catalog, PreflightFailure> {
        self.catalog_at(token, Instant::now())
    }
    fn catalog_at(&self, token: &[u8], now: Instant) -> Result<Catalog, PreflightFailure> {
        if let Source::Unchecked(catalog) = &self.source {
            return Ok(catalog.clone());
        }
        if !token_type_ok(token) {
            return Err(PreflightFailure::TokenType);
        }
        let digest: [u8; 32] = Sha256::digest(token).into();
        let reused = self.cache.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|c| c.digest == digest && now.saturating_duration_since(c.at) < FRESH_FOR)
                .map(|c| c.catalog.clone())
        });
        if let Some(catalog) = reused {
            return Ok(catalog);
        }
        let answer = match &self.source {
            Source::Network => fetch_blocking(token),
            Source::Fixed(answer) => answer.clone(),
            Source::Unchecked(_) => unreachable!(),
        };
        if let Ok(mut cache) = self.cache.lock() {
            *cache = answer.as_ref().ok().map(|catalog| Cached {
                at: now,
                digest,
                catalog: catalog.clone(),
            });
        }
        answer
    }
    /// The catalog plus the chosen model's presence, before Connect touches the workspace.
    pub fn require(&self, token: &[u8], model: &str) -> Result<(), CheckError> {
        let catalog = self.catalog(token).map_err(CheckError::Failure)?;
        require(model, &catalog)
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
