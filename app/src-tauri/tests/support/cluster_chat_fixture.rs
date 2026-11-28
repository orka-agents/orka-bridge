//! A fake Orka API over real HTTP for cluster conversations: the request shapes and replies
//! Orka v0.4.0 gave a real kind cluster (create 201, delete 204, missing 404 with an error
//! object, events with a cursor, Sessions with a JSONL transcript).
use bridge_engine::chat::ChatError;
use bridge_engine::chat_cluster::{ClusterOpen, ClusterWire, WireFuture, WireRequest};
use bridge_runtime_protocol::ChatMethod;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub const NAMESPACE: &str = "team-a";
pub const PROVIDER: &str = "fixture";

#[derive(Default)]
pub struct Orka {
    /// Every request line, in order, as `METHOD /api/v1/...?...`.
    pub log: Vec<String>,
    pub bodies: Vec<Value>,
    objects: HashMap<String, Value>,
    reads: HashMap<String, u32>,
    /// `METHOD collection` pairs refused with 403, as a narrower role would.
    pub deny: HashSet<String>,
    /// Keep Tasks Running until deleted, so Stop has something to stop.
    pub hold: bool,
    /// Drop `sessionRef` from created Tasks, as an Orka without Sessions would.
    pub no_sessions: bool,
    pub opens: u32,
    pub closes: u32,
    /// Refuse this many opens first, as a busy kubeconfig slot would.
    pub refuse_opens: u32,
    /// Hold Session reads until notified, after the Task has finished: the last moment of a turn.
    pub session_gate: Option<Arc<tokio::sync::Notify>>,
    pub session_waiting: bool,
}
impl Orka {
    pub fn requests(&self, method: &str) -> Vec<&String> {
        self.log
            .iter()
            .filter(|line| line.starts_with(&format!("{method} ")))
            .collect()
    }
    fn reply(&mut self, method: &str, uri: &str, body: &[u8]) -> (u16, Value) {
        self.log.push(format!("{method} {uri}"));
        let (path, query) = uri
            .strip_prefix("/api/v1/")
            .and_then(|rest| rest.split_once('?'))
            .expect("an Orka API request line");
        assert!(
            query.starts_with(&format!("namespace={NAMESPACE}")),
            "every request names the watched namespace: {uri}"
        );
        let parts: Vec<_> = path.split('/').collect();
        if self.deny.contains(&format!("{method} {}", parts[0])) {
            return (
                403,
                json!({"error":{"code":403,"message":"not authorized"}}),
            );
        }
        let missing = |what: &str| {
            (
                404,
                json!({"error":{"code":404,"message":format!("{what} not found")}}),
            )
        };
        match (method, parts.as_slice()) {
            ("POST", [collection]) => {
                let b: Value = serde_json::from_slice(body).unwrap();
                self.bodies.push(b.clone());
                assert_eq!(b["namespace"], NAMESPACE);
                let name = b["name"].as_str().unwrap().to_owned();
                let kind = if *collection == "agents" {
                    "Agent"
                } else {
                    "Task"
                };
                let mut object = json!({"apiVersion":"core.orka.ai/v1alpha1","kind":kind,
                    "metadata":{"name":name,"namespace":NAMESPACE,"uid":format!("uid-{name}"),"generation":1},
                    "spec":b["spec"]});
                if *collection == "tasks" {
                    object["spec"]["timeout"] = json!("2m0s");
                    object["spec"]["retryPolicy"] = json!({});
                    object["status"] = json!({});
                    if self.no_sessions {
                        object["spec"].as_object_mut().unwrap().remove("sessionRef");
                    } else {
                        let session = b["spec"]["sessionRef"]["name"].as_str().unwrap();
                        let entry = self.objects.entry(format!("sessions/{session}")).or_insert_with(
                            || json!({"name":session,"namespace":NAMESPACE,"activeTask":"","transcript":""}),
                        );
                        entry["activeTask"] = json!(name);
                    }
                }
                self.objects
                    .insert(format!("{collection}/{name}"), object.clone());
                (201, object)
            }
            ("GET", ["providers", name]) if *name == PROVIDER => (
                200,
                json!({"metadata":{"name":PROVIDER,"namespace":NAMESPACE},"spec":{"type":"openai","defaultModel":"fixture-model"}}),
            ),
            ("GET", ["tasks", name]) => {
                let key = format!("tasks/{name}");
                let Some(mut task) = self.objects.get(&key).cloned() else {
                    return missing("task");
                };
                let reads = self.reads.entry(key.clone()).or_default();
                *reads += 1;
                let phase = if self.hold || *reads < 2 {
                    "Running"
                } else {
                    "Succeeded"
                };
                task["status"] = json!({"phase":phase});
                if phase == "Succeeded" {
                    self.finish(name, &task);
                }
                (200, task)
            }
            ("GET", ["tasks", name, "result"]) => {
                if !self.objects.contains_key(&format!("tasks/{name}")) {
                    return missing("task");
                }
                if self
                    .reads
                    .get(&format!("tasks/{name}"))
                    .copied()
                    .unwrap_or(0)
                    < 2
                    || self.hold
                {
                    return (
                        404,
                        json!({"error":{"code":404,"message":"task has no result"}}),
                    );
                }
                (200, json!({"result":format!("Answer for {name}")}))
            }
            ("GET", ["tasks", name, "events"]) => {
                let Some(task) = self.objects.get(&format!("tasks/{name}")) else {
                    return missing("task");
                };
                let agent = task["spec"]["agentRef"]["name"].clone();
                let after: u64 = query
                    .split('&')
                    .find_map(|p| p.strip_prefix("after="))
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!(query.ends_with("&limit=1000"));
                let all = [
                    json!({"namespace":NAMESPACE,"streamType":"task","streamID":name,"seq":1,"type":"ToolCallStarted","taskName":name,"agentName":agent,"toolName":"web_fetch","toolCallID":"call-1"}),
                    json!({"namespace":NAMESPACE,"streamType":"task","streamID":name,"seq":2,"type":"ToolCallCompleted","taskName":name,"agentName":agent,"toolName":"web_fetch","toolCallID":"call-1"}),
                ];
                let events: Vec<_> = all.into_iter().skip(after as usize).collect();
                (
                    200,
                    json!({"namespace":NAMESPACE,"streamType":"task","streamID":name,"afterSeq":after,"latestSeq":2,"events":events}),
                )
            }
            ("GET", ["agents", _] | ["sessions", _]) => match self.objects.get(path) {
                Some(object) => (200, object.clone()),
                None => missing(parts[0].trim_end_matches('s')),
            },
            ("DELETE", [collection, name]) => {
                if self.objects.remove(path).is_none() {
                    return missing(collection.trim_end_matches('s'));
                }
                // Deleting a Task releases its Session, as the controller does.
                if *collection == "tasks" {
                    for (key, session) in self.objects.iter_mut() {
                        if key.starts_with("sessions/") && session["activeTask"] == *name {
                            session["activeTask"] = json!("");
                        }
                    }
                }
                (204, Value::Null)
            }
            _ => (404, json!({"error":{"code":404,"message":"Not Found"}})),
        }
    }
    /// The controller appends both messages to the Session and releases it.
    fn finish(&mut self, name: &str, task: &Value) {
        let Some(session) = task["spec"]["sessionRef"]["name"].as_str() else {
            return;
        };
        let uid = task["metadata"]["uid"].as_str().unwrap();
        let entry = self
            .objects
            .get_mut(&format!("sessions/{session}"))
            .unwrap();
        if entry["activeTask"] != name {
            return;
        }
        let mut text = entry["transcript"].as_str().unwrap().to_owned();
        for (role, content) in [
            ("user", task["spec"]["prompt"].clone()),
            ("assistant", json!(format!("Answer for {name}"))),
        ] {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(
                &json!({"id":format!("task:{uid}:{role}"),"role":role,"content":content})
                    .to_string(),
            );
        }
        entry["transcript"] = json!(text);
        entry["activeTask"] = json!("");
    }
}

