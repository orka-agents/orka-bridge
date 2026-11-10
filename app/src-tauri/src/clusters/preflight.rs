//! GET-only evidence shared by preview and Start; no model catalogue or Tool copying.
use super::{
    client::{continuation, items},
    *,
};
use crate::engine::run::{ParsedPair, ToolKind};
use bridge_runtime_protocol::SnapshotId;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckStatus {
    Allowed,
    Blocked,
    CannotCheck,
    Unverified,
}
#[derive(Serialize)]
pub struct CheckRow {
    pub label: String,
    pub status: CheckStatus,
    pub message: String,
}
impl CheckRow {
    pub(crate) fn new(
        label: impl Into<String>,
        status: CheckStatus,
        message: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            status,
            message: message.into(),
        }
    }
    pub(crate) fn permits(&self) -> bool {
        matches!(self.status, CheckStatus::Allowed | CheckStatus::Unverified)
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendCheck {
    pub source: Option<SnapshotId>,
    pub target_id: String,
    pub provider_id: String,
    pub model: Option<String>,
    pub rows: Vec<CheckRow>,
    pub can_send: bool,
}
impl SendCheck {
    pub(crate) fn finish(mut self) -> Self {
        self.can_send = !self.rows.is_empty() && self.rows.iter().all(CheckRow::permits);
        self
    }
}
// Check the fields this native profile actually sends or reads, not an image pin
// or a full-schema equality. Optional source features are not made universal gates.
pub(super) fn supports_schema(resource: &str, schema: &Value) -> bool {
    if schema["type"] != "object" {
        return false;
    }
    let required: &[(&str, &str)] = match resource {
        "agents" => &[
            ("spec.model.name", "string"),
            ("spec.providerRef.name", "string"),
            ("spec.systemPrompt.inline", "string"),
            ("spec.tools[].name", "string"),
        ],
        "tasks" => &[
            ("spec.type", "string"),
            ("spec.agentRef.name", "string"),
            ("spec.prompt", "string"),
            ("spec.timeout", "string"),
            ("spec.retryPolicy.maxRetries", "integer"),
        ],
        "providers" => &[("status.ready", "boolean")],
        // Kubernetes validates ObjectMeta implicitly, even when omitted here.
        "tools" => &[],
        _ => return false,
    };
    required.iter().all(|(path, kind)| {
        let mut node = schema;
        for component in path.split('.') {
            if node["type"] != "object" {
                return false;
            }
            let name = component.strip_suffix("[]").unwrap_or(component);
            node = &node["properties"][name];
            if component.ends_with("[]") {
                if node["type"] != "array" {
                    return false;
                }
                node = &node["items"];
            }
        }
        node["type"] == *kind
    }) && (resource != "tasks"
        || schema["properties"]["spec"]["properties"]["type"]
            .get("enum")
            .is_none_or(|v| v.as_array().is_some_and(|a| a.iter().any(|v| v == "ai"))))
}

#[derive(Default)]
struct Catalogue {
    builtins: HashSet<String>,
    resources: HashSet<String>,
}
impl RemoteAccess {
    pub(crate) async fn check(
        &self,
        stream: &OrkaStreamTransport,
        provider: ProviderId,
        pair: &ParsedPair,
    ) -> Vec<CheckRow> {
        use CheckStatus::*;
        if self.recheck_reads(stream).await.is_err() {
            return vec![CheckRow::new(
                "Access",
                CannotCheck,
                "Bridge cannot check the selected access. Reconnect in Clusters.",
            )];
        }
        let mut rows = vec![CheckRow::new(
            "Access",
            Allowed,
            "The selected identity can read this Orka. Send still needs permission to create an Agent and checks.",
        )];
        let selected = self.providers().iter().find(|p| p.id == provider);
        let (status, message) = match self.configured_providers(stream).await {
            Err(_) => (CannotCheck, "Bridge cannot check the selected Provider."),
            Ok(current) => {
                match selected.and_then(|p| current.iter().find(|(name, _)| *name == p.name)) {
                    Some((_, true)) => (Allowed, "The selected Provider is configured and ready."),
                    Some((_, false)) => (Blocked, "The selected Provider is not ready."),
                    None => (Blocked, "Select a configured Provider in Clusters."),
                }
            }
        };
        rows.push(CheckRow::new("Provider", status, message));
        rows.push(CheckRow::new(
            "Model",
            Unverified,
            "The file's exact model will be tested here; runtime availability is not checked yet.",
        ));
        let mut catalog = Catalogue::default();
        let readable = self.catalogue(stream, pair, &mut catalog).await.is_ok();
        for tool in pair.declared_tools() {
            let builtin = catalog.builtins.contains(&tool.name);
            let resource = catalog.resources.contains(&tool.name);
            let (status, message) = match tool.kind {
                ToolKind::Builtin if builtin => (Allowed, "Built in to this Orka.".into()),
                _ if !readable => (
                    CannotCheck,
                    format!(
                        "Bridge cannot check {} on {}.",
                        tool.name,
                        self.binding().context_name
                    ),
                ),
                ToolKind::Builtin => (Blocked, "This built-in is not listed by this Orka.".into()),
                ToolKind::Resource if !resource => (
                    Blocked,
                    format!(
                        "Missing Tool {}; create it on {}.",
                        tool.name,
                        self.binding().context_name
                    ),
                ),
                ToolKind::Resource if builtin => (
                    Unverified,
                    format!(
                        "{} has the same name as a built-in (implementation unverified).",
                        tool.name
                    ),
                ),
                ToolKind::Resource => (
                    Allowed,
                    "Tool exists in the watched namespace; its endpoint and policy will be tested."
                        .into(),
                ),
            };
            rows.push(CheckRow::new(&tool.name, status, message));
        }
        rows
    }
    async fn catalogue(
        &self,
        stream: &OrkaStreamTransport,
        pair: &ParsedPair,
        catalog: &mut Catalogue,
    ) -> Result<(), AccessOutcome> {
        let required = pair.declared_tools();
        let resources = required.iter().any(|t| t.kind == ToolKind::Resource);
        if required.is_empty() {
            return Ok(());
        }
        let mut after = None;
        let mut seen = HashSet::new();
        loop {
            let page = self
                .read_via(stream, OrkaResource::Tools, after.as_deref())
                .await?;
            for item in items(&page)? {
                let name = item["name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .ok_or(AccessOutcome::Unavailable)?;
                match item["builtin"].as_bool() {
                    Some(true) => {
                        catalog.builtins.insert(name.into());
                    }
                    Some(false) if !resources => {}
                    Some(false) if item["namespace"] == self.binding().watched_namespace => {
                        catalog.resources.insert(name.into());
                    }
                    _ => return Err(AccessOutcome::Unavailable),
                }
            }
            // Positive built-in evidence needs no unrelated resource pages.
            // Resources exhaust the catalogue, including later collision evidence.
            if !resources && required.iter().all(|t| catalog.builtins.contains(&t.name)) {
                return Ok(());
            }
            after = continuation(&page)?;
            if after.is_none() {
                return Ok(());
            }
            if !seen.insert(after.clone()) {
                return Err(AccessOutcome::Unavailable);
            }
        }
    }
}
