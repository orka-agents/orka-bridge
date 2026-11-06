//! Native-only authoring API. Not a renderer command or guest protocol extension.
#[path = "run_source.rs"]
mod source;
pub use source::{Assertion, ParsedPair, RunCase};
#[path = "run_http.rs"]
mod http;
use bridge_runtime_protocol::{SnapshotId, Uuid};
pub use http::{NativeRunTarget, RunLimits};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt;
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Source,
    Configuration,
    Transport,
    Deadline,
    Response,
    Identity,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Passed,
    Failed,
    Unknown,
}
#[derive(Clone, Debug)]
pub struct AssertionReport {
    pub id: String,
    pub verdict: Verdict,
}
#[derive(Debug)]
pub struct CaseReport {
    pub id: String,
    pub phase: String,
    pub answer: Option<String>,
    /// Names from attributable completed calls, not a complete URL/body trace.
    pub completed_tools: Vec<String>,
    pub assertions: Vec<AssertionReport>,
    pub verdict: Verdict,
}
#[derive(Debug)]
pub struct RunReport {
    pub cases: Vec<CaseReport>,
    pub verdict: Verdict,
}
fn aggregate(rows: impl Iterator<Item = Verdict>) -> Verdict {
    let rows: Vec<_> = rows.collect();
    if rows.contains(&Verdict::Failed) {
        Verdict::Failed
    } else if rows.is_empty() || rows.contains(&Verdict::Unknown) {
        Verdict::Unknown
    } else {
        Verdict::Passed
    }
}
#[derive(Clone)]
pub struct ResourceId {
    pub name: String,
    pub namespace: String,
    pub uid: String,
    pub generation: u64,
    spec: Value,
    kind: &'static str,
}
impl ResourceId {
    fn acknowledge(
        value: &Value,
        requested: &Value,
        name: &str,
        namespace: &str,
        kind: &'static str,
    ) -> Result<Self, ErrorKind> {
        let metadata = &value["metadata"];
        let uid = metadata["uid"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(ErrorKind::Identity)?;
        let generation = metadata["generation"].as_u64().unwrap_or(0);
        let mut requested = requested.clone();
        if kind == "Task" {
            // Go serializes this fixed 120-second duration canonically and omits
            // MaxRetries=0. Still refuse any actual retry authority in the reply.
            if requested["timeout"] == "120s" {
                requested["timeout"] = json!("2m0s");
            }
            if value["spec"]["retryPolicy"]
                .get("maxRetries")
                .is_some_and(|v| v != 0)
            {
                return Err(ErrorKind::Identity);
            }
            if let Some(policy) = requested["retryPolicy"].as_object_mut() {
                policy.remove("maxRetries");
            }
        }
        // Create handlers omit TypeMeta; the fixed POST endpoint supplies its
        // type. An explicit conflicting type is never accepted.
        if value
            .get("apiVersion")
            .is_some_and(|v| v != "core.orka.ai/v1alpha1")
            || value.get("kind").is_some_and(|v| v != kind)
            || metadata["name"] != name
            || metadata["namespace"] != namespace
            || kind == "Agent" && generation == 0
            || !includes(&value["spec"], &requested)
        {
            return Err(ErrorKind::Identity);
        }
        Ok(Self {
            name: name.into(),
            namespace: namespace.into(),
            uid: uid.into(),
            generation,
            spec: value["spec"].clone(),
            kind,
        })
    }
    fn unchanged(&self, value: &Value) -> Result<(), ErrorKind> {
        if value["apiVersion"] != "core.orka.ai/v1alpha1" || value["kind"] != self.kind {
            return Err(ErrorKind::Identity);
        }
        let current = Self::acknowledge(value, &self.spec, &self.name, &self.namespace, self.kind)?;
        if current.uid != self.uid
            || current.generation != self.generation
            || current.spec != self.spec
        {
            return Err(ErrorKind::Identity);
        }
        Ok(())
    }
    fn path(&self) -> String {
        format!(
            "{}/{}",
            if self.kind == "Agent" {
                "agents"
            } else {
                "tasks"
            },
            self.name
        )
    }
}
fn includes(value: &Value, requested: &Value) -> bool {
    match (value, requested) {
        (Value::Object(value), Value::Object(requested)) => requested
            .iter()
            .all(|(k, v)| value.get(k).is_some_and(|got| includes(got, v))),
        (Value::Array(value), Value::Array(requested)) => {
            value.len() == requested.len()
                && value.iter().zip(requested).all(|(a, b)| includes(a, b))
        }
        _ => value == requested,
    }
}
/// An in-memory receipt, tied to this runner. Clone and retain it before observing.
#[derive(Clone)]
pub struct RunHandle {
    pair: ParsedPair,
    target: Uuid,
    agent: Option<ResourceId>,
    tasks: Vec<ResourceId>,
}
impl RunHandle {
    pub fn snapshot(&self) -> &SnapshotId {
        self.pair.snapshot()
    }
    pub fn agent(&self) -> Option<&ResourceId> {
        self.agent.as_ref()
    }
    pub fn tasks(&self) -> &[ResourceId] {
        &self.tasks
    }
}
impl fmt::Debug for RunHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunHandle")
            .field("acknowledged_tasks", &self.tasks.len())
            .finish_non_exhaustive()
    }
}
/// This generated name may have been created even when no response was received.
#[derive(Clone)]
pub struct PendingMutation {
    pub kind: &'static str,
    pub name: String,
}
pub struct RunError {
    pub kind: ErrorKind,
    pub handle: Box<RunHandle>,
    pub pending: Option<PendingMutation>,
}
impl fmt::Debug for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Native Run error: {:?}", self.kind)
    }
}
impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
impl std::error::Error for RunError {}

