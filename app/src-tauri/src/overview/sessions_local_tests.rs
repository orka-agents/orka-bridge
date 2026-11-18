use super::*;
use serde_json::json;
use std::{collections::VecDeque, sync::Mutex};

type Answer = Result<(Uuid, io::Result<OrkaReply>), RuntimeError>;
#[derive(Default)]
struct Fake {
    answers: Mutex<VecDeque<Answer>>,
    actions: Mutex<Vec<EngineActionWire>>,
    stopped: bool,
    last: Mutex<Option<Uuid>>,
}
impl Fake {
    fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            ..Self::default()
        }
    }
    fn actions(&self) -> Vec<EngineActionWire> {
        self.actions.lock().unwrap().clone()
    }
}
impl LocalSessions for Fake {
    fn ready(&self) -> bool {
        !self.stopped
    }
    fn last_boot(&self) -> Option<Uuid> {
        *self.last.lock().unwrap()
    }
    fn sessions_request(&self, action: EngineActionWire) -> impl Future<Output = Answer> + Send {
        self.actions.lock().unwrap().push(action);
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("no answer left");
        if let Ok((boot, _)) = &answer {
            *self.last.lock().unwrap() = Some(*boot);
        }
        async move { answer }
    }
}
const BOOT: Uuid = Uuid::from_u128(0x0102_0304_0506_0708_0000_0000_0000_0001);
const SHOWN_BOOT: u64 = 0x0102_0304_0506_0708 >> 11;
fn ok(body: Value) -> Answer {
    Ok((BOOT, Ok(OrkaReply { status: 200, body })))
}
fn listed(name: &str, updated: &str) -> Value {
    json!({"name":name,"namespace":ORKA_NAMESPACE,"sessionType":"chat","messageCount":2,
        "createdAt":"2026-01-01T00:00:00Z","updatedAt":updated})
}
fn page(items: Vec<Value>, next: Option<&str>) -> Value {
    json!({"items":items,"metadata":{"continue":next}})
}

#[tokio::test]
async fn pages_on_one_boot_list_newest_updated_first() {
    let vm = Fake::new(vec![
        ok(page(vec![listed("a", "2026-01-02T00:00:00Z")], Some("c1"))),
        ok(page(vec![listed("b", "2026-01-03T00:00:00Z")], None)),
    ]);
    let view = read(&vm).await;
    assert_eq!(view.key, LOCAL);
    assert_eq!(view.state, "complete");
    assert_eq!(view.connection, Some(SHOWN_BOOT));
    assert!(view.observed_at.is_some());
    let names: Vec<_> = view.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["b", "a"]);
    assert_eq!(
        vm.actions(),
        [
            EngineActionWire::SessionList { after: None },
            EngineActionWire::SessionList {
                after: Some("c1".into())
            },
        ]
    );
}

#[tokio::test]
async fn a_vm_image_without_sessions_reads_is_outdated() {
    let refused = || Ok((BOOT, Err(io::ErrorKind::Unsupported.into())));
    let view = read(&Fake::new(vec![refused()])).await;
    assert_eq!((view.state, view.entries.len()), ("outdated", 0));
    assert_eq!(view.connection, Some(SHOWN_BOOT));
    let shown = read_transcript(&Fake::new(vec![refused()]), "a").await;
    assert_eq!(shown.state, "outdated");
}

#[tokio::test]
async fn refusals_and_a_stopped_vm_map_to_place_states() {
    let status = |status| {
        Ok((
            BOOT,
            Ok(OrkaReply {
                status,
                body: Value::Null,
            }),
        ))
    };
    for (answer, state) in [
        (status(403), "denied"),
        (status(401), "denied"),
        (status(500), "unavailable"),
        (
            Ok((BOOT, Err(io::ErrorKind::InvalidData.into()))),
            "unavailable",
        ),
        (Err(RuntimeError::Unavailable), "disconnected"),
        (Err(RuntimeError::Closed), "disconnected"),
    ] {
        let view = read(&Fake::new(vec![answer])).await;
        assert_eq!(view.state, state);
        assert!(view.entries.is_empty() && view.observed_at.is_none());
    }
}

