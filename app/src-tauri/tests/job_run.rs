//! A job drafted, run and reviewed against a fake guest. The fake answers each Task by its
//! Agent's role (drafter, job, reviewer), so no model is ever called.
use bridge_engine::{
    agent_folder::{AgentFolder, DocumentId},
    example_run::{ExampleRun, JobInput, RunSnapshot, RunState},
    job_run::ReviewVerdict,
    local_vm::{LocalFactory, LocalSession, LocalVm, LocalVmStatus, StartFuture},
    runtime::{client::VmClient, types::RuntimeError},
};
use bridge_runtime_protocol::*;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

const MODEL: &str = "gpt-5.6-terra";
const PROMPT: &str = "Read https://example.com and tell me its title";
const DRAFT: &str = r#"{"instructions":"Fetch the page and reply with its title only.","tools":["web_fetch"],"check":{"type":"toolCalled","tool":"web_fetch"}}"#;
const ANSWER: &str = "The title is Example Domain.";
const REVIEW: &str = r#"{"verdict":"done","summary":"It gives the page's title.","gaps":[]}"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Drafter,
    Job,
    Reviewer,
}
type Hook = Box<dyn FnOnce() + Send>;
#[derive(Default)]
struct Script {
    answers: Mutex<HashMap<&'static str, String>>,
    /// Tasks for this role stay Running until cancelled.
    hold: Mutex<Option<Role>>,
    /// Runs once, when the first result for this role is read.
    after_result: Mutex<Option<(Role, Hook)>>,
    /// Runs once, when the first Task for this role is created.
    on_create: Mutex<Option<(Role, Hook)>>,
    posts: Mutex<Vec<(String, Value)>>,
    cleaned: Mutex<Vec<String>>,
}
impl Script {
    fn new() -> Arc<Self> {
        let script = Self::default();
        {
            let mut answers = script.answers.lock().unwrap();
            answers.insert("drafter", DRAFT.into());
            answers.insert("job", ANSWER.into());
            answers.insert("reviewer", REVIEW.into());
        }
        Arc::new(script)
    }
    fn answer(&self, role: &'static str, text: &str) {
        self.answers.lock().unwrap().insert(role, text.into());
    }
    fn created(&self, role: Role) -> usize {
        self.posts
            .lock()
            .unwrap()
            .iter()
            .filter(|(path, body)| path == "agents" && role_of(&body["spec"]) == role)
            .count()
    }
    fn tasks(&self) -> usize {
        self.posts
            .lock()
            .unwrap()
            .iter()
            .filter(|(path, _)| path == "tasks")
            .count()
    }
}
fn role_of(agent_spec: &Value) -> Role {
    let prompt = agent_spec["systemPrompt"]["inline"]
        .as_str()
        .unwrap_or("")
        .trim_start();
    if prompt.starts_with("You turn a job") {
        Role::Drafter
    } else if prompt.starts_with("You review") {
        Role::Reviewer
    } else {
        Role::Job
    }
}
fn key(role: Role) -> &'static str {
    match role {
        Role::Drafter => "drafter",
        Role::Job => "job",
        Role::Reviewer => "reviewer",
    }
}

