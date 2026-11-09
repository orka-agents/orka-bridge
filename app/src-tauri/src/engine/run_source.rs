use super::{AssertionReport, ErrorKind, Verdict};
use bridge_runtime_protocol::{SnapshotId, MAX_FILE};
use serde::Deserialize;
use std::collections::HashSet;

/// Validated immutable source. Assertion values never form part of either prompt.
#[derive(Clone)]
pub struct ParsedPair {
    snapshot: SnapshotId,
    prompt: String,
    model: String,
    tools: Vec<String>,
    cases: Vec<RunCase>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCase {
    pub id: String,
    input: String,
    assertions: Vec<Assertion>,
}
#[derive(Clone, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Assertion {
    #[serde(rename = "contains")]
    Contains { id: String, value: String },
    #[serde(rename = "toolCalled")]
    ToolCalled { id: String, tool: String },
}
impl Assertion {
    pub fn id(&self) -> &str {
        match self {
            Self::Contains { id, .. } | Self::ToolCalled { id, .. } => id,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentHeader {
    name: Option<String>,
    description: Option<String>,
    model: String,
    kmx: Kmx,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Kmx {
    schema_version: u32,
    runtime: String,
    #[serde(default)]
    tools: Vec<Tool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tool {
    kind: String,
    name: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestsHeader {
    schema_version: u32,
}
fn frontmatter(bytes: &[u8]) -> Result<(&str, &str), ErrorKind> {
    if bytes.len() > MAX_FILE {
        return Err(ErrorKind::Source);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ErrorKind::Source)?;
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().ok_or(ErrorKind::Source)?;
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return Err(ErrorKind::Source);
    }
    let mut end = first.len();
    for line in lines {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Ok((&text[first.len()..end], &text[end + line.len()..]));
        }
        end += line.len();
    }
    Err(ErrorKind::Source)
}
fn yaml<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, ErrorKind> {
    serde_yaml_ng::from_str(s).map_err(|_| ErrorKind::Source)
}
fn present(s: &str) -> bool {
    !s.trim().is_empty() && !s.contains('\0')
}
// Name classification, not authorization or target availability. Keep aligned
// with KnownBuiltInToolNames at the Orka revision documented in docs/native-run.md.
fn supported_builtin(name: &str) -> bool {
    matches!(
        name,
        "web_search"
            | "web_fetch"
            | "code_exec"
            | "file_read"
            | "file_write"
            | "request_approval"
            | "reply_in_conversation"
            | "create_ai_task"
            | "create_pr_monitor"
            | "create_container_task"
            | "create_agent_task"
            | "check_task_progress"
            | "fetch_task_output"
            | "wait_for_task"
            | "cancel_task"
            | "list_agents"
            | "list_tools"
            | "list_tasks"
            | "list_connections"
            | "create_agent"
            | "update_agent"
            | "delete_agent"
            | "create_tool"
            | "delete_tool"
            | "delete_session"
            | "delegate_task"
            | "wait_for_tasks"
            | "send_message"
            | "check_messages"
            | "create_pull_request"
            | "check_pull_request_ci"
            | "merge_pull_request"
            | "auto_merge_pull_request"
            | "review_pull_request"
            | "post_review_comment"
            | "check_pr_review_marker"
            | "list_issues"
            | "list_pull_requests"
            | "get_issue"
            | "comment_on_issue"
            | "update_plan"
            | "recall_memory"
            | "remember"
            | "propose_memory"
            | "search_transcript"
    )
}

/// Fence recognition is separate from case YAML decoding. Noncanonical test
/// fences are refused, never silently dropped beside a valid case.
fn test_blocks(body: &str) -> Result<Vec<RunCase>, ErrorKind> {
    let mut cases = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut block: Option<String> = None;
    for line in body.split_inclusive('\n') {
        let marker = line.trim_end_matches(['\r', '\n']);
        let trimmed = marker.trim_start_matches([' ', '\t']);
        let first = trimmed.as_bytes().first().copied();
        let candidate = first.filter(|c| matches!(c, b'`' | b'~')).and_then(|c| {
            let count = trimmed.bytes().take_while(|b| *b == c).count();
            (count >= 3).then_some((c, count, &trimmed[count..]))
        });
        if let Some((character, length)) = fence {
            if candidate
                .is_some_and(|(c, n, rest)| c == character && n >= length && rest.trim().is_empty())
            {
                if let Some(text) = block.take() {
                    cases.push(yaml(&text)?);
                }
                fence = None;
            } else if let Some(text) = &mut block {
                text.push_str(line);
            }
        } else if let Some((character, length, info)) = candidate {
            if info.contains("agent-test") {
                if marker != "```yaml agent-test" {
                    return Err(ErrorKind::Source);
                }
                block = Some(String::new());
            }
            fence = Some((character, length));
        }
    }
    if fence.is_some() || cases.is_empty() || cases.len() > 32 {
        return Err(ErrorKind::Source);
    }
    Ok(cases)
}
impl ParsedPair {
    pub fn parse(agent: &[u8], tests: &[u8]) -> Result<Self, ErrorKind> {
        let snapshot = SnapshotId::of(agent, tests);
        let (header, prompt) = frontmatter(agent)?;
        let agent: AgentHeader = yaml(header)?;
        let (header, body) = frontmatter(tests)?;
        let tests_header: TestsHeader = yaml(header)?;
        if agent.kmx.schema_version != 1
            || tests_header.schema_version != 1
            || agent.kmx.runtime != "orka"
            || !present(&agent.model)
            || !present(prompt)
        {
            return Err(ErrorKind::Source);
        }
        // Metadata is deliberately inert; it never chooses a remote resource name.
        let _ = (agent.name, agent.description);
        let mut tools = Vec::new();
        for tool in agent.kmx.tools {
            if tool.kind != "builtin"
                || !supported_builtin(&tool.name)
                || tools.contains(&tool.name)
            {
                return Err(ErrorKind::Source);
            }
            tools.push(tool.name);
        }
        let cases = test_blocks(body)?;
        let mut ids = HashSet::new();
        for case in &cases {
            if !present(&case.id)
                || !ids.insert(&case.id)
                || !present(&case.input)
                || case.assertions.is_empty()
                || case.assertions.len() > 64
            {
                return Err(ErrorKind::Source);
            }
            let mut assertion_ids = HashSet::new();
            for assertion in &case.assertions {
                let value = match assertion {
                    Assertion::Contains { value, .. } => value,
                    Assertion::ToolCalled { tool, .. } => tool,
                };
                if !present(assertion.id())
                    || !assertion_ids.insert(assertion.id())
                    || !present(value)
                {
                    return Err(ErrorKind::Source);
                }
            }
        }
        Ok(Self {
            snapshot,
            prompt: prompt.into(),
            model: agent.model,
            tools,
            cases,
        })
    }
    pub fn snapshot(&self) -> &SnapshotId {
        &self.snapshot
    }
    pub fn prompt(&self) -> &str {
        &self.prompt
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn tools(&self) -> &[String] {
        &self.tools
    }
    pub fn cases(&self) -> &[RunCase] {
        &self.cases
    }
}
impl RunCase {
    pub fn input(&self) -> &str {
        &self.input
    }
    pub fn assertions(&self) -> &[Assertion] {
        &self.assertions
    }
    pub(super) fn evaluate(&self, answer: Option<&str>, tools: &[String]) -> Vec<AssertionReport> {
        self.assertions
            .iter()
            .map(|assertion| {
                let verdict = match assertion {
                    Assertion::Contains { value, .. } => match answer {
                        Some(text) if text.contains(value) => Verdict::Passed,
                        Some(_) => Verdict::Failed,
                        None => Verdict::Unknown,
                    },
                    Assertion::ToolCalled { tool, .. } if tools.contains(tool) => Verdict::Passed,
                    Assertion::ToolCalled { .. } => Verdict::Unknown,
                };
                AssertionReport {
                    id: assertion.id().into(),
                    kind: match assertion {
                        Assertion::Contains { .. } => "contains",
                        Assertion::ToolCalled { .. } => "toolCalled",
                    },
                    verdict,
                }
            })
            .collect()
    }
}
