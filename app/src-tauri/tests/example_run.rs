use bridge_engine::{
    example_run::{ExampleRun, RunState},
    local_vm::{LocalFactory, LocalSession, LocalVm, LocalVmStatus, StartFuture},
    runtime::{client::VmClient, types::RuntimeError},
};
use bridge_runtime_protocol::*;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::watch;
const ANSWER: &str = "cf699c56340dc775230fde4ef84237d27563ea6ef52164c7d078072b586c3918";
struct Channel {
    input: std::io::Cursor<Vec<u8>>,
    objects: HashMap<String, Value>,
    output: Vec<u8>,
    missing_tools: bool,
    posts: Arc<Mutex<Vec<Value>>>,
    lost_task: bool,
}
impl Write for Channel {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Frame::Request(r) = serde_json::from_slice(bytes)? else {
            panic!("request")
        };
        let result = match r.body {
            RequestBody::ModelStatus {} => ResultBody::ModelState(ModelState {
                phase: ModelPhase::Ready,
                credential: CredentialState {
                    profile: CREDENTIAL_PROFILE,
                    generation: Some(r.boot_nonce),
                    installed: true,
                },
                connection: Some(ModelConnection {
                    id: r.boot_nonce,
                    generation: r.boot_nonce,
                    selection: ModelSelection {
                        provider: "copilot".into(),
                        model: "gpt-5.6-terra".into(),
                    },
                }),
            }),
            RequestBody::Execute {
                operation,
                action: EngineActionWire::Orka { request },
                ..
            } => {
                let mut status = 200;
                let body = if let Some(body) = request.body {
                    self.posts.lock().unwrap().push(body.clone());
                    if self.lost_task && request.path == "tasks" {
                        return Err(io::ErrorKind::BrokenPipe.into());
                    }
                    status = 201;
                    let name = body["name"].as_str().unwrap();
                    let mut object = json!({"apiVersion":"core.orka.ai/v1alpha1","kind":if request.path=="agents" {"Agent"}else{"Task"},"metadata":{"name":name,"namespace":"orka-system","uid":format!("uid-{name}"),"generation":1},"spec":body["spec"],"status":{"phase":"Succeeded"}});
                    if request.path == "tasks" {
                        object["spec"]["timeout"] = json!("2m0s");
                        object["spec"]["retryPolicy"]
                            .as_object_mut()
                            .unwrap()
                            .remove("maxRetries");
                    }
                    self.objects
                        .insert(format!("{}/{name}", request.path), object.clone());
                    object
                } else if request.path.ends_with("/result") {
                    json!({"result":ANSWER})
                } else if request.path.ends_with("/events") {
                    let name = request.path.split('/').nth(1).unwrap();
                    let agent = self
                        .objects
                        .values()
                        .find(|o| o["kind"] == "Agent")
                        .unwrap()["metadata"]["name"]
                        .clone();
                    let after = request.after.unwrap();
                    let events = if self.missing_tools {
                        vec![]
                    } else {
                        vec![
                            json!({"namespace":"orka-system","streamType":"task","streamID":name,"seq":after+1,"type":"ToolCallCompleted","taskName":name,"agentName":agent,"toolName":"web_fetch","toolCallID":"call-1"}),
                        ]
                    };
                    json!({"namespace":"orka-system","streamType":"task","streamID":name,"afterSeq":after,"latestSeq":if self.missing_tools{after}else{after+1},"events":events})
                } else {
                    self.objects[&request.path].clone()
                };
                self.output = serde_json::to_vec(&json!({"status":status,"body":body}))?;
                ResultBody::Operation {
                    operation,
                    info: OperationInfo {
                        state: OperationState::Completed,
                        exit: Some(0),
                        bytes: self.output.len(),
                        has_diagnostics: false,
                    },
                }
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
        self.input = std::io::Cursor::new(data);
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
struct Session(VmClient, Arc<AtomicBool>);
impl LocalSession for Session {
    fn with_client(
        &mut self,
        work: &mut dyn FnMut(Uuid, &mut VmClient) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        work(Uuid::new_v4(), &mut self.0)
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
        self.1.store(true, Ordering::SeqCst);
        Ok(())
    }
}
struct Factory(Mutex<Option<Session>>);
impl LocalFactory for Factory {
    fn launch(
        &self,
        _: watch::Receiver<bool>,
        _: Arc<dyn Fn(LocalVmStatus) + Send + Sync>,
    ) -> StartFuture<'_> {
        Box::pin(async {
            Ok(Box::new(self.0.lock().unwrap().take().unwrap()) as Box<dyn LocalSession>)
        })
    }
}
async fn fixture(
    missing_tools: bool,
    lost_task: bool,
) -> (Arc<LocalVm>, Arc<Mutex<Vec<Value>>>, Arc<AtomicBool>) {
    let id = Uuid::new_v4();
    let posts = Arc::new(Mutex::new(Vec::new()));
    let stopped = Arc::new(AtomicBool::new(false));
    let client = VmClient::established(
        Box::new(Channel {
            input: std::io::Cursor::new(Vec::new()),
            objects: HashMap::new(),
            output: Vec::new(),
            missing_tools,
            posts: posts.clone(),
            lost_task,
        }),
        id,
    )
    .unwrap();
    let vm = LocalVm::new(Arc::new(Factory(Mutex::new(Some(Session(
        client,
        stopped.clone(),
    ))))));
    vm.start();
    for _ in 0..100 {
        if vm.status() == LocalVmStatus::Ready {
            return (vm, posts, stopped);
        }
        tokio::task::yield_now().await;
    }
    panic!("VM never ready")
}
async fn finished(run: &ExampleRun, vm: &LocalVm) -> bridge_engine::example_run::RunSnapshot {
    for _ in 0..500 {
        let status = run.status(vm);
        if status.state != RunState::Running {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("Run never finished")
}
#[tokio::test]
async fn example_uses_one_client_exact_pair_and_retains_receipts_after_waiter_is_gone() {
    let (vm, posts, stopped) = fixture(false, false).await;
    let run = ExampleRun::new(tokio::runtime::Handle::current());
    assert_eq!(run.start(vm.clone()).state, RunState::Running);
    run.start(vm.clone()); // duplicate renderer command cannot submit twice
    let result = finished(&run, &vm).await;
    let report = result.report.unwrap();
    assert_eq!(report.verdict, bridge_engine::engine::run::Verdict::Passed);
    assert_eq!(report.cases[0].answer.as_deref(), Some(ANSWER));
    assert_eq!(report.cases[0].completed_tools, ["web_fetch"]);
    assert_eq!(
        report.cases[0]
            .assertions
            .iter()
            .map(|a| a.kind)
            .collect::<Vec<_>>(),
        ["contains", "toolCalled"]
    );
    assert_eq!(result.acknowledged.len(), 2);
    {
        let writes = posts.lock().unwrap();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0]["spec"]["model"]["name"], "gpt-5.6-terra");
        assert_eq!(writes[0]["spec"]["providerRef"]["name"], "copilot");
        assert!(!writes[0].to_string().contains(ANSWER));
    }
    vm.shutdown().await.unwrap();
    assert!(stopped.load(Ordering::SeqCst));
}
#[test]
fn blocking_ipc_caller_does_not_need_an_entered_tokio_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let run = ExampleRun::new(runtime.handle().clone());
    let vm = LocalVm::unavailable();
    assert_eq!(run.start(vm.clone()).state, RunState::Running);
    let result = runtime.block_on(finished(&run, &vm));
    assert_eq!(result.state, RunState::Unknown);
    assert_eq!(result.error, Some("session"));
}
#[tokio::test]
async fn absent_completed_tools_stays_unknown_with_contains_passed() {
    let (vm, _, _) = fixture(true, false).await;
    let run = ExampleRun::new(tokio::runtime::Handle::current());
    run.start(vm.clone());
    let result = finished(&run, &vm).await;
    let report = result.report.unwrap();
    assert_eq!(report.verdict, bridge_engine::engine::run::Verdict::Unknown);
    assert_eq!(
        report.cases[0].assertions[0].verdict,
        bridge_engine::engine::run::Verdict::Passed
    );
    assert_eq!(
        report.cases[0].assertions[1].verdict,
        bridge_engine::engine::run::Verdict::Unknown
    );
    vm.shutdown().await.unwrap();
}
#[tokio::test]
async fn uncertain_task_write_keeps_agent_and_pending_task_without_retry() {
    let (vm, posts, _) = fixture(false, true).await;
    let run = ExampleRun::new(tokio::runtime::Handle::current());
    run.start(vm.clone());
    let result = finished(&run, &vm).await;
    assert_eq!(result.state, RunState::Unknown);
    assert_eq!(result.acknowledged.len(), 1);
    assert_eq!(result.pending.unwrap().kind, "Task");
    assert_eq!(posts.lock().unwrap().len(), 2);
    vm.shutdown().await.unwrap();
}
