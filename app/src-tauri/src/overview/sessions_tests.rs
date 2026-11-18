use super::*;
use serde_json::json;

fn listed(name: &str, updated: &str) -> Value {
    json!({"id":name,"name":name,"namespace":"agents","sessionType":"chat","messageCount":4,
        "inputTokens":10,"outputTokens":20,"activeTask":"","createdAt":"2026-01-01T00:00:00Z","updatedAt":updated})
}

#[test]
fn listed_sessions_keep_identity_times_and_running_task_only() {
    let mut value = listed("bridge-chat-1", "2026-01-02T00:00:00Z");
    value["activeTask"] = json!("run-1");
    let read = session(&value, "agents").unwrap().unwrap();
    assert_eq!(
        read,
        Session {
            name: "bridge-chat-1".into(),
            session_type: Some("chat".into()),
            message_count: 4,
            created_at: 1767225600000,
            updated_at: 1767312000000,
            active_task: Some("run-1".into()),
            agent: None,
        }
    );
    // Token counts are not carried.
    assert!(!serde_json::to_string(&read).unwrap().contains("Tokens"));

    // An empty or malformed running Task name is absent; an odd type is dropped, not trusted.
    value["activeTask"] = json!("../x");
    value["sessionType"] = json!("chat type");
    let read = session(&value, "agents").unwrap().unwrap();
    assert_eq!((read.active_task, read.session_type), (None, None));
}

#[test]
fn gateway_and_malformed_sessions_are_skipped_or_rejected() {
    let mut gateway = listed("g", "2026-01-02T00:00:00Z");
    gateway["sessionType"] = json!("gateway");
    assert_eq!(session(&gateway, "agents"), Ok(None));
    // A gateway Session's channel-derived name, or another namespace, does not fail the list.
    gateway["name"] = json!("telegram:12345@bot");
    gateway["namespace"] = json!("other");
    assert_eq!(session(&gateway, "agents"), Ok(None));
    for (field, bad) in [
        ("namespace", json!("other")),
        ("name", json!("a/b")),
        ("name", json!("")),
        ("name", json!(".hidden")),
        ("messageCount", json!(-1)),
        ("createdAt", json!("yesterday")),
        ("updatedAt", Value::Null),
    ] {
        let mut value = listed("s", "2026-01-02T00:00:00Z");
        value[field] = bad;
        assert_eq!(
            session(&value, "agents"),
            Err(ClusterError::Unavailable),
            "{field}"
        );
    }
}

#[test]
fn newest_sorts_by_update_then_name_and_marks_overflow_partial() {
    let entries: Vec<_> = (0..60)
        .map(|i| {
            session(
                &listed(
                    &format!("s-{i:02}"),
                    &format!("2026-01-01T00:{:02}:00Z", i % 30),
                ),
                "agents",
            )
            .unwrap()
            .unwrap()
        })
        .collect();
    let list = newest(entries.clone(), true);
    assert!(!list.complete);
    assert_eq!(list.entries.len(), SHOWN);
    assert_eq!(list.entries[0].name, "s-29");
    assert_eq!(list.entries[1].name, "s-59");
    let list = newest(entries[..3].to_vec(), true);
    assert!(list.complete);
    assert_eq!(
        list.entries
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["s-02", "s-01", "s-00"]
    );
}

#[test]
fn active_task_requires_the_same_task_and_reports_a_finished_one_as_ended() {
    let mut task = json!({"metadata":{"name":"run-1","namespace":"agents"},"spec":{"agentRef":{"name":"reader"}},"status":{"phase":"Running"}});
    assert_eq!(
        active_task(&task, "agents", "run-1"),
        Some(ActiveTask::Running(Some("reader".into())))
    );
    assert_eq!(active_task(&task, "agents", "run-2"), None);
    assert_eq!(active_task(&task, "other", "run-1"), None);
    for phase in ["Succeeded", "Failed", "Cancelled"] {
        task["status"]["phase"] = json!(phase);
        assert_eq!(
            active_task(&task, "agents", "run-1"),
            Some(ActiveTask::Ended)
        );
    }
    task["status"] = Value::Null;
    task["spec"] = json!({});
    assert_eq!(
        active_task(&task, "agents", "run-1"),
        Some(ActiveTask::Running(None))
    );
}

#[test]
fn transcript_keeps_roles_text_tools_and_the_latest_messages() {
    let lines = [
        json!({"id":"1","role":"user","content":"hello","ts":"2026-01-01T00:00:00Z"}),
        json!({"id":"2","role":"assistant","content":"","toolCalls":[{"id":"c","name":"web_fetch","arguments":{"url":"x"}},{"function":{"name":"shell"}}],"ts":"2026-01-01T00:00:01Z"}),
        json!({"id":"3","role":"tool","content":"page","toolCallID":"c"}),
    ]
    .map(|l| l.to_string())
    .join("\n");
    let reply = json!({"name":"s","namespace":"agents","transcript":lines});
    let read = transcript(&reply, "agents", "s").unwrap();
    assert!(read.complete);
    assert_eq!(read.messages.len(), 3);
    assert_eq!(read.messages[0].text, "hello");
    assert_eq!(read.messages[0].at, Some(1767225600000));
    assert_eq!(read.messages[1].tools, ["web_fetch", "shell"]);
    assert_eq!(read.messages[2].at, None);
    // Tool arguments are never carried.
    assert!(!format!("{read:?}").contains("url"));

    // Unparseable lines are left out and make the transcript partial.
    let reply = json!({"name":"s","namespace":"agents","transcript":"not json\n{\"content\":\"no role\"}\n{\"role\":\"user\",\"content\":\"ok\"}"});
    let read = transcript(&reply, "agents", "s").unwrap();
    assert!(!read.complete);
    assert_eq!(read.messages.len(), 1);

    // Long text is cut on a character boundary; only the latest messages are kept.
    let long = "é".repeat(MESSAGE_TEXT);
    let many: Vec<_> = (0..MAX_MESSAGES + 5)
        .map(|i| json!({"role":"user","content":if i == MAX_MESSAGES + 4 { long.clone() } else { i.to_string() }}).to_string())
        .collect();
    let reply = json!({"name":"s","namespace":"agents","transcript":many.join("\n")});
    let read = transcript(&reply, "agents", "s").unwrap();
    assert!(!read.complete);
    assert_eq!(read.messages.len(), MAX_MESSAGES);
    assert_eq!(read.messages[0].text, "5");
    let last = read.messages.last().unwrap();
    assert!(last.cut && last.text.len() <= MESSAGE_TEXT && last.text.len() > MESSAGE_TEXT - 2);

    // An empty Session has no transcript text.
    let empty = json!({"name":"s","namespace":"agents","transcript":""});
    assert_eq!(transcript(&empty, "agents", "s").unwrap().messages, []);
}

#[test]
fn transcript_rejects_another_session_or_namespace() {
    for reply in [
        json!({"name":"t","namespace":"agents","transcript":""}),
        json!({"name":"s","namespace":"other","transcript":""}),
        json!({"name":"s","namespace":"agents","transcript":[]}),
    ] {
        assert_eq!(
            transcript(&reply, "agents", "s"),
            Err(ClusterError::Unavailable)
        );
    }
}

#[test]
fn session_names_never_carry_path_syntax() {
    for good in ["bridge-chat-0123", "a", "session_1.v2", "ABC"] {
        assert!(valid_session(good), "{good}");
    }
    for bad in [
        "",
        "a/b",
        "..",
        ".a",
        "a?b",
        "a b",
        "a%2Fb",
        &"a".repeat(254),
    ] {
        assert!(!valid_session(bad), "{bad}");
    }
}
