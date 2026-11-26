//! The Navigator on Home: a Bridge-owned agent with fixed instructions, a bundled digest of
//! the Bridge and Orka documentation and no tools. It runs on the local workspace through the
//! chat path under its own `bridge-nav-*` names. Its replies are data: the renderer validates
//! any proposal it contains and nothing happens until the person confirms a card.
use crate::{
    agent_folder::DocumentId,
    agents::Agents,
    chat::{ChatOwner, ChatSnapshot},
    engine::run::ParsedAgent,
};
use bridge_runtime_protocol::{ModelSelection, ModelVendor, Uuid};
use serde::Serialize;

/// The key of the Navigator's one live conversation. It is never a saved agent file, and it
/// lives in the Navigator's own conversation owner, apart from every document's.
pub const CONVERSATION: DocumentId =
    DocumentId(Uuid::from_u128(0x6e617669_6761_4074_9f2e_6e6176696761));
/// Bundled with the renderer sources so one file is reviewed, tested for size and shipped.
const INSTRUCTIONS: &str = include_str!("../../src/home/navigator/instructions.md");
const DIGEST: &str = include_str!("../../src/home/navigator/digest.md");
/// With the instructions and digest, these keep the whole Agent create body inside the
/// transport's 64 KiB `MAX_ORKA_BODY`; `navigator/size.test.ts` checks the same budget.
pub const MAX_SAVED_AGENTS: usize = 32;
pub const MAX_SAVED_AGENT_LINE: usize = 240;

/// The Navigator's agent file for one conversation, on the model the workspace is bound to.
pub fn agent_source(model: &str, saved_agents: &str) -> Vec<u8> {
    // A JSON string is a valid double-quoted YAML scalar; the binding's model is plain ASCII.
    let model = serde_json::to_string(model).expect("a string serializes");
    format!(
        "---\nname: navigator\ndescription: \"Explains Orka and Bridge and proposes cards\"\nmodel: {model}\nkmx:\n  schemaVersion: 1\n  runtime: orka\n  tools: []\n---\n\n{INSTRUCTIONS}\n\n# Saved agents on this computer\n\n{saved_agents}\n\n{DIGEST}"
    )
    .into_bytes()
}

/// One line per saved agent whose file can be read: its name and description, bounded.
pub fn saved_agents(agents: &Agents) -> String {
    let Ok(list) = agents.list() else {
        return "Bridge could not read the saved agents.".into();
    };
    let lines: Vec<String> = list
        .into_iter()
        .filter(|entry| entry.available)
        .take(MAX_SAVED_AGENTS)
        .map(|entry| {
            let description = agents
                .get(entry.id)
                .ok()
                .and_then(|folder| folder.open().ok())
                .and_then(|pair| ParsedAgent::parse(pair.agent.as_bytes()).ok())
                .and_then(|parsed| parsed.description().map(str::to_owned))
                .unwrap_or_default();
            saved_agent_line(&entry.name, &description)
        })
        .collect();
    if lines.is_empty() {
        "None yet.".into()
    } else {
        lines.join("\n")
    }
}

fn saved_agent_line(name: &str, description: &str) -> String {
    let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut line = if description.is_empty() {
        format!("- {name}")
    } else {
        format!("- {name}: {description}")
    };
    if line.len() > MAX_SAVED_AGENT_LINE {
        let mut end = MAX_SAVED_AGENT_LINE;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
    }
    line
}

/// The model the Navigator last thought with, as the composer names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinksWith {
    /// `copilot`, `openai` or `anthropic`.
    pub kind: &'static str,
    pub model: String,
}
impl From<&ModelSelection> for ThinksWith {
    fn from(selection: &ModelSelection) -> Self {
        Self {
            kind: selection.vendor.map_or("copilot", ModelVendor::as_str),
            model: selection.model.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigatorSnapshot {
    pub chat: ChatSnapshot,
    pub thinks_with: Option<ThinksWith>,
}
pub fn snapshot(owner: &ChatOwner, chat: ChatSnapshot) -> NavigatorSnapshot {
    NavigatorSnapshot {
        chat,
        thinks_with: owner.thinks_with().as_ref().map(ThinksWith::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_runtime_protocol::{ChatMethod, ChatRequest, MAX_ORKA_BODY, NAVIGATOR_FAMILY};

    #[test]
    fn navigator_source_parses_as_a_tool_free_agent_on_the_bound_model() {
        let source = agent_source("gpt-5.6-terra", "- web-reader: Answers from a page");
        let parsed = ParsedAgent::parse(&source).unwrap();
        assert_eq!(parsed.model(), "gpt-5.6-terra");
        assert!(parsed.tools().is_empty());
        assert!(!parsed.has_unknown_local_builtin() && !parsed.has_unsupported_local_tool());
        assert!(parsed.prompt().contains("navigator-card"));
        assert!(parsed
            .prompt()
            .contains("# Saved agents on this computer\n\n- web-reader: Answers from a page"));
        assert!(parsed.prompt().contains("Source: "));
    }

    #[test]
    fn the_largest_navigator_agent_fits_one_create_request() {
        // Quotes and newlines cost the most once the body is JSON.
        let line = saved_agent_line(&"a".repeat(63), &"\"é ".repeat(400));
        let saved = vec![line; MAX_SAVED_AGENTS].join("\n");
        let source = agent_source(&"m".repeat(256), &saved);
        let parsed = ParsedAgent::parse(&source).unwrap();
        let body = serde_json::json!({
            "name": format!("{}{}", NAVIGATOR_FAMILY.agent, "0".repeat(32)),
            "namespace": "orka-system",
            "spec": parsed.spec(&"p".repeat(45)),
        });
        assert!(serde_json::to_vec(&body).unwrap().len() <= MAX_ORKA_BODY);
        let request = ChatRequest {
            method: ChatMethod::Post,
            path: "agents".into(),
            after: None,
            body: Some(body),
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn saved_agent_lines_are_single_bounded_lines() {
        assert_eq!(saved_agent_line("web-reader", ""), "- web-reader");
        assert_eq!(
            saved_agent_line("web-reader", "  Reads\n a page "),
            "- web-reader: Reads a page"
        );
        let long = saved_agent_line("a", &"é".repeat(400));
        assert!(long.len() <= MAX_SAVED_AGENT_LINE && !long.contains('\n'));
    }

    #[test]
    fn thinks_with_names_copilot_or_the_key_vendor() {
        let copilot = ModelSelection {
            vendor: None,
            provider: "copilot".into(),
            model: "gpt-5.6-terra".into(),
        };
        assert_eq!(
            serde_json::to_value(ThinksWith::from(&copilot)).unwrap(),
            serde_json::json!({"kind":"copilot","model":"gpt-5.6-terra"})
        );
        let key = ModelSelection {
            vendor: Some(ModelVendor::Anthropic),
            ..copilot
        };
        assert_eq!(ThinksWith::from(&key).kind, "anthropic");
    }
}
