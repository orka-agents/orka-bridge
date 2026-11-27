use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

const TOKEN: &[u8] = b"gho_synthetic0preflight0only";

/// A local stand-in for the models endpoint. Each accepted connection gets the next
/// canned reply; `None` holds the connection open without answering.
struct Fake {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}
fn fake(replies: Vec<Option<Vec<u8>>>) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/models", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    thread::spawn(move || {
        for reply in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&head).into_owned());
            // Each connection is answered on its own thread, so a held one never blocks the next.
            thread::spawn(move || match reply {
                Some(bytes) => {
                    let _ = stream.write_all(&bytes);
                }
                None => thread::sleep(Duration::from_millis(1500)),
            });
        }
    });
    Fake { url, requests }
}
fn http(status: &str, body: &[u8]) -> Option<Vec<u8>> {
    let mut reply = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    reply.extend_from_slice(body);
    Some(reply)
}
const LIMITS: Limits = Limits {
    connect: Duration::from_millis(500),
    total: Duration::from_millis(500),
    https_only: false,
};
async fn run(fake: &Fake) -> Result<Catalog, PreflightFailure> {
    fetch_from(&fake.url, TOKEN, LIMITS).await
}
const LIVE_SHAPE: &[u8] = br#"{"object":"list","data":[
  {"id":"gpt-5.6-luna","object":"model","model_picker_enabled":true,"capabilities":{"type":"chat","family":"x","supports":{"tool_calls":true,"streaming":true}},"supported_endpoints":["/responses"],"policy":{"state":"enabled"}},
  {"id":"gpt-6.1-terra","name":"GPT-6.1 Terra","capabilities":{"type":"chat","supports":{"tool_calls":true}}},
  {"id":"hidden-legacy","model_picker_enabled":false,"capabilities":{"type":"chat","supports":{"tool_calls":true}}},
  {"id":"needs-opt-in","policy":{"state":"unconfigured"},"capabilities":{"type":"chat","supports":{"tool_calls":true}}},
  {"id":"silent-tools","capabilities":{"type":"chat","supports":{}}},
  {"id":"text-embedding-3-small","capabilities":{"type":"embeddings","supports":{}}},
  {"id":"gpt-4.1","capabilities":{"type":"chat","supports":{"tool_calls":true}}},
  {"id":"no-tools","capabilities":{"type":"chat","supports":{"tool_calls":false}}}
]}"#;

