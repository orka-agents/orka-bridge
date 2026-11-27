//! A job without a folder: draft an agent from one sentence, run it, review the answer.
//! Model text is parsed as data and never written as a file or YAML directly.
use crate::{
    agent_folder::{child, read, replace, DocumentId},
    engine::run::{ErrorKind, ParsedPair},
};
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf, sync::Mutex};

/// Matches the first screen's job box: trimmed, at most 8 KiB, tabs and newlines only.
pub const MAX_PROMPT: usize = 8 * 1024;
const MAX_INSTRUCTIONS: usize = 4 * 1024;
const MAX_CHECK: usize = 200;
const MAX_SUMMARY: usize = 500;
const MAX_GAP: usize = 300;
const MAX_GAPS: usize = 5;
/// The reviewer reads at most this much of the answer.
const MAX_REVIEWED_ANSWER: usize = 16 * 1024;
const DIRECTORY: &str = "job-runs-v1";
const JOB_TOOLS: [&str; 1] = ["web_fetch"];

const DRAFTER: &str = "You turn a job someone wants done into instructions for an AI agent and one check. \
Don't do the job yourself.

Reply with only a JSON object and no other text, in this shape:
{\"instructions\": \"...\", \"tools\": [], \"check\": {\"type\": \"contains\", \"value\": \"...\"}}

- instructions: what the agent should do and how it should answer, written to the agent, under 2,000 characters.
- tools: [] or [\"web_fetch\"]. Use web_fetch only when the job needs a web page read.
- check: {\"type\": \"toolCalled\", \"tool\": \"web_fetch\"} when the job needs that page read, otherwise \
{\"type\": \"contains\", \"value\": \"...\"} with short exact text that any correct answer contains.";

const REVIEWER: &str = "You review whether an AI agent's answer does the job it was given. You can't run \
anything or open links; judge only from the text.

Reply with only a JSON object and no other text, in this shape:
{\"verdict\": \"done\", \"summary\": \"...\", \"gaps\": []}

- verdict: \"done\" if the answer does what was asked, \"partly\" if it does some of it, \"notDone\" if it doesn't.
- summary: one sentence on how well the answer does the job.
- gaps: what the answer left out or got wrong, at most 5, one sentence each. [] when nothing is missing.";

/// The same rules as the job box, so a saved job always reaches here unchanged.
pub fn valid_prompt(prompt: &str) -> bool {
    prompt == prompt.trim()
        && !prompt.is_empty()
        && prompt.len() <= MAX_PROMPT
        && !prompt
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}
fn plain(text: &str, limit: usize) -> bool {
    !text.trim().is_empty() && text.len() <= limit && !text.chars().any(|c| c.is_control())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Check {
    #[serde(rename = "contains")]
    Contains { value: String },
    #[serde(rename = "toolCalled")]
    ToolCalled { tool: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub instructions: String,
    pub tools: Vec<String>,
    pub check: Check,
}
impl Draft {
    fn valid(&self) -> bool {
        let tools_ok = self.tools.len() <= JOB_TOOLS.len()
            && self.tools.iter().all(|t| JOB_TOOLS.contains(&t.as_str()));
        let check_ok = match &self.check {
            Check::Contains { value } => plain(value, MAX_CHECK),
            Check::ToolCalled { tool } => self.tools.contains(tool),
        };
        !self.instructions.trim().is_empty()
            && self.instructions.len() <= MAX_INSTRUCTIONS
            && !self.instructions.contains('\0')
            && tools_ok
            && check_ok
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewVerdict {
    Done,
    Partly,
    NotDone,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub verdict: ReviewVerdict,
    pub summary: String,
    pub gaps: Vec<String>,
}
impl Review {
    fn valid(&self) -> bool {
        plain(&self.summary, MAX_SUMMARY)
            && self.gaps.len() <= MAX_GAPS
            && self.gaps.iter().all(|gap| plain(gap, MAX_GAP))
    }
}

/// One JSON object, optionally inside a single fenced block. Anything else is refused.
fn json_object<T: for<'de> Deserialize<'de>>(answer: &str) -> Option<T> {
    let mut text = answer.trim();
    if let Some(rest) = text.strip_prefix("```") {
        let (_, body) = rest.split_once('\n')?;
        text = body.trim_end().strip_suffix("```")?.trim();
    }
    serde_json::from_str(text).ok()
}
pub fn parse_draft(answer: &str) -> Option<Draft> {
    json_object::<Draft>(answer).filter(Draft::valid)
}
pub fn parse_review(answer: &str) -> Option<Review> {
    json_object::<Review>(answer).filter(Review::valid)
}

fn quoted(text: &str) -> Result<String, ErrorKind> {
    // A JSON string is a YAML double-quoted scalar and stays on one line.
    serde_json::to_string(text).map_err(|_| ErrorKind::Source)
}
fn agent_file(name: &str, model: &str, tools: &[String], body: &str) -> Result<String, ErrorKind> {
    let mut tools_yaml = String::new();
    for tool in tools {
        tools_yaml.push_str(&format!(
            "\n    - kind: builtin\n      name: {}",
            quoted(tool)?
        ));
    }
    let tools_yaml = if tools.is_empty() {
        String::new()
    } else {
        format!("\n  tools:{tools_yaml}")
    };
    Ok(format!(
        "---\nname: {}\nmodel: {}\nkmx:\n  schemaVersion: 1\n  runtime: orka{tools_yaml}\n---\n\n{}\n",
        quoted(name)?,
        quoted(model)?,
        body.trim()
    ))
}
fn tests_file(input: &str, check: &Check) -> Result<String, ErrorKind> {
    let assertion = match check {
        Check::Contains { value } => format!("type: contains\n    value: {}", quoted(value)?),
        Check::ToolCalled { tool } => format!("type: toolCalled\n    tool: {}", quoted(tool)?),
    };
    Ok(format!(
        "---\nschemaVersion: 1\n---\n\n```yaml agent-test\nid: job\ninput: {}\nassertions:\n  - id: check\n    {assertion}\n```\n",
        quoted(input)?
    ))
}
fn pair(agent: &str, tests: &str) -> Result<ParsedPair, ErrorKind> {
    ParsedPair::parse(agent.as_bytes(), tests.as_bytes())
}
/// The drafter has no tools. Its required assertion only names the JSON key;
/// `parse_draft` is the real validation.
pub fn draft_pair(model: &str, prompt: &str) -> Result<ParsedPair, ErrorKind> {
    let check = Check::Contains {
        value: "\"instructions\"".into(),
    };
    pair(
        &agent_file("job-drafter", model, &[], DRAFTER)?,
        &tests_file(prompt, &check)?,
    )
}
/// The saved agent's files: the drafted check becomes its first test (#289).
pub fn job_files(model: &str, prompt: &str, draft: &Draft) -> Result<(String, String), ErrorKind> {
    Ok((
        agent_file("job", model, &draft.tools, &draft.instructions)?,
        tests_file(prompt, &draft.check)?,
    ))
}
pub fn job_pair(model: &str, prompt: &str, draft: &Draft) -> Result<ParsedPair, ErrorKind> {
    let (agent, tests) = job_files(model, prompt, draft)?;
    pair(&agent, &tests)
}
pub fn review_pair(model: &str, prompt: &str, answer: &str) -> Result<ParsedPair, ErrorKind> {
    let mut cut = answer.len().min(MAX_REVIEWED_ANSWER);
    while !answer.is_char_boundary(cut) {
        cut -= 1;
    }
    let note = if cut < answer.len() {
        "\n\n(The answer is cut here; judge what is shown.)"
    } else {
        ""
    };
    let input = format!("Job:\n{prompt}\n\nAnswer:\n{}{note}", &answer[..cut]);
    let check = Check::Contains {
        value: "\"verdict\"".into(),
    };
    pair(
        &agent_file("job-reviewer", model, &[], REVIEWER)?,
        &tests_file(&input, &check)?,
    )
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    prompt: String,
    draft: Draft,
}
/// Drafts by job id, so a rerun doesn't draft again. Absent or unreadable means draft again.
pub struct Drafts {
    app_data: Option<PathBuf>,
    lock: Mutex<()>,
}
impl Drafts {
    pub fn new(app_data: Option<PathBuf>) -> Self {
        Self {
            app_data,
            lock: Mutex::new(()),
        }
    }
    fn root(&self, create: bool) -> io::Result<Option<Dir>> {
        let Some(path) = &self.app_data else {
            return Ok(None);
        };
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound && create => {
                std::fs::create_dir_all(path)?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
            Ok(meta) if !meta.file_type().is_dir() => return Err(io::ErrorKind::InvalidData.into()),
            Ok(_) => (),
        }
        let parent = Dir::open_ambient_dir(path, cap_std::ambient_authority())?;
        if create {
            return child(&parent, DIRECTORY).map(Some);
        }
        match parent.symlink_metadata(DIRECTORY) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
            Ok(_) => parent.open_dir_nofollow(DIRECTORY).map(Some),
        }
    }
    fn leaf(id: DocumentId) -> String {
        format!("{}.json", id.0.simple())
    }
    pub fn load(&self, id: DocumentId, prompt: &str) -> Option<Draft> {
        let _guard = self.lock.lock().ok()?;
        let root = self.root(false).ok()??;
        let bytes = read(&root, &Self::leaf(id), 64 * 1024).ok()?;
        let saved: Saved = serde_json::from_slice(&bytes).ok()?;
        (saved.version == 1 && saved.prompt == prompt && saved.draft.valid()).then_some(saved.draft)
    }
    pub fn save(&self, id: DocumentId, prompt: &str, draft: &Draft) -> io::Result<()> {
        let _guard = self.lock.lock().map_err(|_| io::Error::other("lock"))?;
        let root = self
            .root(true)?
            .ok_or_else(|| io::Error::other("no app data"))?;
        let bytes = serde_json::to_vec(&Saved {
            version: 1,
            prompt: prompt.into(),
            draft: draft.clone(),
        })?;
        replace(&root, &Self::leaf(id), &bytes)
    }
}

#[cfg(test)]
#[path = "job_run_tests.rs"]
mod tests;