struct Channel {
    input: io::Cursor<Vec<u8>>,
    output: Vec<u8>,
    objects: HashMap<String, Value>,
    script: Arc<Script>,
    connected: bool,
}
impl Channel {
    fn task_role(&self, name: &str) -> Role {
        let agent = self.objects[&format!("tasks/{name}")]["spec"]["agentRef"]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        role_of(&self.objects[&format!("agents/{agent}")]["spec"])
    }
    fn orka(&mut self, request: OrkaRequest) -> io::Result<(u16, Value)> {
        let path = request.path.clone();
        if path.ends_with("/cancel") {
            let name = path.split('/').nth(1).unwrap().to_owned();
            self.objects.get_mut(&format!("tasks/{name}")).unwrap()["status"]["phase"] =
                json!("Cancelled");
            return Ok((200, json!({})));
        }
        if let Some(body) = request.body {
            self.script
                .posts
                .lock()
                .unwrap()
                .push((path.clone(), body.clone()));
            let name = body["name"].as_str().unwrap().to_owned();
            let kind = if path == "agents" { "Agent" } else { "Task" };
            let mut object = json!({"apiVersion":"core.orka.ai/v1alpha1","kind":kind,"metadata":{"name":name,"namespace":"orka-system","uid":format!("uid-{name}"),"generation":1},"spec":body["spec"],"status":{"phase":"Succeeded"}});
            if path == "tasks" {
                object["spec"]["timeout"] = json!("2m0s");
                object["spec"]["retryPolicy"]
                    .as_object_mut()
                    .unwrap()
                    .remove("maxRetries");
                self.objects.insert(format!("tasks/{name}"), object.clone());
                let role = self.task_role(&name);
                if *self.script.hold.lock().unwrap() == Some(role) {
                    object["status"]["phase"] = json!("Running");
                    self.objects.insert(format!("tasks/{name}"), object.clone());
                }
                let hook = {
                    let mut slot = self.script.on_create.lock().unwrap();
                    match slot.take() {
                        Some((wanted, hook)) if wanted == role => Some(hook),
                        other => {
                            *slot = other;
                            None
                        }
                    }
                };
                if let Some(hook) = hook {
                    hook();
                }
            } else {
                self.objects
                    .insert(format!("agents/{name}"), object.clone());
            }
            return Ok((201, object));
        }
        if path.ends_with("/result") {
            let name = path.split('/').nth(1).unwrap().to_owned();
            let role = self.task_role(&name);
            let answer = self.script.answers.lock().unwrap()[key(role)].clone();
            let hook = {
                let mut slot = self.script.after_result.lock().unwrap();
                match slot.take() {
                    Some((wanted, hook)) if wanted == role => Some(hook),
                    other => {
                        *slot = other;
                        None
                    }
                }
            };
            if let Some(hook) = hook {
                hook();
            }
            return Ok((200, json!({ "result": answer })));
        }
        if path.ends_with("/events") {
            let name = path.split('/').nth(1).unwrap().to_owned();
            let agent = self.objects[&format!("tasks/{name}")]["spec"]["agentRef"]["name"].clone();
            let after = request.after.unwrap();
            let used = self.task_role(&name) == Role::Job;
            let events = if used {
                vec![
                    json!({"namespace":"orka-system","streamType":"task","streamID":name,"seq":after+1,"type":"ToolCallCompleted","taskName":name,"agentName":agent,"toolName":"web_fetch","toolCallID":"call-1"}),
                ]
            } else {
                vec![]
            };
            return Ok((
                200,
                json!({"namespace":"orka-system","streamType":"task","streamID":name,"afterSeq":after,"latestSeq":after + events.len() as u64,"events":events}),
            ));
        }
        Ok((200, self.objects[&path].clone()))
    }
    fn completed(
        &mut self,
        operation: Uuid,
        value: &impl serde::Serialize,
    ) -> io::Result<ResultBody> {
        self.output = serde_json::to_vec(value)?;
        Ok(ResultBody::Operation {
            operation,
            info: OperationInfo {
                state: OperationState::Completed,
                exit: Some(0),
                bytes: self.output.len(),
                has_diagnostics: false,
            },
        })
    }
}
impl Write for Channel {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Frame::Request(r) = serde_json::from_slice(bytes)? else {
            panic!("request")
        };
        let result = match r.body {
            RequestBody::ModelStatus {} => ResultBody::ModelState(ModelState {
                retained_cleanup: true,
                diagnostic: None,
                phase: if self.connected {
                    ModelPhase::Ready
                } else {
                    ModelPhase::Idle
                },
                credential: CredentialState {
                    profile: CREDENTIAL_PROFILE,
                    generation: Some(r.boot_nonce),
                    installed: self.connected,
                },
                connection: self.connected.then(|| ModelConnection {
                    id: r.boot_nonce,
                    generation: r.boot_nonce,
                    selection: ModelSelection {
                        vendor: None,
                        provider: "copilot".into(),
                        model: MODEL.into(),
                    },
                }),
            }),
            RequestBody::Execute {
                operation,
                action: EngineActionWire::Orka { request },
                ..
            } => {
                let (status, body) = self.orka(request)?;
                self.completed(operation, &json!({"status":status,"body":body}))?
            }
            RequestBody::Execute {
                operation,
                action: EngineActionWire::RunWorker { name, .. },
                ..
            } => {
                let phase = self.objects[&format!("tasks/{name}")]["status"]["phase"].clone();
                let finished = matches!(phase.as_str(), Some("Succeeded" | "Failed" | "Cancelled"));
                self.completed(operation, &json!({"finished":finished,"pods":0}))?
            }
            RequestBody::Execute {
                operation,
                action: EngineActionWire::CleanupRunAgent { uid },
                ..
            } => {
                self.script.cleaned.lock().unwrap().push(uid.clone());
                self.objects
                    .retain(|_, o| o["kind"] != "Agent" || o["metadata"]["uid"] != uid);
                self.completed(operation, &CleanupRunReply { removed: true })?
            }
            RequestBody::Status { operation } => ResultBody::Operation {
                operation,
                info: OperationInfo {
                    state: OperationState::Completed,
                    exit: Some(0),
                    bytes: self.output.len(),
                    has_diagnostics: false,
                },
            },
            RequestBody::ReadResult { offset, .. } => ResultBody::Chunk {
                data: encode_chunk(&self.output[offset..]),
            },
            _ => panic!("unexpected request"),
        };
        let mut data = serde_json::to_vec(&Frame::Result(WireResult {
            version: VERSION,
            boot_nonce: r.boot_nonce,
            request_id: r.request_id,
            body: result,
        }))?;
        data.push(b'\n');
        self.input = io::Cursor::new(data);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Read for Channel {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.input.read(bytes)
    }
}
struct Session(VmClient, Uuid);
impl LocalSession for Session {
    fn run_cancel_supported(&self) -> bool {
        true
    }
    fn with_client(
        &mut self,
        work: &mut dyn FnMut(Uuid, &mut VmClient) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        work(self.1, &mut self.0)
    }
    fn execute(
        &mut self,
        _: bridge_engine::engine::types::EngineRequest,
    ) -> bridge_engine::engine::TransportFuture<'_> {
        Box::pin(async {
            Err(bridge_engine::engine::types::EngineError::new(
                bridge_engine::engine::types::ErrorKind::Unavailable,
            ))
        })
    }
    fn stop(&mut self) -> Result<(), RuntimeError> {
        Ok(())
    }
}
struct Factory(Mutex<Option<Session>>);
impl LocalFactory for Factory {
    fn launch(
        &self,
        _: watch::Receiver<bool>,
        _: Arc<dyn Fn(bridge_engine::local_vm::LocalProgress) + Send + Sync>,
    ) -> StartFuture<'_> {
        Box::pin(async {
            Ok(Box::new(self.0.lock().unwrap().take().unwrap()) as Box<dyn LocalSession>)
        })
    }
}