#[tokio::test]
async fn sends_exactly_the_relay_request_and_parses_its_reply() {
    let server = fake(vec![http("200 OK", LIVE_SHAPE)]);
    let catalog = run(&server).await.unwrap();
    assert!(catalog.usable("gpt-6.1-terra") && catalog.usable("gpt-4.1"));
    assert!(!catalog.usable("text-embedding-3-small") && !catalog.usable("no-tools"));
    assert_eq!(
        catalog
            .models()
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["gpt-5.6-luna", "gpt-6.1-terra", "gpt-4.1"]
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let head = requests[0].to_ascii_lowercase();
    assert!(head.starts_with("get /models http/1.1\r\n"), "{head}");
    for line in [
        "authorization: bearer gho_synthetic0preflight0only\r\n",
        "copilot-integration-id: copilot-developer-cli\r\n",
        "editor-version: vscode/1.95.0\r\n",
        "editor-plugin-version: copilot-chat/0.26.7\r\n",
        "user-agent: githubcopilotchat/0.26.7\r\n",
        "x-github-api-version: 2026-08-20\r\n",
    ] {
        assert!(head.contains(line), "missing {line:?}");
    }
}

#[tokio::test]
async fn statuses_map_to_typed_outcomes_without_retrying_a_reply() {
    for (status, outcome) in [
        ("401 Unauthorized", PreflightFailure::SignInRejected),
        ("403 Forbidden", PreflightFailure::AccessDenied),
        ("500 Internal Server Error", PreflightFailure::Unreachable),
        ("503 Service Unavailable", PreflightFailure::Unreachable),
        ("429 Too Many Requests", PreflightFailure::Unreachable),
        ("404 Not Found", PreflightFailure::Malformed),
    ] {
        let server = fake(vec![http(status, b"{}"), http("200 OK", LIVE_SHAPE)]);
        assert_eq!(run(&server).await, Err(outcome), "{status}");
        assert_eq!(server.requests.lock().unwrap().len(), 1, "{status}");
    }
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let server = fake(vec![
        Some(b"HTTP/1.1 302 Found\r\nlocation: http://example.invalid/models\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec()),
        http("200 OK", LIVE_SHAPE),
    ]);
    assert_eq!(run(&server).await, Err(PreflightFailure::Malformed));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_timeout_is_retried_once_then_unreachable() {
    let server = fake(vec![None, None, http("200 OK", LIVE_SHAPE)]);
    let started = Instant::now();
    assert_eq!(run(&server).await, Err(PreflightFailure::Unreachable));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(server.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_timeout_then_an_answer_succeeds() {
    let server = fake(vec![None, http("200 OK", LIVE_SHAPE)]);
    assert!(run(&server).await.unwrap().usable("gpt-4.1"));
}

#[tokio::test]
async fn nothing_listening_is_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/models", listener.local_addr().unwrap());
    drop(listener);
    assert_eq!(
        fetch_from(&url, TOKEN, LIMITS).await,
        Err(PreflightFailure::Unreachable)
    );
}

#[tokio::test]
async fn hostile_bodies_are_malformed() {
    let huge = format!(
        r#"{{"data":[{{"id":"gpt-4.1","pad":"{}"}}]}}"#,
        "x".repeat(MAX_BODY)
    );
    let many = format!(
        r#"{{"data":[{}]}}"#,
        (0..=MAX_MODELS)
            .map(|i| format!(r#"{{"id":"m{i}"}}"#))
            .collect::<Vec<_>>()
            .join(",")
    );
    for body in [
        huge.as_bytes(),
        many.as_bytes(),
        b"not json",
        b"[]",
        br#"{"data":{"id":"gpt-4.1"}}"#,
        br#"{"data":[{"id":7}]}"#,
        br#"{"data":[{"name":"gpt-4.1"}]}"#,
        br#"{"models":[]}"#,
    ] {
        let server = fake(vec![http("200 OK", body)]);
        assert_eq!(
            run(&server).await,
            Err(PreflightFailure::Malformed),
            "{}",
            String::from_utf8_lossy(&body[..body.len().min(60)])
        );
    }
    // A body larger than the cap without a length header is cut off too.
    let mut chunked = b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n".to_vec();
    chunked.extend_from_slice(huge.as_bytes());
    let server = fake(vec![Some(chunked)]);
    assert_eq!(run(&server).await, Err(PreflightFailure::Malformed));
}

#[test]
fn odd_ids_and_names_are_dropped_not_offered() {
    let chat = r#""capabilities":{"type":"chat","supports":{"tool_calls":true}}"#;
    let body = format!(
        r#"{{"data":[{{"id":"gpt-4.1\n",{chat}}},{{"id":"<script>",{chat}}},{{"id":"auto",{chat}}},{{"id":"",{chat}}},
        {{"id":"-lead",{chat}}},{{"id":"a b",{chat}}},{{"id":"gpt\u202e1",{chat}}},{{"id":"gpt-4.1","capabilities":7}},
        {{"id":"gpt-4.1",{chat},"name":"bad\nname"}},{{"id":"ok_1.2-3",{chat},"name":7}},{{"id":"x","capabilities":{{"supports":"yes"}}}}]}}"#
    );
    let catalog = parse(body.as_bytes()).unwrap();
    // The first gpt-4.1 entry wins and has no tool support, so it is not listed.
    assert_eq!(
        catalog
            .models()
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["ok_1.2-3"]
    );
    assert!(catalog.models().iter().all(|m| m.name.is_none()));
    assert!(!valid_id(&"m".repeat(101)));
}

#[test]
fn token_types_are_checked_without_network() {
    for token in [b"gho_abc".as_slice(), b"ghu_abc", b"github_pat_11AB_cd"] {
        assert!(token_type_ok(token));
    }
    for token in [
        b"ghp_classic".as_slice(),
        b"gho_",
        b"gho_has space",
        b"sk-ant-x",
        b"",
        b"synthetic-model-only",
    ] {
        assert!(!token_type_ok(token));
    }
    let preflight = Preflight::fixed(Ok(Catalog::offering(&["gpt-4.1"])));
    assert_eq!(
        preflight.catalog(b"ghp_classic"),
        Err(PreflightFailure::TokenType)
    );
}

#[test]
fn preselection_uses_the_previous_choice_then_the_list_then_the_first_model() {
    let catalog = Catalog::offering(&["zeta", "gpt-4.1", "gpt-5.6-luna", "gpt-6.1-terra"]);
    assert_eq!(preselect(Some("zeta"), &catalog).as_deref(), Some("zeta"));
    assert_eq!(
        preselect(Some("gone"), &catalog).as_deref(),
        Some("gpt-6.1-terra")
    );
    assert_eq!(
        preselect(None, &Catalog::offering(&["gpt-4.1", "gpt-5.6-luna"])).as_deref(),
        Some("gpt-5.6-luna")
    );
    assert_eq!(
        preselect(None, &Catalog::offering(&["zeta", "eta"])).as_deref(),
        Some("zeta")
    );
    assert_eq!(preselect(None, &Catalog::offering(&[])), None);
}

#[test]
fn a_chosen_model_must_still_be_offered() {
    let catalog = Catalog::offering(&["gpt-4.1"]);
    assert_eq!(require("gpt-4.1", &catalog), Ok(()));
    let ids: Vec<String> = (0..20).map(|i| format!("other-{i}")).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    match require("gpt-5.6-terra", &Catalog::offering(&refs)) {
        Err(CheckError::NotOffered { wanted, catalog }) => {
            assert_eq!(wanted, "gpt-5.6-terra");
            assert_eq!(catalog.models().len(), 20);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(Catalog::offering(&refs).models().len(), 20);
}

#[test]
fn successful_answers_are_reused_for_the_same_sign_in_while_fresh() {
    let preflight = Preflight::fixed(Ok(Catalog::offering(&["gpt-4.1"])));
    let now = Instant::now();
    assert!(preflight.catalog_at(TOKEN, now).unwrap().usable("gpt-4.1"));
    let cached = |token: &[u8], at: Instant| {
        preflight.cache.lock().unwrap().as_ref().is_some_and(|c| {
            c.digest == <[u8; 32]>::from(Sha256::digest(token))
                && at.saturating_duration_since(c.at) < FRESH_FOR
        })
    };
    assert!(cached(TOKEN, now + Duration::from_secs(60)));
    assert!(!cached(b"gho_another", now));
    assert!(!cached(TOKEN, now + FRESH_FOR));
    preflight.forget();
    assert!(!cached(TOKEN, now));
    // A failure is never remembered: Check again asks again.
    let failing = Preflight::fixed(Err(PreflightFailure::AccessDenied));
    assert_eq!(failing.catalog(TOKEN), Err(PreflightFailure::AccessDenied));
    assert!(failing.cache.lock().unwrap().is_none());
}

#[test]
fn debug_output_never_shows_a_token() {
    let preflight = Preflight::fixed(Ok(Catalog::offering(&["gpt-4.1"])));
    preflight.catalog(TOKEN).unwrap();
    let shown = format!(
        "{preflight:?} {:?} {:?}",
        preflight.require(TOKEN, "gpt-4.1"),
        Preflight::fixed(Err(PreflightFailure::SignInRejected)).catalog(TOKEN)
    );
    assert!(!shown.contains("synthetic0preflight"), "{shown}");
}

#[test]
fn preferred_models_pass_the_id_pattern() {
    assert!(PREFERRED_MODELS.iter().all(|id| valid_id(id)));
}

/// Live check of the real reply shape. Ignored by default; run by hand with
/// `BRIDGE_COPILOT_TOKEN_FILE` naming a file that holds a sign-in token.
/// Prints only the status class and the ids.
#[test]
#[ignore]
fn live_reply_parses() {
    let path = std::env::var("BRIDGE_COPILOT_TOKEN_FILE").unwrap();
    let token = std::fs::read(path).unwrap();
    let token = token.trim_ascii();
    match fetch_blocking(token) {
        Ok(catalog) => {
            println!(
                "ok; usable ids: {:?}",
                catalog.models().iter().map(|m| &m.id).collect::<Vec<_>>()
            );
            println!("preselected: {:?}", preselect(None, &catalog));
        }
        Err(failure) => println!("failure: {failure:?}"),
    }
}
