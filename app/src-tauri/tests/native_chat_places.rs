//! Conversations are per place: switching between this computer and clusters never shows,
//! continues or confirms another place's turns, and nothing is deleted on a switch.
use bridge_engine::{
    agent_folder::DocumentId,
    chat::{Channel, ChatError, ChatOwner, ChatSnapshot, ChatState, MAX_CONVERSATIONS},
    chat_cluster::{ClusterChannel, ClusterOpen},
};
use bridge_runtime_protocol::{ChatMethod, ChatRequest};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
// Shared fixtures; each test file uses part of them, and the Tauri-free harness loads them once per file.
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "support/cluster_chat_fixture.rs"]
mod cluster_fixture;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "support/chat_fixture.rs"]
mod local_fixture;
use cluster_fixture::{serve, Orka, NAMESPACE, PROVIDER};

fn cluster(key: &str, provider: &str, open: Arc<dyn ClusterOpen>) -> Channel {
    Channel::Cluster(Arc::new(
        ClusterChannel::new(key.into(), NAMESPACE.into(), provider.into(), open).unwrap(),
    ))
}
async fn settled(owner: &ChatOwner, channel: &Channel, id: DocumentId) -> ChatSnapshot {
    until(owner, channel, id, |s| !s.busy).await
}
async fn until(
    owner: &ChatOwner,
    channel: &Channel,
    id: DocumentId,
    what: impl Fn(&ChatSnapshot) -> bool,
) -> ChatSnapshot {
    for _ in 0..2000 {
        let s = owner.status_on(id, channel);
        if what(&s) {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!(
        "never reached the expected state: {:?}",
        owner.status_on(id, channel)
    );
}
fn turns(s: &ChatSnapshot) -> Vec<String> {
    s.versions
        .iter()
        .flat_map(|v| v.turns.iter().map(|t| t.message.clone()))
        .collect()
}
const NAV: DocumentId = bridge_engine::navigator::CONVERSATION;
fn sessions(o: &Orka) -> Vec<String> {
    o.bodies
        .iter()
        .filter_map(|b| b["spec"]["sessionRef"]["name"].as_str().map(str::to_owned))
        .collect()
}
async fn say(owner: &Arc<ChatOwner>, channel: &Channel, message: &str) -> ChatSnapshot {
    owner
        .send_navigator_on(channel.clone(), "- web-reader".into(), message.into())
        .unwrap();
    let s = settled(owner, channel, NAV).await;
    assert_eq!(s.state, ChatState::Ready, "{s:?}");
    s
}

// Alternating two clusters: each keeps its own turns, Agent and Session.
#[tokio::test]
async fn two_places_alternate_without_sharing_turns_or_sessions() {
    let (a_orka, a_open) = serve().await;
    let (b_orka, b_open) = serve().await;
    let (a, b) = (
        cluster("a", PROVIDER, a_open),
        cluster("b", PROVIDER, b_open),
    );
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    say(&owner, &a, "First on a").await;
    let b_view = say(&owner, &b, "First on b").await;
    assert_eq!(turns(&b_view), ["First on b"]);
    let a_view = say(&owner, &a, "Second on a").await;
    assert_eq!(turns(&a_view), ["First on a", "Second on a"]);
    assert_eq!(
        a_view.versions.len(),
        1,
        "the switch did not start a new conversation"
    );
    assert_eq!(turns(&owner.status_on(NAV, &b)), ["First on b"]);
    let a = a_orka.lock().unwrap();
    assert_eq!(
        a.log
            .iter()
            .filter(|l| l.starts_with("POST /api/v1/agents?"))
            .count(),
        1
    );
    let s = sessions(&a);
    assert_eq!(s.len(), 2);
    assert_eq!(s[0], s[1], "the second turn on a continues a's Session");
    assert!(
        a.requests("DELETE").is_empty() && b_orka.lock().unwrap().requests("DELETE").is_empty()
    );
}

// A message after switching to this computer goes there, and the cluster's conversation stays.
#[tokio::test]
async fn a_send_after_switching_goes_to_the_selected_place_only() {
    let (orka, open) = serve().await;
    let remote = cluster("team", PROVIDER, open);
    let f = local_fixture::Fixture::new().await;
    let local = Channel::Local(f.vm.clone());
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    say(&owner, &remote, "On the cluster").await;
    say(&owner, &remote, "Again on the cluster").await;
    let here = say(&owner, &local, "whats going on").await;
    assert_eq!(turns(&here), ["whats going on"]);
    assert!(here.error.is_none());
    assert_eq!(f.guest.lock().unwrap().posts("agents").len(), 1);
    // Nothing was deleted or touched on the cluster by the switch.
    let before = orka.lock().unwrap().log.len();
    let there = owner.status_on(NAV, &remote);
    assert_eq!(turns(&there), ["On the cluster", "Again on the cluster"]);
    assert!(there.can_send && there.state == ChatState::Ready);
    assert!(orka.lock().unwrap().requests("DELETE").is_empty());
    assert_eq!(orka.lock().unwrap().log.len(), before);
    // Back on the cluster, the conversation continues where it was.
    let back = say(&owner, &remote, "Back on the cluster").await;
    assert_eq!(back.versions.len(), 1);
    assert_eq!(turns(&back).len(), 3);
    assert_eq!(
        owner.thinks_with_on(NAV, &remote.place()).unwrap().0.model,
        "fixture-model"
    );
    assert_eq!(
        owner.thinks_with_on(NAV, &local.place()).unwrap().0.model,
        "gpt-5.6-terra"
    );
}

// A workspace image that predates the Navigator is never sent a Navigator request: such a guest
// stops serving its channel on one. The refusal is named, and the workspace stays usable.
#[tokio::test]
async fn an_older_image_refuses_the_navigator_before_anything_is_sent() {
    let f = local_fixture::Fixture::older_image().await;
    let local = Channel::Local(f.vm.clone());
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    owner
        .send_navigator_on(local.clone(), "- web-reader".into(), "Create a task".into())
        .unwrap();
    let s = settled(&owner, &local, NAV).await;
    assert_eq!(s.error, Some(ChatError::WorkspaceOutdated));
    assert_eq!(s.state, ChatState::Ready, "never shown as unconfirmed");
    assert!(f.guest.lock().unwrap().requests.is_empty());
    // Try a task on the same workspace still works.
    owner
        .send_on(local.clone(), f.folder.clone(), "Read it".into())
        .unwrap();
    let id = f.folder.document_id();
    let task = settled(&owner, &local, id).await;
    assert_eq!(task.state, ChatState::Ready, "{task:?}");
    assert!(task.error.is_none());
}

// A turn on a place that is no longer selected keeps running, and Stop still reaches it.
#[tokio::test]
async fn a_running_turn_on_an_unselected_place_keeps_running_and_stops_there() {
    let (orka, open) = serve().await;
    orka.lock().unwrap().hold = true;
    let remote = cluster("team", PROVIDER, open);
    let f = local_fixture::Fixture::new().await;
    let local = Channel::Local(f.vm.clone());
    let id = f.folder.document_id();
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    owner
        .send_on(
            remote.clone(),
            f.folder.clone(),
            "Long on the cluster".into(),
        )
        .unwrap();
    until(&owner, &remote, id, |s| s.state == ChatState::Working).await;
    owner
        .send_on(local.clone(), f.folder.clone(), "Quick here".into())
        .unwrap();
    let here = settled(&owner, &local, id).await;
    assert_eq!(turns(&here), ["Quick here"]);
    let there = owner.status_on(id, &remote);
    assert!(there.busy && there.state == ChatState::Working);
    assert_eq!(turns(&there), ["Long on the cluster"]);
    owner.stop_on(remote.clone(), id).unwrap();
    let stopped = settled(&owner, &remote, id).await;
    assert!(stopped.versions[0].turns[0].termination_unconfirmed);
    assert_eq!(orka.lock().unwrap().requests("DELETE").len(), 1);
}

// Beyond the cap, the least recently used idle conversation is dropped as New would drop it.
#[tokio::test]
async fn the_cap_drops_the_oldest_idle_conversation_and_cleans_up_after_it() {
    let (orka, open) = serve().await;
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    let places: Vec<_> = (0..=MAX_CONVERSATIONS)
        .map(|n| cluster(&format!("place-{n}"), PROVIDER, open.clone()))
        .collect();
    for (n, place) in places.iter().enumerate() {
        say(&owner, place, &format!("On place {n}")).await;
    }
    for _ in 0..400 {
        if orka.lock().unwrap().requests("DELETE").len() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        owner.status_on(NAV, &places[0]).versions.is_empty(),
        "the oldest was dropped"
    );
    assert_eq!(turns(&owner.status_on(NAV, &places[1])), ["On place 1"]);
    let deletes = orka.lock().unwrap().requests("DELETE").len();
    assert_eq!(deletes, 3, "its Task, Session and Agent");
}

// Another AI connection on the same cluster shows the earlier conversation read-only until New.
#[tokio::test]
async fn another_ai_connection_shows_the_conversation_read_only_until_new() {
    let (_orka, open) = serve().await;
    let first = cluster("team", PROVIDER, open.clone());
    let other = cluster("team", "another", open);
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    say(&owner, &first, "On the first connection").await;
    let view = owner.status_on(NAV, &other);
    assert!(view.read_only && !view.can_send);
    assert_eq!(turns(&view), ["On the first connection"]);
    assert_eq!(
        owner
            .send_navigator_on(other.clone(), String::new(), "Hello".into())
            .unwrap_err(),
        ChatError::PlaceChanged
    );
    assert!(!owner.status_on(NAV, &first).read_only);
    owner.new_conversation_on(&other, NAV).unwrap();
    assert!(!owner.status_on(NAV, &other).read_only);
}

/// The released guest's Chat contract (protocol at 807a59c1), vendored: only the Try a task
/// family, the fixed namespace, and no Agent or Session deletion.
fn released_guest_accepts(r: &ChatRequest) -> bool {
    fn generated(name: &str, prefix: &str) -> bool {
        name.strip_prefix(prefix).is_some_and(|id| {
            id.len() == 32
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    }
    if r.path.len() > 64 {
        return false;
    }
    let parts: Vec<_> = r.path.split('/').collect();
    let (create, events) = match (r.method, parts.as_slice()) {
        (ChatMethod::Post, ["agents"] | ["tasks"]) => (true, false),
        (ChatMethod::Get, ["agents", n]) if generated(n, "bridge-chat-agent-") => (false, false),
        (ChatMethod::Get | ChatMethod::Delete, ["tasks", n])
            if generated(n, "bridge-chat-task-") =>
        {
            (false, false)
        }
        (ChatMethod::Get, ["tasks", n, "result"]) if generated(n, "bridge-chat-task-") => {
            (false, false)
        }
        (ChatMethod::Get, ["tasks", n, "events"]) if generated(n, "bridge-chat-task-") => {
            (false, true)
        }
        (ChatMethod::Get, ["sessions", n]) if generated(n, "bridge-chat-") => (false, false),
        _ => return false,
    };
    if r.after.is_some() != events {
        return false;
    }
    if !create {
        return r.body.is_none();
    }
    let Some(body) = r.body.as_ref().and_then(Value::as_object) else {
        return false;
    };
    let prefix = if parts[0] == "agents" {
        "bridge-chat-agent-"
    } else {
        "bridge-chat-task-"
    };
    if body.len() != 3
        || body.get("namespace").and_then(Value::as_str) != Some("orka-system")
        || !body
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| generated(n, prefix))
        || !body.get("spec").is_some_and(Value::is_object)
    {
        return false;
    }
    if parts[0] == "tasks" {
        let spec = &body["spec"];
        if spec["type"] != "ai"
            || !spec["agentRef"]["name"]
                .as_str()
                .is_some_and(|n| generated(n, "bridge-chat-agent-"))
            || spec["agentRef"]
                .get("namespace")
                .is_some_and(|n| n.as_str() != Some("orka-system"))
            || !spec["sessionRef"]["name"]
                .as_str()
                .is_some_and(|n| generated(n, "bridge-chat-"))
        {
            return false;
        }
    }
    serde_json::to_vec(body).unwrap().len() <= bridge_runtime_protocol::MAX_ORKA_BODY
}

// Every request Try a task sends on this computer, through two turns, Stop, New and a cluster
// switch, is one the released guest accepts. Agent and Session deletion never leave the cluster path.
#[tokio::test]
async fn every_local_request_is_one_the_released_guest_accepts() {
    let (_orka, open) = serve().await;
    let remote = cluster("team", PROVIDER, open);
    let f = local_fixture::Fixture::new().await;
    let local = Channel::Local(f.vm.clone());
    let id = f.folder.document_id();
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    for message in ["First", "Second"] {
        owner
            .send_on(local.clone(), f.folder.clone(), message.into())
            .unwrap();
        settled(&owner, &local, id).await;
    }
    owner.new_conversation_on(&local, id).unwrap();
    owner
        .send_on(remote.clone(), f.folder.clone(), "There".into())
        .unwrap();
    settled(&owner, &remote, id).await;
    f.guest.lock().unwrap().release = false;
    owner
        .send_on(local.clone(), f.folder.clone(), "Third".into())
        .unwrap();
    until(&owner, &local, id, |s| {
        s.state == ChatState::WaitingForSession
    })
    .await;
    owner.stop_on(local.clone(), id).unwrap();
    f.guest.lock().unwrap().release = true;
    settled(&owner, &local, id).await;
    let g = f.guest.lock().unwrap();
    assert!(g.requests.len() > 10);
    for r in &g.requests {
        assert!(
            released_guest_accepts(r),
            "the released guest would refuse {r:?}"
        );
        assert!(!r.navigator());
        assert!(!(r.method == ChatMethod::Delete && !r.path.starts_with("tasks/")));
    }
}