struct Fixture {
    vm: Arc<LocalVm>,
    run: Arc<ExampleRun>,
    script: Arc<Script>,
    home: tempfile::TempDir,
}
async fn fixture_with(script: Arc<Script>, connected: bool) -> Fixture {
    let id = Uuid::new_v4();
    let client = VmClient::established_with_hello(
        Box::new(Channel {
            input: io::Cursor::new(Vec::new()),
            output: Vec::new(),
            objects: HashMap::new(),
            script: script.clone(),
            connected,
        }),
        id,
        &StageHello {
            version: VERSION,
            stage: Stage::Serve,
            guest_nonce: Uuid::new_v4(),
            accepted_boot_nonce: Some(id),
            run_cleanup: true,
        },
    )
    .unwrap();
    let vm = LocalVm::new(Arc::new(Factory(Mutex::new(Some(Session(client, id))))));
    vm.start();
    for _ in 0..100 {
        if vm.status() == LocalVmStatus::Ready {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(vm.status(), LocalVmStatus::Ready);
    let home = tempfile::tempdir().unwrap();
    let run = ExampleRun::with_history_root(
        tokio::runtime::Handle::current(),
        true,
        Arc::new(AgentFolder::new(Some(home.path().join("agents")))),
        Some(home.path().join("data")),
    );
    Fixture {
        vm,
        run,
        script,
        home,
    }
}
async fn fixture(script: Arc<Script>) -> Fixture {
    fixture_with(script, true).await
}
fn job(prompt: &str) -> JobInput {
    JobInput {
        id: doc(Uuid::new_v4()),
        prompt: prompt.into(),
    }
}
async fn finished(f: &Fixture, id: DocumentId) -> RunSnapshot {
    for _ in 0..2000 {
        let status = f.run.status_document(&f.vm, id);
        if status.state != RunState::Running {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("job never finished")
}
fn json_of(snapshot: &RunSnapshot) -> Value {
    serde_json::to_value(snapshot).unwrap()
}

#[tokio::test]
async fn job_is_drafted_run_and_reviewed_and_only_its_own_run_is_kept() {
    let f = fixture(Script::new()).await;
    let input = job(PROMPT);
    let started = f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    assert_eq!(started.state, RunState::Running);
    assert_eq!(json_of(&started)["job"]["stage"], "draft");
    let done = finished(&f, input.id).await;
    assert_eq!(done.state, RunState::Complete);
    assert_eq!(done.error, None);
    let outcome = done.job.as_ref().unwrap();
    assert_eq!(outcome.stage, "review");
    assert_eq!(
        outcome.instructions.as_deref(),
        Some("Fetch the page and reply with its title only.")
    );
    assert_eq!(outcome.tools, ["web_fetch"]);
    assert_eq!(outcome.answer.as_deref(), Some(ANSWER));
    assert_eq!(outcome.used, ["web_fetch"]);
    let review = outcome.review.as_ref().unwrap();
    assert_eq!(review.verdict, ReviewVerdict::Done);
    assert!(review.gaps.is_empty());
    assert!(outcome.finished_at.unwrap() >= outcome.started_at);
    assert_eq!(done.model.as_deref(), Some(MODEL));
    for role in [Role::Drafter, Role::Job, Role::Reviewer] {
        assert_eq!(f.script.created(role), 1, "{role:?}");
    }
    assert_eq!(f.script.tasks(), 3);
    // Drafter and reviewer Agents are cleaned up; history points at the job's Agent.
    let history = f.run.local_results().unwrap();
    assert_eq!(history.results.len(), 1);
    let row = &history.results[0];
    assert_eq!(row.document_id, input.id);
    let job_agent = f
        .script
        .posts
        .lock()
        .unwrap()
        .iter()
        .find(|(path, body)| path == "agents" && role_of(&body["spec"]) == Role::Job)
        .map(|(_, body)| format!("uid-{}", body["name"].as_str().unwrap()))
        .unwrap();
    assert_eq!(row.agent_uid, job_agent);
    let cleaned = f.script.cleaned.lock().unwrap().clone();
    assert_eq!(cleaned.len(), 2);
    assert!(!cleaned.contains(&job_agent));
    assert!(!f.run.is_active());
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn every_stage_uses_the_connected_model_and_only_the_job_has_tools() {
    let f = fixture(Script::new()).await;
    let input = job(PROMPT);
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    finished(&f, input.id).await;
    let posts = f.script.posts.lock().unwrap().clone();
    for (path, body) in &posts {
        if path != "agents" {
            continue;
        }
        let spec = &body["spec"];
        assert_eq!(spec["model"]["name"], MODEL);
        let tools = spec["tools"].as_array().unwrap();
        match role_of(spec) {
            Role::Job => assert_eq!(tools, &[json!({"name":"web_fetch"})]),
            _ => assert!(tools.is_empty()),
        }
    }
    let prompts: Vec<_> = posts
        .iter()
        .filter(|(path, _)| path == "tasks")
        .map(|(_, body)| body["spec"]["prompt"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(prompts[0], PROMPT);
    assert_eq!(prompts[1], PROMPT);
    assert_eq!(prompts[2], format!("Job:\n{PROMPT}\n\nAnswer:\n{ANSWER}"));
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_rerun_reuses_the_saved_draft() {
    let f = fixture(Script::new()).await;
    let input = job(PROMPT);
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    finished(&f, input.id).await;
    // A different drafter answer would show up if it were asked again.
    f.script.answer(
        "drafter",
        r#"{"instructions":"Other.","tools":[],"check":{"type":"contains","value":"x"}}"#,
    );
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    let again = finished(&f, input.id).await;
    assert_eq!(again.error, None);
    assert_eq!(f.script.created(Role::Drafter), 1);
    assert_eq!(f.script.created(Role::Job), 2);
    assert_eq!(
        again.job.unwrap().instructions.as_deref(),
        Some("Fetch the page and reply with its title only.")
    );
    // Another sentence under the same id drafts again.
    let edited = JobInput {
        id: input.id,
        prompt: "Read https://example.com and summarize it".into(),
    };
    f.run.start_job(f.vm.clone(), edited.clone()).unwrap();
    let redrafted = finished(&f, edited.id).await;
    assert_eq!(f.script.created(Role::Drafter), 2);
    assert_eq!(
        redrafted.job.unwrap().instructions.as_deref(),
        Some("Other.")
    );
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_unusable_draft_ends_before_the_job_runs() {
    for answer in [
        "I can't help with that.",
        r#"{"instructions":"x","tools":["code_exec"],"check":{"type":"contains","value":"x"}}"#,
        "",
    ] {
        let script = Script::new();
        script.answer("drafter", answer);
        let f = fixture(script).await;
        let input = job(PROMPT);
        f.run.start_job(f.vm.clone(), input.clone()).unwrap();
        let done = finished(&f, input.id).await;
        assert_eq!(done.state, RunState::Unknown, "{answer:?}");
        assert_eq!(done.error, Some("draft"));
        let outcome = done.job.unwrap();
        assert_eq!(outcome.stage, "draft");
        assert!(outcome.answer.is_none() && outcome.review.is_none());
        assert_eq!(f.script.created(Role::Job), 0);
        assert_eq!(f.script.created(Role::Reviewer), 0);
        assert!(!f.run.is_active());
        f.vm.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn an_unusable_review_keeps_the_answer_and_says_so() {
    let script = Script::new();
    script.answer(
        "reviewer",
        r#"{"verdict":"passed","summary":"Looks good.","gaps":[]}"#,
    );
    let f = fixture(script).await;
    let input = job(PROMPT);
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    let done = finished(&f, input.id).await;
    assert_eq!(done.state, RunState::Unknown);
    assert_eq!(done.error, Some("review"));
    let outcome = done.job.unwrap();
    assert_eq!(outcome.answer.as_deref(), Some(ANSWER));
    assert!(outcome.review.is_none());
    assert!(outcome.finished_at.is_some());
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn no_answer_skips_the_review() {
    let script = Script::new();
    script.answer("job", "   ");
    let f = fixture(script).await;
    let input = job(PROMPT);
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    let done = finished(&f, input.id).await;
    assert_eq!(done.state, RunState::Unknown);
    assert_eq!(done.error, Some("answer"));
    assert_eq!(done.job.unwrap().stage, "run");
    assert_eq!(f.script.created(Role::Reviewer), 0);
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn review_partly_lists_what_was_left_out() {
    let script = Script::new();
    script.answer(
        "reviewer",
        "```json\n{\"verdict\":\"partly\",\"summary\":\"Title only.\",\"gaps\":[\"No link to the page.\"]}\n```",
    );
    let f = fixture(script).await;
    let input = job(PROMPT);
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    let done = finished(&f, input.id).await;
    let review = done.job.unwrap().review.unwrap();
    assert_eq!(review.verdict, ReviewVerdict::Partly);
    assert_eq!(review.gaps, ["No link to the page."]);
    let wire = json_of(&f.run.status_document(&f.vm, input.id));
    assert_eq!(wire["job"]["review"]["verdict"], "partly");
    f.vm.shutdown().await.unwrap();
}

fn stop_hook(f: &Fixture, id: DocumentId) -> Hook {
    let run = Arc::downgrade(&f.run);
    let vm = Arc::downgrade(&f.vm);
    Box::new(move || {
        run.upgrade().unwrap().cancel(vm.upgrade().unwrap(), id);
    })
}

#[tokio::test]
async fn stop_while_drafting_cancels_the_drafter_and_nothing_else_runs() {
    let script = Script::new();
    *script.hold.lock().unwrap() = Some(Role::Drafter);
    let f = fixture(script).await;
    let input = job(PROMPT);
    *f.script.on_create.lock().unwrap() = Some((Role::Drafter, stop_hook(&f, input.id)));
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    let done = finished(&f, input.id).await;
    assert_eq!(json_of(&done)["cancellation"], "cancelled");
    assert_eq!(f.script.created(Role::Job), 0);
    assert_eq!(done.job.unwrap().stage, "draft");
    assert!(f
        .script
        .posts
        .lock()
        .unwrap()
        .iter()
        .all(|(path, _)| !path.ends_with("/cancel")));
    assert!(!f.run.is_active());
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_while_the_job_runs_cancels_it_and_skips_the_review() {
    let script = Script::new();
    *script.hold.lock().unwrap() = Some(Role::Job);
    let f = fixture(script).await;
    let input = job(PROMPT);
    *f.script.on_create.lock().unwrap() = Some((Role::Job, stop_hook(&f, input.id)));
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    let done = finished(&f, input.id).await;
    assert_eq!(json_of(&done)["cancellation"], "cancelled");
    assert_eq!(done.error, None);
    let outcome = done.job.unwrap();
    assert_eq!(outcome.stage, "run");
    assert!(outcome.instructions.is_some(), "the draft is kept");
    assert!(outcome.review.is_none());
    assert_eq!(f.script.created(Role::Reviewer), 0);
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_between_drafting_and_running_never_starts_the_job() {
    let f = fixture(Script::new()).await;
    let input = job(PROMPT);
    // Stop lands after the drafter's answer is read, before the job's Task exists.
    *f.script.after_result.lock().unwrap() = Some((Role::Drafter, stop_hook(&f, input.id)));
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    let done = finished(&f, input.id).await;
    let wire = json_of(&done);
    assert!(
        matches!(
            wire["cancellation"].as_str(),
            Some("cancelled" | "beforeStart")
        ),
        "{wire}"
    );
    assert_eq!(f.script.tasks(), 1, "only the drafter's Task was created");
    assert_eq!(f.script.created(Role::Reviewer), 0);
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_between_running_and_review_keeps_the_answer_unreviewed() {
    let f = fixture(Script::new()).await;
    let input = job(PROMPT);
    *f.script.after_result.lock().unwrap() = Some((Role::Job, stop_hook(&f, input.id)));
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    let done = finished(&f, input.id).await;
    let wire = json_of(&done);
    assert!(
        matches!(
            wire["cancellation"].as_str(),
            Some("cancelled" | "beforeStart")
        ),
        "{wire}"
    );
    let outcome = done.job.unwrap();
    assert_eq!(outcome.answer.as_deref(), Some(ANSWER));
    assert!(outcome.review.is_none());
    assert_eq!(f.script.tasks(), 2);
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_while_reviewing_cancels_the_review() {
    let script = Script::new();
    *script.hold.lock().unwrap() = Some(Role::Reviewer);
    let f = fixture(script).await;
    let input = job(PROMPT);
    *f.script.on_create.lock().unwrap() = Some((Role::Reviewer, stop_hook(&f, input.id)));
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    let done = finished(&f, input.id).await;
    assert_eq!(json_of(&done)["cancellation"], "cancelled");
    let outcome = done.job.unwrap();
    assert_eq!(outcome.stage, "review");
    assert_eq!(outcome.answer.as_deref(), Some(ANSWER));
    assert!(outcome.review.is_none());
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_jobs_are_refused_before_any_request() {
    let f = fixture(Script::new()).await;
    for bad in [
        JobInput {
            id: DocumentId::SAMPLE,
            prompt: PROMPT.into(),
        },
        JobInput {
            id: doc(Uuid::nil()),
            prompt: PROMPT.into(),
        },
        job(""),
        job(" padded "),
        job("bell\u{7}"),
        job(&"a".repeat(8 * 1024 + 1)),
    ] {
        assert!(f.run.start_job(f.vm.clone(), bad).is_err());
    }
    assert!(f.script.posts.lock().unwrap().is_empty());
    assert!(!f.run.is_active());
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_job_waits_for_the_ai_connection() {
    let f = fixture_with(Script::new(), false).await;
    let input = job(PROMPT);
    f.run.start_job(f.vm.clone(), input.clone()).unwrap();
    let done = finished(&f, input.id).await;
    assert_eq!(done.error, Some("model"));
    assert!(f.script.posts.lock().unwrap().is_empty());
    f.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn one_run_at_a_time_across_jobs_and_agents() {
    let script = Script::new();
    *script.hold.lock().unwrap() = Some(Role::Drafter);
    let f = fixture(script).await;
    let first = job(PROMPT);
    let second = job("Another job");
    let seen = Arc::new(Mutex::new(Vec::new()));
    {
        let run = Arc::downgrade(&f.run);
        let vm = Arc::downgrade(&f.vm);
        let (seen, second, first_id) = (seen.clone(), second.clone(), first.id);
        *f.script.on_create.lock().unwrap() = Some((
            Role::Drafter,
            Box::new(move || {
                let (run, vm) = (run.upgrade().unwrap(), vm.upgrade().unwrap());
                let busy = run.start_job(vm.clone(), second).unwrap();
                seen.lock()
                    .unwrap()
                    .push((busy.state, busy.busy.map(|b| b.name)));
                let agent = run.start(vm.clone());
                seen.lock()
                    .unwrap()
                    .push((agent.state, agent.busy.map(|b| b.name)));
                run.cancel(vm, first_id);
            }),
        ));
    }
    f.run.start_job(f.vm.clone(), first.clone()).unwrap();
    finished(&f, first.id).await;
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        [
            (RunState::Idle, Some("Your job".to_owned())),
            (RunState::Idle, Some("Your job".to_owned())),
        ]
    );
    assert_eq!(
        f.script.created(Role::Drafter),
        1,
        "the second job never started"
    );
    assert!(f.home.path().exists());
    f.vm.shutdown().await.unwrap();
}
fn doc(id: Uuid) -> DocumentId {
    serde_json::from_value(json!(id)).unwrap()
}
