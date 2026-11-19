//! Read-only Overview Sessions under their own identity. Replies are bounded projections:
//! raw Sessions, and transcripts in a list, never leave the guest.
use super::{failed, Orka, REQUEST_BUDGET, SESSIONS_IDENTITY};
use bridge_runtime_protocol::*;
use serde_json::{json, Map, Value};
use std::{io, time::Instant};

/// Room under MAX_FILE for the reply envelope around a projected body.
const SESSION_REPLY: usize = MAX_FILE - 4096;
/// Longest short field (role, time, name, type) a projection keeps.
const SHORT_TEXT: usize = 512;
/// Tool calls named per message.
const TOOL_CALLS: usize = 64;
/// Stands in for a Session ID too long to keep. It fails `valid_session_name`, so the host
/// counts the Session as one it can't open rather than failing the whole list.
const LONG_NAME: &str = "-too-long";
const LIST_KEYS: [&str; 7] = [
    "name",
    "namespace",
    "sessionType",
    "messageCount",
    "createdAt",
    "updatedAt",
    "activeTask",
];

fn short(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| s.len() <= SHORT_TEXT)
}
/// Scalars only, so a listed Session cannot carry a nested payload.
fn scalar(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => true,
        Value::String(s) => s.len() <= SHORT_TEXT,
        _ => false,
    }
}
pub(super) fn list_body(page: &Value) -> io::Result<Value> {
    let items = page["items"].as_array().ok_or_else(failed)?;
    if items.len() > SESSION_PAGE {
        return Err(failed());
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let item = item.as_object().ok_or_else(failed)?;
        let mut kept = Map::new();
        for key in LIST_KEYS {
            if let Some(value) = item.get(key).filter(|v| scalar(v)) {
                kept.insert(key.into(), value.clone());
            }
        }
        if item
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| n.len() > SHORT_TEXT)
        {
            kept.insert("name".into(), json!(LONG_NAME));
        }
        out.push(Value::Object(kept));
    }
    let next = match page.get("metadata").and_then(|m| m.get("continue")) {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(s)) if s.is_empty() => Value::Null,
        Some(Value::String(s)) if valid_session_cursor(s) => json!(s),
        // Orka's cursor carries the page's last Session ID, so a long ID makes a cursor too long
        // to pass on. The rows are still good: they are returned, marked cut, with no next page.
        Some(Value::String(s)) if s.len() > MAX_SESSION_CURSOR => {
            return Ok(json!({"items":out,"metadata":{"continue":null,"cut":true}}));
        }
        _ => return Err(failed()),
    };
    Ok(json!({"items":out,"metadata":{"continue":next}}))
}
/// One JSONL message reduced to role, bounded content, tool names and time; None when the
/// line is not a JSON object.
fn message(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    let value = value.as_object()?;
    let mut kept = Map::new();
    if let Some(role) = value.get("role").and_then(short) {
        kept.insert("role".into(), json!(role));
    }
    let mut cut = false;
    if let Some(content) = value.get("content").and_then(Value::as_str) {
        let mut end = content.len().min(SESSION_MESSAGE_TEXT);
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        cut = end < content.len();
        kept.insert("content".into(), json!(&content[..end]));
    } else if value.get("content").is_some_and(|c| !c.is_null()) {
        // Content Bridge does not show, such as structured parts, is marked as cut.
        cut = true;
    }
    if let Some(calls) = value.get("toolCalls").and_then(Value::as_array) {
        cut |= calls.len() > TOOL_CALLS;
        let names: Vec<Value> = calls
            .iter()
            .take(TOOL_CALLS)
            .filter_map(|c| {
                c.get("name").and_then(short).or_else(|| {
                    c.get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(short)
                })
            })
            .map(|name| json!({ "name": name }))
            .collect();
        kept.insert("toolCalls".into(), Value::Array(names));
    }
    if cut {
        kept.insert("cut".into(), json!(true));
    }
    if let Some(ts) = value.get("ts").and_then(short) {
        kept.insert("ts".into(), json!(ts));
    }
    serde_json::to_string(&Value::Object(kept)).ok()
}
pub(super) fn read_body(session: &Value, name: &str) -> io::Result<Value> {
    let namespace = session["namespace"].as_str().ok_or_else(failed)?;
    if session["name"].as_str() != Some(name) || namespace.len() > 253 {
        return Err(failed());
    }
    let raw = match &session["transcript"] {
        Value::Null => "",
        Value::String(s) => s.as_str(),
        _ => return Err(failed()),
    };
    // Newest first: only the window that can be shown is parsed.
    let mut lines = Vec::new();
    let mut cut = false;
    for line in raw.lines().rev().filter(|l| !l.trim().is_empty()) {
        if lines.len() == SESSION_MESSAGES {
            cut = true;
            break;
        }
        match message(line) {
            Some(kept) => lines.push(kept),
            None => cut = true,
        }
    }
    lines.reverse();
    // A line costs its escaped length; its two quotes stand in for the escaped newline.
    let mut total = serde_json::to_vec(
        &json!({"name":name,"namespace":namespace,"transcript":"","transcriptCut":true}),
    )?
    .len();
    let sizes: Vec<usize> = lines
        .iter()
        .map(|l| serde_json::to_string(l).map(|s| s.len()))
        .collect::<Result<_, _>>()?;
    total += sizes.iter().sum::<usize>();
    let mut dropped = 0;
    while total > SESSION_REPLY && dropped < lines.len() {
        total -= sizes[dropped];
        dropped += 1;
    }
    if dropped > 0 {
        lines.drain(..dropped);
        cut = true;
    }
    Ok(json!({"name":name,"namespace":namespace,"transcript":lines.join("\n"),"transcriptCut":cut}))
}
impl Orka {
    /// One page of Sessions after Orka's cursor, without transcripts.
    pub fn session_list(&mut self, after: Option<&str>) -> io::Result<EngineResult> {
        self.session_list_until(after, Instant::now() + REQUEST_BUDGET)
    }
    fn session_list_until(
        &mut self,
        after: Option<&str>,
        deadline: Instant,
    ) -> io::Result<EngineResult> {
        if self.identity != SESSIONS_IDENTITY || after.is_some_and(|a| !valid_session_cursor(a)) {
            return Err(failed());
        }
        let limit = SESSION_PAGE.to_string();
        let mut query = vec![("limit", limit.as_str())];
        if let Some(after) = after {
            query.push(("continue", after));
        }
        self.session_get("sessions", &query, deadline, list_body)
    }
    /// One Session's latest transcript lines.
    pub fn session_read(&mut self, name: &str) -> io::Result<EngineResult> {
        if self.identity != SESSIONS_IDENTITY || !valid_session_name(name) {
            return Err(failed());
        }
        let path = format!("sessions/{name}");
        self.session_get(&path, &[], Instant::now() + REQUEST_BUDGET, |v| {
            read_body(v, name)
        })
    }
    fn session_get(
        &mut self,
        path: &str,
        query: &[(&str, &str)],
        deadline: Instant,
        project: impl FnOnce(&Value) -> io::Result<Value>,
    ) -> io::Result<EngineResult> {
        let (status, raw) = self.send(
            reqwest::Method::GET,
            path,
            None,
            None,
            query,
            MAX_SESSION_READ,
            deadline,
        )?;
        let body = if status == 200 {
            project(&serde_json::from_slice(&raw).map_err(|_| failed())?)?
        } else {
            Value::Null
        };
        let stdout = serde_json::to_vec(&OrkaReply { status, body })?;
        if stdout.len() > MAX_FILE || self.echoes(&stdout) {
            return Err(failed());
        }
        Ok(EngineResult {
            exit: 0,
            stdout,
            has_diagnostics: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{list_body, message, LONG_NAME, SHORT_TEXT};
    use bridge_runtime_protocol::valid_session_name;
    use serde_json::{json, Value};

    #[test]
    fn a_cursor_too_long_to_pass_on_keeps_the_page_marked_cut() {
        let page = json!({"items":[{"name":"kept","namespace":"orka-system"}],
            "metadata":{"continue":"a".repeat(bridge_runtime_protocol::MAX_SESSION_CURSOR + 1)}});
        let body = list_body(&page).unwrap();
        assert_eq!(body["items"][0]["name"], "kept");
        assert_eq!(body["metadata"], json!({"continue":null,"cut":true}));
        // A cursor with characters Orka never sends still fails the page.
        assert!(list_body(&json!({"items":[],"metadata":{"continue":"a b"}})).is_err());
    }

    #[test]
    fn an_id_too_long_to_keep_is_listed_under_a_name_the_host_cannot_request() {
        let long = "a".repeat(SHORT_TEXT + 1);
        let page = json!({"items":[{"name":long,"namespace":"orka-system"},
            {"name":"kept","namespace":"orka-system"}]});
        let body = list_body(&page).unwrap();
        assert_eq!(body["items"][0]["name"], LONG_NAME);
        assert_eq!(body["items"][1]["name"], "kept");
        assert!(!valid_session_name(LONG_NAME));
    }

    #[test]
    fn content_bridge_does_not_show_marks_the_message_cut() {
        let kept = |line: Value| -> Value {
            serde_json::from_str(&message(&line.to_string()).unwrap()).unwrap()
        };
        let parts = kept(json!({"role":"user","content":[{"type":"image"}]}));
        assert_eq!(parts["cut"], true);
        assert!(parts.get("content").is_none());
        assert_ne!(kept(json!({"role":"user","content":null}))["cut"], true);
        assert_ne!(kept(json!({"role":"user","content":"hi"}))["cut"], true);
    }
}
