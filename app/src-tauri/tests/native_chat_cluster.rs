//! The conversation state machine on a cluster: the same turns as the local workspace, sent
//! over a fake Orka API on real HTTP in the place's watched namespace.
use bridge_engine::{
    agent_folder::AgentFolder,
    chat::{Channel, ChatError, ChatOwner, ChatSnapshot, ChatState},
    chat_cluster::{ChatPermission, ClusterChannel},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
#[path = "support/cluster_chat_fixture.rs"]
mod cluster_fixture;
use cluster_fixture::{serve, Orka, NAMESPACE, PROVIDER};

async fn place() -> (Arc<Mutex<Orka>>, Channel) {
    let (orka, open) = serve().await;
    let channel =
        ClusterChannel::new("place-key".into(), NAMESPACE.into(), PROVIDER.into(), open).unwrap();
    (orka, Channel::Cluster(Arc::new(channel)))
}
async fn until(
    owner: &ChatOwner,
    channel: &Channel,
    id: bridge_engine::agent_folder::DocumentId,
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
async fn settled(
    owner: &ChatOwner,
    channel: &Channel,
    id: bridge_engine::agent_folder::DocumentId,
) -> ChatSnapshot {
    until(owner, channel, id, |s| !s.busy).await
}
fn folder() -> (tempfile::TempDir, Arc<AgentFolder>) {
    let home = tempfile::tempdir().unwrap();
    let folder = Arc::new(AgentFolder::new(Some(home.path().into())));
    (home, folder)
}
fn navigator() -> bridge_engine::agent_folder::DocumentId {
    bridge_engine::navigator::CONVERSATION
}

// Two Navigator turns keep one Agent and one Session; the second Task appends without create.
#[tokio::test]
async fn navigator_turns_on_a_cluster_share_one_session_and_the_default_model() {
    let (orka, channel) = place().await;
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    owner
        .send_navigator_on(
            channel.clone(),
            "- web-reader".into(),
            "What is a Task?".into(),
        )
        .unwrap();
    let first = settled(&owner, &channel, navigator()).await;
    assert_eq!(first.state, ChatState::Ready, "{first:?}");
    owner
        .send_navigator_on(
            channel.clone(),
            "- web-reader".into(),
            "And a Session?".into(),
        )
        .unwrap();
    let second = settled(&owner, &channel, navigator()).await;
    assert_eq!(second.state, ChatState::Ready, "{second:?}");
    assert_eq!(second.versions.len(), 1);
    let turns = &second.versions[0].turns;
    assert_eq!(turns.len(), 2);
    assert!(turns
        .iter()
        .all(|t| t.context_retained && t.phase == "Succeeded" && t.tools.len() == 2));
    assert!(turns[1]
        .answer
        .as_deref()
        .unwrap()
        .starts_with("Answer for bridge-nav-task-"));
    let place = bridge_engine::chat::Place::Cluster("place-key".into());
    let (thinks, cluster) = owner.thinks_with_on(navigator(), &place).unwrap();
    assert!(cluster);
    assert_eq!(thinks.model, "fixture-model");
    let o = orka.lock().unwrap();
    let agents: Vec<_> = o
        .bodies
        .iter()
        .filter(|b| b["name"].as_str().unwrap().starts_with("bridge-nav-agent-"))
        .collect();
    let tasks: Vec<_> = o
        .bodies
        .iter()
        .filter(|b| b["name"].as_str().unwrap().starts_with("bridge-nav-task-"))
        .collect();
    assert_eq!((agents.len(), tasks.len()), (1, 2));
    assert_eq!(agents[0]["spec"]["providerRef"]["name"], PROVIDER);
    assert_eq!(agents[0]["spec"]["model"]["name"], "fixture-model");
    let session = &tasks[0]["spec"]["sessionRef"];
    assert!(session["name"].as_str().unwrap().starts_with("bridge-nav-"));
    assert_eq!(session["create"], true);
    assert_eq!(tasks[1]["spec"]["sessionRef"]["name"], session["name"]);
    assert!(tasks[1]["spec"]["sessionRef"].get("create").is_none());
    // The Provider is read once, for the first turn's agent file, before anything is created.
    assert!(o.log[0].starts_with(&format!(
        "GET /api/v1/providers/{PROVIDER}?namespace={NAMESPACE}"
    )));
    assert!(o.log[1].starts_with("POST /api/v1/agents?namespace=team-a"));
    assert!(o.log[2].starts_with("POST /api/v1/tasks?namespace=team-a"));
    assert!(o.requests("DELETE").is_empty());
    assert_eq!(o.opens, o.closes, "every worker run closes its connection");
}

// New deletes what the replaced conversation created: its Tasks, then its Session, then its Agent.
#[tokio::test]
async fn new_conversation_deletes_the_previous_conversations_resources() {
    let (orka, channel) = place().await;
    let (_home, folder) = folder();
    let id = folder.document_id();
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    owner
        .send_on(channel.clone(), folder.clone(), "First".into())
        .unwrap();
    assert_eq!(settled(&owner, &channel, id).await.state, ChatState::Ready);
    owner
        .send_on(channel.clone(), folder, "Second".into())
        .unwrap();
    assert_eq!(settled(&owner, &channel, id).await.state, ChatState::Ready);
    owner.new_conversation_on(&channel, id).unwrap();
    for _ in 0..400 {
        if orka.lock().unwrap().requests("DELETE").len() == 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let o = orka.lock().unwrap();
    let deletes: Vec<_> = o
        .requests("DELETE")
        .iter()
        .map(|l| l.split('?').next().unwrap().to_owned())
        .collect();
    assert_eq!(deletes.len(), 4, "{:?}", o.log);
    assert!(deletes[0].starts_with("DELETE /api/v1/tasks/bridge-chat-task-"));
    assert!(deletes[1].starts_with("DELETE /api/v1/tasks/bridge-chat-task-"));
    assert!(deletes[2].starts_with("DELETE /api/v1/sessions/bridge-chat-"));
    assert!(deletes[3].starts_with("DELETE /api/v1/agents/bridge-chat-agent-"));
    assert!(!owner.status_on(id, &channel).kept_on_cluster);
}

// A role without delete leaves everything and says so once.
#[tokio::test]
async fn refused_cleanup_keeps_resources_and_latches_one_notice() {
    let (orka, channel) = place().await;
    orka.lock().unwrap().deny.insert("DELETE tasks".into());
    let (_home, folder) = folder();
    let id = folder.document_id();
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    owner
        .send_on(channel.clone(), folder, "First".into())
        .unwrap();
    assert_eq!(settled(&owner, &channel, id).await.state, ChatState::Ready);
    owner.new_conversation_on(&channel, id).unwrap();
    let s = until(&owner, &channel, id, |s| s.kept_on_cluster).await;
    assert!(s.error.is_none() && s.can_send);
}

// A refused create names the permission and creates nothing further.
#[tokio::test]
async fn a_refused_task_create_names_the_missing_permission() {
    let (orka, channel) = place().await;
    orka.lock().unwrap().deny.insert("POST tasks".into());
    let (_home, folder) = folder();
    let id = folder.document_id();
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    owner
        .send_on(channel.clone(), folder, "First".into())
        .unwrap();
    let s = settled(&owner, &channel, id).await;
    assert_eq!(s.error, Some(ChatError::Permission));
    assert_eq!(s.denied, Some(ChatPermission::CreateTasks));
    let o = orka.lock().unwrap();
    assert_eq!(o.requests("POST").len(), 2);
    assert!(
        o.requests("GET").is_empty(),
        "nothing is read after a refused create: {:?}",
        o.log
    );
}

// Stop deletes the running Task, as the local path does, and the Session is then released.
#[tokio::test]
async fn stop_deletes_the_running_task() {
    let (orka, channel) = place().await;
    orka.lock().unwrap().hold = true;
    let (_home, folder) = folder();
    let id = folder.document_id();
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    owner
        .send_on(channel.clone(), folder, "Long task".into())
        .unwrap();
    until(&owner, &channel, id, |s| s.state == ChatState::Working).await;
    owner.stop_on(channel.clone(), id).unwrap();
    let s = settled(&owner, &channel, id).await;
    assert!(s.versions[0].turns[0].termination_unconfirmed, "{s:?}");
    // Absent unless true, so the renderer's exact turn shape is unchanged.
    assert!(serde_json::to_value(&s.versions[0].turns[0])
        .unwrap()
        .get("finishedBeforeStop")
        .is_none());
    let o = orka.lock().unwrap();
    assert_eq!(o.requests("DELETE").len(), 1);
    assert!(o.requests("DELETE")[0].starts_with("DELETE /api/v1/tasks/bridge-chat-task-"));
}

// Stop pressed after the worker's last request is never lost: the turn says its Task had finished.
#[tokio::test]
async fn a_stop_that_arrives_as_the_task_finishes_says_it_had_already_finished() {
    let (orka, channel) = place().await;
    let gate = Arc::new(tokio::sync::Notify::new());
    orka.lock().unwrap().session_gate = Some(gate.clone());
    let (_home, folder) = folder();
    let id = folder.document_id();
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    owner
        .send_on(channel.clone(), folder, "Quick task".into())
        .unwrap();
    for _ in 0..2000 {
        if orka.lock().unwrap().session_waiting {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(orka.lock().unwrap().session_waiting);
    let pressed = owner.stop_on(channel.clone(), id).unwrap();
    assert_eq!(
        pressed.state,
        ChatState::RequestingStop,
        "acknowledged at once"
    );
    gate.notify_one();
    let s = settled(&owner, &channel, id).await;
    let turn = &s.versions[0].turns[0];
    assert_eq!(turn.phase, "Succeeded");
    assert!(turn.finished_before_stop, "{s:?}");
    assert_eq!(
        serde_json::to_value(turn).unwrap()["finishedBeforeStop"],
        true
    );
    assert!(!turn.termination_unconfirmed);
    assert!(orka.lock().unwrap().requests("DELETE").is_empty());
    // Stop pressed again on the finished turn is answered the same way, not refused.
    let again = owner.stop_on(channel.clone(), id).unwrap();
    assert!(again.versions[0].turns[0].finished_before_stop);
    assert!(!again.busy);
}

// An Orka without Sessions drops the reference: the one-off Task is deleted and the reason is precise.
#[tokio::test]
async fn an_orka_without_sessions_is_named_and_its_task_deleted() {
    let (orka, channel) = place().await;
    orka.lock().unwrap().no_sessions = true;
    let (_home, folder) = folder();
    let id = folder.document_id();
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    owner
        .send_on(channel.clone(), folder, "First".into())
        .unwrap();
    let s = settled(&owner, &channel, id).await;
    assert_eq!(s.error, Some(ChatError::SessionsUnsupported));
    let o = orka.lock().unwrap();
    assert_eq!(o.requests("DELETE").len(), 1);
}

// The next message's connection may briefly lose the shared kubeconfig slot to a cleanup.
#[tokio::test]
async fn a_briefly_refused_open_is_retried_before_the_message_fails() {
    let (orka, channel) = place().await;
    orka.lock().unwrap().refuse_opens = 2;
    let (_home, folder) = folder();
    let id = folder.document_id();
    let owner = ChatOwner::new(tokio::runtime::Handle::current());
    owner
        .send_on(channel.clone(), folder, "First".into())
        .unwrap();
    let s = settled(&owner, &channel, id).await;
    assert_eq!(s.state, ChatState::Ready, "{s:?}");
    assert!(s.error.is_none());
}
