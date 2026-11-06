//! First run copies this example agent and its test into the new agent repository.
//! The test must fail unless the agent really fetches the page. These checks
//! catch edits that would let it pass another way: a tool that can return mock
//! results, recorded tool results, the answer in the prompt or input, or a page
//! that can change.

const AGENT: &str = include_str!("../example-agent/web-reader.agent.md");
const TESTS: &str = include_str!("../example-agent/web-reader.agent.test.md");

fn frontmatter(source: &str) -> Vec<&str> {
    let mut lines = source.lines();
    assert_eq!(lines.next(), Some("---"), "missing frontmatter");
    lines.take_while(|line| *line != "---").collect()
}

fn test_fences(source: &str) -> Vec<Vec<&str>> {
    let mut fences = Vec::new();
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        if line == "```yaml agent-test" {
            fences.push(lines.by_ref().take_while(|line| *line != "```").collect());
        }
    }
    fences
}

fn field<'a>(lines: &[&'a str], key: &str) -> Vec<&'a str> {
    lines
        .iter()
        .filter_map(|line| line.trim_start().trim_start_matches("- ").strip_prefix(key))
        .collect()
}

/// An exact release path never changes; `stable.txt` or `latest` would.
fn is_pinned_release_page(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://dl.k8s.io/release/v") else {
        return false;
    };
    let Some((version, path)) = rest.split_once('/') else {
        return false;
    };
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        && path.ends_with(".sha256")
}

#[test]
fn example_agent_offers_only_web_fetch() {
    let header = frontmatter(AGENT);
    assert!(header.contains(&"  runtime: orka"));
    let tools: Vec<_> = header
        .iter()
        .skip_while(|line| **line != "  tools:")
        .skip(1)
        .take_while(|line| line.starts_with("    "))
        .map(|line| line.trim())
        .collect();
    assert_eq!(tools, ["- kind: builtin", "name: web_fetch"]);
    // web_search can substitute mock results when its backend fails.
    assert!(!AGENT.contains("web_search"));
}

#[test]
fn example_test_passes_only_with_the_fetched_page() {
    let fences = test_fences(TESTS);
    assert_eq!(fences.len(), 1, "keep one example test");
    let case = &fences[0];
    assert!(
        field(case, "mocks:").is_empty(),
        "recorded tool results would pass without fetching"
    );

    let input = field(case, "input: ");
    assert_eq!(input.len(), 1);
    let url = input[0]
        .split_whitespace()
        .find(|word| word.starts_with("https://"))
        .expect("the input names the page to fetch");
    assert!(is_pinned_release_page(url), "unpinned page: {url}");

    let types = field(case, "type: ");
    let values = field(case, "value: ");
    assert_eq!(types, ["contains", "toolCalled"]);
    assert_eq!(field(case, "tool: "), ["web_fetch"]);
    assert_eq!(values.len(), 1);
    let checksum = values[0];
    let lower_hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
    assert!(
        checksum.len() == 64 && checksum.bytes().all(lower_hex),
        "expect the page's whole SHA-256 checksum"
    );
    assert!(
        !AGENT.contains(checksum),
        "the prompt gives away the answer"
    );
    assert!(
        !input[0].contains(checksum),
        "the input gives away the answer"
    );
}