pub struct NativeRunner {
    http: http::Http,
    id: Uuid,
}
impl NativeRunner {
    pub fn new(target: NativeRunTarget, limits: RunLimits) -> Result<Self, ErrorKind> {
        Ok(Self {
            http: http::Http::new(target, limits)?,
            id: Uuid::new_v4(),
        })
    }
    /// No POST retry, replacement, adoption or automatic deletion. Dropping this
    /// future cannot stop remote work; a failed write retains its attempted name.
    pub async fn submit(&self, pair: ParsedPair) -> Result<RunHandle, RunError> {
        let mut handle = RunHandle {
            pair,
            target: self.id,
            agent: None,
            tasks: Vec::new(),
        };
        let mut pending = None;
        let result = self.submit_inner(&mut handle, &mut pending).await;
        match result {
            Ok(()) => Ok(handle),
            Err(kind) => Err(RunError {
                kind,
                handle: Box::new(handle),
                pending,
            }),
        }
    }
    async fn submit_inner(
        &self,
        handle: &mut RunHandle,
        pending: &mut Option<PendingMutation>,
    ) -> Result<(), ErrorKind> {
        let deadline = self.http.deadline();
        let namespace = &self.http.target.namespace;
        let name = format!("bridge-run-{}", Uuid::new_v4().simple());
        let spec = json!({"providerRef":{"name":self.http.target.provider},"model":{"name":handle.pair.model()},"systemPrompt":{"inline":handle.pair.prompt()},"tools":handle.pair.tools().iter().map(|name| json!({"name":name})).collect::<Vec<_>>()});
        *pending = Some(PendingMutation {
            kind: "Agent",
            name: name.clone(),
        });
        let value = self
            .http
            .request(
                "agents",
                None,
                Some(&json!({"name":name,"namespace":namespace,"spec":spec})),
                deadline,
            )
            .await?;
        handle.agent = Some(ResourceId::acknowledge(
            &value, &spec, &name, namespace, "Agent",
        )?);
        *pending = None;
        for case in handle.pair.cases() {
            let task = format!("bridge-task-{}", Uuid::new_v4().simple());
            let spec = json!({"type":"ai","agentRef":{"name":name},"prompt":case.input(),"timeout":"120s","retryPolicy":{"maxRetries":0}});
            *pending = Some(PendingMutation {
                kind: "Task",
                name: task.clone(),
            });
            let value = self
                .http
                .request(
                    "tasks",
                    None,
                    Some(&json!({"name":task,"namespace":namespace,"spec":spec})),
                    deadline,
                )
                .await?;
            handle.tasks.push(ResourceId::acknowledge(
                &value, &spec, &task, namespace, "Task",
            )?);
            *pending = None;
        }
        Ok(())
    }
    /// GET-only, finite observation. Timeout/drop is not remote cancellation.
    /// Partial receipts can read already acknowledged tasks without replaying writes.
    pub async fn observe(&self, handle: &RunHandle) -> Result<RunReport, RunError> {
        self.observe_inner(handle).await.map_err(|kind| RunError {
            kind,
            handle: Box::new(handle.clone()),
            pending: None,
        })
    }
    async fn checked(&self, resource: &ResourceId, deadline: Instant) -> Result<Value, ErrorKind> {
        let value = self
            .http
            .request(&resource.path(), None, None, deadline)
            .await?;
        resource.unchanged(&value)?;
        Ok(value)
    }
    async fn observe_inner(&self, handle: &RunHandle) -> Result<RunReport, ErrorKind> {
        if handle.target != self.id {
            return Err(ErrorKind::Configuration);
        }
        let agent = handle.agent.as_ref().ok_or(ErrorKind::Identity)?;
        let deadline = self.http.deadline();
        let mut cases = Vec::new();
        for (case, task) in handle.pair.cases().iter().zip(&handle.tasks) {
            let mut terminal_rounds = 0;
            loop {
                self.checked(agent, deadline).await?;
                let value = self.checked(task, deadline).await?;
                let phase = value["status"]["phase"].as_str().unwrap_or("");
                if matches!(phase, "Succeeded" | "Failed" | "Cancelled") {
                    terminal_rounds += 1;
                    let result = self
                        .http
                        .request(&format!("{}/result", task.path()), None, None, deadline)
                        .await
                        .ok();
                    let answer = result
                        .as_ref()
                        .and_then(|v| v["result"].as_str())
                        .map(str::to_owned);
                    // Event recording is best effort. Any untrusted/unavailable drain
                    // loses tool proof, never turns missing evidence into a failure.
                    let tools = self.tools(task, agent, deadline).await.unwrap_or_default();
                    self.checked(task, deadline).await?;
                    self.checked(agent, deadline).await?;
                    let assertions = case.evaluate(answer.as_deref(), &tools);
                    let verdict = if phase == "Succeeded" {
                        aggregate(assertions.iter().map(|r| r.verdict))
                    } else {
                        Verdict::Failed
                    };
                    if verdict != Verdict::Unknown || terminal_rounds == 3 {
                        cases.push(CaseReport {
                            id: case.id.clone(),
                            phase: phase.into(),
                            answer,
                            completed_tools: tools,
                            assertions,
                            verdict,
                        });
                        break;
                    }
                } else if !matches!(
                    phase,
                    "" | "Pending" | "Running" | "Finalizing" | "Scheduled"
                ) {
                    return Err(ErrorKind::Response);
                }
                self.http.pause(deadline).await?;
            }
        }
        self.checked(agent, deadline).await?;
        let verdict = if handle.tasks.len() == handle.pair.cases().len() {
            aggregate(cases.iter().map(|c| c.verdict))
        } else {
            Verdict::Unknown
        };
        Ok(RunReport { cases, verdict })
    }
    async fn tools(
        &self,
        task: &ResourceId,
        agent: &ResourceId,
        deadline: Instant,
    ) -> Result<Vec<String>, ErrorKind> {
        let mut after = 0;
        let mut tools = Vec::new();
        for _ in 0..16 {
            let value = self
                .http
                .request(
                    &format!("{}/events", task.path()),
                    Some(after),
                    None,
                    deadline,
                )
                .await?;
            let page: EventPage = serde_json::from_value(value).map_err(|_| ErrorKind::Response)?;
            if page.namespace != task.namespace
                || page.stream_type != "task"
                || page.stream_id != task.name
                || page.after_seq != after
                || page.latest_seq < after
                || page.events.len() > 1000
            {
                return Err(ErrorKind::Identity);
            }
            for event in page.events {
                if event.seq != after + 1
                    || event.seq > page.latest_seq
                    || event.namespace != task.namespace
                    || event.stream_type != "task"
                    || event.stream_id != task.name
                    || event
                        .task_name
                        .as_ref()
                        .is_some_and(|s| !s.is_empty() && s != &task.name)
                    || event
                        .agent_name
                        .as_ref()
                        .is_some_and(|s| !s.is_empty() && s != &agent.name)
                {
                    return Err(ErrorKind::Identity);
                }
                after = event.seq;
                if event.kind == "ToolCallCompleted"
                    && !event.tool_name.trim().is_empty()
                    && !event.tool_call_id.trim().is_empty()
                {
                    tools.push(event.tool_name);
                }
            }
            if after == page.latest_seq {
                return Ok(tools);
            }
            if after == page.after_seq {
                return Err(ErrorKind::Response);
            }
        }
        Err(ErrorKind::Response)
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventPage {
    namespace: String,
    stream_type: String,
    #[serde(rename = "streamID")]
    stream_id: String,
    after_seq: u64,
    latest_seq: u64,
    events: Vec<Event>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    namespace: String,
    stream_type: String,
    #[serde(rename = "streamID")]
    stream_id: String,
    seq: u64,
    #[serde(rename = "type")]
    kind: String,
    task_name: Option<String>,
    agent_name: Option<String>,
    #[serde(default)]
    tool_name: String,
    #[serde(default, rename = "toolCallID")]
    tool_call_id: String,
}
