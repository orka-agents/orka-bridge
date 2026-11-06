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
            if tool.kind != "builtin" || !present(&tool.name) || tools.contains(&tool.name) {
                return Err(ErrorKind::Source);
            }
            tools.push(tool.name);
        }
        let mut cases: Vec<RunCase> = Vec::new();
        let mut block = None;
        let mut other_fence = false;
        for line in body.split_inclusive('\n') {
            let marker = line.trim_end_matches(['\r', '\n']);
            if marker == "```yaml agent-test" && !other_fence {
                if block.is_some() {
                    return Err(ErrorKind::Source);
                }
                block = Some(String::new());
            } else if marker == "```" && block.is_some() {
                cases.push(yaml(&block.take().ok_or(ErrorKind::Source)?)?);
            } else if let Some(text) = &mut block {
                text.push_str(line);
            } else if marker.starts_with("```") {
                if marker.contains("agent-test") {
                    return Err(ErrorKind::Source);
                }
                other_fence = !other_fence;
            }
        }
        if block.is_some() || cases.is_empty() || cases.len() > 32 {
            return Err(ErrorKind::Source);
        }
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
                    verdict,
                }
            })
            .collect()
    }
}
