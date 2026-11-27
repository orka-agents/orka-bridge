use crate::{orka::generated, ProtocolError, MAX_ORKA_BODY, ORKA_NAMESPACE};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatMethod {
    Get,
    Post,
    Delete,
}
/// The generated names one kind of conversation owns: its Agents, its Tasks and its Session references.
/// Each name is the prefix followed by 32 lowercase hex digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatFamily {
    pub agent: &'static str,
    pub task: &'static str,
    pub session: &'static str,
}
/// Designer's Try a task conversations.
pub const CHAT_FAMILY: ChatFamily = ChatFamily {
    agent: "bridge-chat-agent-",
    task: "bridge-chat-task-",
    session: "bridge-chat-",
};
/// The Navigator's conversations on Home. Kept apart from Try a task so neither can
/// reference, read or stop the other's resources, and so lists can tell them apart.
pub const NAVIGATOR_FAMILY: ChatFamily = ChatFamily {
    agent: "bridge-nav-agent-",
    task: "bridge-nav-task-",
    session: "bridge-nav-",
};
const FAMILIES: [ChatFamily; 2] = [CHAT_FAMILY, NAVIGATOR_FAMILY];
impl ChatFamily {
    fn owning(name: &str, prefix: impl Fn(&ChatFamily) -> &'static str) -> Option<Self> {
        FAMILIES
            .into_iter()
            .find(|family| generated(name, prefix(family)))
    }
}
/// Chat has a separate method/path contract; Run never acquires DELETE authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatRequest {
    pub method: ChatMethod,
    pub path: String,
    pub after: Option<u64>,
    pub body: Option<Value>,
}
/// A cluster's watched namespace or Provider name: one lowercase DNS label.
pub fn dns_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}
impl ChatRequest {
    /// The local workspace's contract: the fixed namespace, and no cleanup authority.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.check(ORKA_NAMESPACE, None)
    }
    /// A cluster conversation in its installation's watched namespace. It may also delete its
    /// own generated Agent and Session when a conversation is replaced, and read the one
    /// Provider its AI connection names.
    pub fn validate_cluster(&self, namespace: &str, provider: &str) -> Result<(), ProtocolError> {
        if !dns_label(namespace) || !dns_label(provider) {
            return Err(ProtocolError::Invalid);
        }
        self.check(namespace, Some(provider))
    }
    fn check(&self, namespace: &str, cluster: Option<&str>) -> Result<(), ProtocolError> {
        if self.path.len() > 128 || cluster.is_none() && self.path.len() > 64 {
            return Err(ProtocolError::Invalid);
        }
        let parts: Vec<_> = self.path.split('/').collect();
        let agent = |name: &str| ChatFamily::owning(name, |f| f.agent).is_some();
        let task = |name: &str| ChatFamily::owning(name, |f| f.task).is_some();
        let session = |name: &str| ChatFamily::owning(name, |f| f.session).is_some();
        let (create, events) = match (self.method, parts.as_slice()) {
            (ChatMethod::Post, ["agents"] | ["tasks"]) => (true, false),
            (ChatMethod::Get, ["agents", name]) if agent(name) => (false, false),
            (ChatMethod::Delete, ["agents", name]) if cluster.is_some() && agent(name) => {
                (false, false)
            }
            (ChatMethod::Get | ChatMethod::Delete, ["tasks", name]) if task(name) => (false, false),
            (ChatMethod::Get, ["tasks", name, "result"]) if task(name) => (false, false),
            (ChatMethod::Get, ["tasks", name, "events"]) if task(name) => (false, true),
            (ChatMethod::Get, ["sessions", name]) if session(name) => (false, false),
            (ChatMethod::Delete, ["sessions", name]) if cluster.is_some() && session(name) => {
                (false, false)
            }
            (ChatMethod::Get, ["providers", name]) if cluster == Some(*name) => (false, false),
            _ => return Err(ProtocolError::Invalid),
        };
        if self.after.is_some() != events {
            return Err(ProtocolError::Invalid);
        }
        if !create {
            return if self.body.is_none() {
                Ok(())
            } else {
                Err(ProtocolError::Invalid)
            };
        }
        let body = self
            .body
            .as_ref()
            .and_then(Value::as_object)
            .ok_or(ProtocolError::Invalid)?;
        let name = body.get("name").and_then(Value::as_str).unwrap_or("");
        let family = if parts[0] == "agents" {
            ChatFamily::owning(name, |f| f.agent)
        } else {
            ChatFamily::owning(name, |f| f.task)
        }
        .ok_or(ProtocolError::Invalid)?;
        if body.len() != 3
            || body.get("namespace").and_then(Value::as_str) != Some(namespace)
            || !body.get("spec").is_some_and(Value::is_object)
        {
            return Err(ProtocolError::Invalid);
        }
        if parts[0] == "tasks" {
            // A Task may only reference the Agent and Session of its own family.
            let spec = &body["spec"];
            if spec["type"] != "ai"
                || !spec["agentRef"]["name"]
                    .as_str()
                    .is_some_and(|n| generated(n, family.agent))
                || spec["agentRef"]
                    .get("namespace")
                    .is_some_and(|n| n.as_str() != Some(namespace))
                || !spec["sessionRef"]["name"]
                    .as_str()
                    .is_some_and(|n| generated(n, family.session))
            {
                return Err(ProtocolError::Invalid);
            }
        }
        if serde_json::to_vec(body)
            .map_err(|_| ProtocolError::Invalid)?
            .len()
            > MAX_ORKA_BODY
        {
            return Err(ProtocolError::Oversize);
        }
        Ok(())
    }
}