#[tokio::test]
async fn a_busy_or_stopped_workspace_is_not_retried_and_keeps_the_last_boot() {
    let vm = Fake::new(vec![
        ok(page(vec![listed("a", "2026-01-02T00:00:00Z")], None)),
        Err(RuntimeError::Busy),
    ]);
    assert_eq!(read(&vm).await.state, "complete");
    let view = read(&vm).await;
    assert_eq!(
        (view.state, view.connection),
        ("unavailable", Some(SHOWN_BOOT))
    );
    assert_eq!(vm.actions().len(), 2);
    // A workspace that is not ready is never asked.
    let stopped = Fake {
        stopped: true,
        last: Mutex::new(Some(BOOT)),
        ..Fake::default()
    };
    let view = read(&stopped).await;
    assert_eq!(
        (view.state, view.connection),
        ("disconnected", Some(SHOWN_BOOT))
    );
    assert_eq!(read_transcript(&stopped, "a").await.state, "disconnected");
    assert!(stopped.actions().is_empty());
    // Before any answer there is no boot to keep.
    let view = read(&Fake::new(vec![Err(RuntimeError::Busy)])).await;
    assert_eq!((view.state, view.connection), ("unavailable", None));
}

#[test]
fn a_boot_connection_fits_a_javascript_safe_integer() {
    assert!(connection(Uuid::max()) < 1 << 53);
    assert_ne!(connection(BOOT), connection(Uuid::from_u128(2)));
}

#[tokio::test]
async fn a_new_boot_between_pages_or_a_bad_cursor_fails_the_read() {
    let other = Uuid::from_u128(2);
    let vm = Fake::new(vec![
        ok(page(vec![listed("a", "2026-01-02T00:00:00Z")], Some("c1"))),
        Ok((
            other,
            Ok(OrkaReply {
                status: 200,
                body: page(vec![], None),
            }),
        )),
    ]);
    assert_eq!(read(&vm).await.state, "unavailable");
    for next in [json!("bad cursor"), json!(7)] {
        let body = json!({"items":[],"metadata":{"continue":next}});
        assert_eq!(read(&Fake::new(vec![ok(body)])).await.state, "unavailable");
    }
    // A repeated cursor cannot loop.
    let vm = Fake::new(vec![
        ok(page(vec![listed("a", "2026-01-02T00:00:00Z")], Some("c1"))),
        ok(page(vec![listed("b", "2026-01-02T00:00:00Z")], Some("c1"))),
    ]);
    assert_eq!(read(&vm).await.state, "unavailable");
}

#[tokio::test]
async fn more_sessions_than_one_read_takes_is_partial() {
    let pages = (0..MAX_SESSIONS.div_ceil(SESSION_PAGE))
        .map(|p| {
            let items = (0..SESSION_PAGE)
                .map(|i| listed(&format!("s{p}-{i}"), "2026-01-02T00:00:00Z"))
                .collect();
            ok(page(items, Some(&format!("c{p}"))))
        })
        .collect();
    let view = read(&Fake::new(pages)).await;
    assert_eq!(view.state, "partial");
    assert_eq!(view.entries.len(), super::super::SHOWN);
}

#[tokio::test]
async fn a_projected_transcript_keeps_the_guest_cut_marks() {
    let line = |text: &str, cut: bool| {
        json!({"role":"assistant","content":text,"cut":cut,"toolCalls":[{"name":"search"}],
            "ts":"2026-01-01T00:00:00Z"})
        .to_string()
    };
    let body = json!({"name":"a","namespace":ORKA_NAMESPACE,
        "transcript":format!("{}\n{}", line("short", false), line("long", true)),"transcriptCut":true});
    let vm = Fake::new(vec![ok(body)]);
    let shown = read_transcript(&vm, "a").await;
    assert_eq!(shown.state, "partial");
    let cuts: Vec<_> = shown.messages.iter().map(|m| m.cut).collect();
    assert_eq!(cuts, [false, true]);
    assert_eq!(shown.messages[0].tools, ["search"]);
    assert_eq!(
        vm.actions(),
        [EngineActionWire::SessionRead { name: "a".into() }]
    );
    // Another Session's reply is not shown.
    let wrong = json!({"name":"b","namespace":ORKA_NAMESPACE,"transcript":""});
    assert_eq!(
        read_transcript(&Fake::new(vec![ok(wrong)]), "a")
            .await
            .state,
        "unavailable"
    );
}

#[tokio::test]
async fn an_invalid_name_is_never_sent_to_the_vm() {
    let vm = Fake::new(vec![]);
    assert_eq!(read_transcript(&vm, "../x").await.state, "unavailable");
    assert!(vm.actions().is_empty());
}
