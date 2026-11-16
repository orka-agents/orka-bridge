use super::*;
use serde_json::json;

fn entry(status: Value) -> Result<Entry, ClusterError> {
    task(
        &json!({"metadata":{"name":"run","namespace":"agents","uid":"u","creationTimestamp":"2026-01-01T00:00:00Z"},
            "spec":{"agentRef":{"name":"reader"}},"status":status}),
        "agents",
    )
}

#[test]
fn unknown_or_missing_status_degrades_without_hiding_the_task() {
    let missing = entry(json!(null)).unwrap();
    assert_eq!(missing.phase, Phase::Pending);
    assert_eq!((missing.started_at, missing.completed_at), (None, None));
    let unknown =
        entry(json!({"phase":"Paused","startTime":"not a time","completionTime":7})).unwrap();
    assert_eq!(unknown.phase, Phase::Unknown);
    assert_eq!((unknown.started_at, unknown.completed_at), (None, None));
    assert!(!unknown.test);
}

#[test]
fn failure_messages_are_bounded_on_a_character_boundary() {
    let long = format!("{}é tail", "x".repeat(MESSAGE - 1));
    let failed = entry(json!({"phase":"Failed","message":long})).unwrap();
    assert_eq!(failed.message.unwrap(), "x".repeat(MESSAGE - 1));
    let blank = entry(json!({"phase":"Cancelled","message":"   "})).unwrap();
    assert_eq!(blank.message, None);
    let succeeded = entry(json!({"phase":"Succeeded","message":"done"})).unwrap();
    assert_eq!(succeeded.message, None);
}

#[test]
fn newest_uses_the_latest_recorded_time_then_name() {
    let at = |task: &str, created: u64, completed: Option<u64>| Entry {
        task: task.into(),
        uid: task.into(),
        agent: None,
        agent_uid: None,
        phase: Phase::Succeeded,
        created_at: created,
        started_at: None,
        completed_at: completed,
        message: None,
        test: false,
    };
    let entries = (0..SHOWN + 5)
        .map(|i| at(&format!("filler-{i:03}"), 1, None))
        .chain([at("b", 5, None), at("a", 5, None), at("late", 1, Some(9))])
        .collect();
    let tasks = newest(entries, true);
    let names: Vec<_> = tasks
        .entries
        .iter()
        .take(3)
        .map(|e| e.task.as_str())
        .collect();
    assert_eq!(names, ["late", "a", "b"]);
    assert_eq!(tasks.entries.len(), SHOWN);
}

#[test]
fn only_kubernetes_object_names_reach_an_event_path() {
    for name in ["run", "bridge-task-0a", "a.b-c", &"a".repeat(63)] {
        assert!(valid_task(name), "{name}");
    }
    for name in [
        "",
        "-run",
        "run-",
        "Run",
        "a/b",
        "a..b",
        "a?b",
        "a%2fb",
        &"a".repeat(64),
    ] {
        assert!(!valid_task(name), "{name}");
    }
}

#[test]
fn events_the_type_filter_missed_are_skipped_but_advance_the_cursor() {
    let event = |seq: u64, kind: &str| {
        json!({"namespace":"agents","streamID":"run","seq":seq,"type":kind,"taskName":"run",
            "toolName":"shell","createdAt":"2026-01-01T00:00:00Z"})
    };
    let page = json!({"namespace":"agents","streamType":"task","streamID":"run","afterSeq":0,
        "latestSeq":9,"events":[event(1, "ToolCallCompleted"), event(2, "TaskStarted"),
        event(3, "ToolCallFailed"), event(4, "MessageDelta")]});
    let read = tool_page(&page, "agents", "run", 0, 200).unwrap();
    let failed: Vec<_> = read.calls.iter().map(|c| c.failed).collect();
    assert_eq!(failed, [false, true]);
    assert_eq!(read.next, Some(4));
    assert!(!read.dropped);
}

#[test]
fn event_streams_are_built_only_from_object_names() {
    use crate::clusters::{EventKind, EventStream};
    let stream = EventStream::task_tool_calls("run-1").unwrap();
    assert_eq!(stream.kind(), EventKind::TaskToolCalls);
    assert_eq!(stream.name(), "run-1");
    assert_eq!(stream.types(), ["ToolCallCompleted", "ToolCallFailed"]);
    assert_eq!(stream.path(), "tasks/run-1/events");
    for name in ["", "a/b", "../x", "a?b", "Run"] {
        assert!(EventStream::task_tool_calls(name).is_none(), "{name}");
    }
}