/// Serves the fake on a loopback port, one request per connection.
pub async fn serve() -> (Arc<Mutex<Orka>>, Arc<dyn ClusterOpen>) {
    let orka = Arc::new(Mutex::new(Orka::default()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = orka.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let state = state.clone();
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let (head, body) = loop {
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8(raw[..end].to_vec()).unwrap();
                        let length = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        while raw.len() < end + 4 + length {
                            let n = socket.read(&mut buf).await.unwrap();
                            raw.extend_from_slice(&buf[..n]);
                        }
                        break (head, raw[end + 4..end + 4 + length].to_vec());
                    }
                };
                let line: Vec<_> = head.lines().next().unwrap().split(' ').collect();
                assert!(
                    head.contains("authorization: Bearer synthetic")
                        || head.contains("Authorization: Bearer synthetic")
                );
                let (status, value) = state.lock().unwrap().reply(line[0], line[1], &body);
                let payload = if value.is_null() {
                    Vec::new()
                } else {
                    serde_json::to_vec(&value).unwrap()
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.write_all(&payload).await;
            });
        }
    });
    (orka.clone(), Arc::new(Opener { port, orka }))
}
struct Opener {
    port: u16,
    orka: Arc<Mutex<Orka>>,
}
impl ClusterOpen for Opener {
    fn open(&self) -> WireFuture<'_, Result<Arc<dyn ClusterWire>, ChatError>> {
        {
            let mut orka = self.orka.lock().unwrap();
            if orka.refuse_opens > 0 {
                orka.refuse_opens -= 1;
                return Box::pin(async { Err(ChatError::ClusterUnavailable) });
            }
            orka.opens += 1;
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap();
        let wire = Wire {
            client,
            port: self.port,
            orka: self.orka.clone(),
        };
        Box::pin(async move { Ok(Arc::new(wire) as Arc<dyn ClusterWire>) })
    }
}
struct Wire {
    client: reqwest::Client,
    port: u16,
    orka: Arc<Mutex<Orka>>,
}
impl ClusterWire for Wire {
    fn exchange(&self, request: WireRequest) -> WireFuture<'_, Result<(u16, Vec<u8>), ChatError>> {
        Box::pin(async move {
            let gate = request
                .uri
                .starts_with("/api/v1/sessions/")
                .then(|| self.orka.lock().unwrap().session_gate.clone())
                .flatten();
            if let Some(gate) = gate {
                self.orka.lock().unwrap().session_waiting = true;
                gate.notified().await;
            }
            let method = match request.method {
                ChatMethod::Get => reqwest::Method::GET,
                ChatMethod::Post => reqwest::Method::POST,
                ChatMethod::Delete => reqwest::Method::DELETE,
            };
            let mut http = self
                .client
                .request(
                    method,
                    format!("http://127.0.0.1:{}{}", self.port, request.uri),
                )
                .bearer_auth("synthetic");
            if let Some(body) = request.body {
                http = http.header("Content-Type", "application/json").body(body);
            }
            let response = http.send().await.map_err(|_| ChatError::Connection)?;
            let status = response.status().as_u16();
            let bytes = response.bytes().await.map_err(|_| ChatError::Connection)?;
            Ok((status, bytes.to_vec()))
        })
    }
    fn close(&self) -> WireFuture<'_, ()> {
        self.orka.lock().unwrap().closes += 1;
        Box::pin(async {})
    }
}
