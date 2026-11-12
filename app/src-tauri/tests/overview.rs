use bridge_engine::{agents::Agents, overview::saved_files};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[path = "support/created_agent.rs"]
mod created_agent;

fn owner(home: &Path) -> Agents {
    Agents::new(Some(home.into()), Some(home.join("fixture-data")))
}
fn tree(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.insert(path.strip_prefix(root).unwrap().into(), Vec::new());
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(path, path, &mut files);
    files
}
#[test]
fn registry_observation_is_non_mutating_and_keeps_document_identity() {
    let home = tempfile::tempdir().unwrap();
    let agents = owner(home.path());
    let pair = agents.sample().open().unwrap();
    let a = created_agent::created(home.path(), "alpha", &pair.agent, &pair.tests);
    let b = created_agent::created(home.path(), "beta", &pair.agent, &pair.tests);
    let before = tree(home.path());
    let files = saved_files(&agents, true);
    assert!(files.run_active);
    assert_eq!(files.state, "complete");
    assert_eq!(files.files.len(), 3);
    assert_eq!(files.files[1].document_id, a.document_id());
    assert_eq!(files.files[2].document_id, b.document_id());
    assert_ne!(a.document_id(), b.document_id());
    assert_eq!(files.files[1].source, files.files[2].source);
    assert_eq!(
        files.files[0].summary.as_ref().unwrap().model,
        "gpt-5.6-terra"
    );
    assert_eq!(before, tree(home.path()));
}
#[test]
fn dirty_reopen_is_not_committed_and_unsupported_source_is_unavailable() {
    let home = tempfile::tempdir().unwrap();
    let agents = owner(home.path());
    let pair = agents.sample().open().unwrap();
    let path = home
        .path()
        .join("Orka/agents/web-reader/web-reader.agent.md");
    fs::write(&path, format!("{}\nUncommitted prompt", pair.agent)).unwrap();
    let before = tree(home.path());
    let observed = saved_files(&agents, false);
    assert_ne!(observed.files[0].source.as_ref().unwrap(), &pair.source);
    assert!(observed.files[0].summary.is_some());
    assert_eq!(before, tree(home.path()));
    fs::write(
        &path,
        pair.agent.replace("runtime: orka", "runtime: unsupported"),
    )
    .unwrap();
    let before = tree(home.path());
    let observed = saved_files(&agents, false);
    assert_eq!(observed.files[0].state, "readable");
    assert!(observed.files[0].source.is_some());
    assert!(observed.files[0].summary.is_none());
    assert!(observed.files[0].reason.is_some());
    assert_eq!(before, tree(home.path()));
}
#[test]
fn missing_registered_pair_and_bad_registry_are_not_empty_successes() {
    let home = tempfile::tempdir().unwrap();
    let agents = owner(home.path());
    let pair = agents.sample().open().unwrap();
    created_agent::created(home.path(), "alpha", &pair.agent, &pair.tests);
    fs::remove_dir_all(home.path().join("Orka/agents/alpha")).unwrap();
    let files = saved_files(&agents, false);
    assert_eq!(files.files.len(), 2);
    assert_eq!(files.files[1].state, "unreadable");
    fs::write(
        home.path().join("fixture-data/agents-v1/registry.json"),
        b"not json",
    )
    .unwrap();
    assert_eq!(saved_files(&agents, false).state, "unavailable");
}
#[cfg(unix)]
#[test]
fn ownership_and_nofollow_are_preserved() {
    use std::os::unix::fs::symlink;
    for leaf in [
        ".bridge-agent-folder",
        "web-reader.agent.md",
        "web-reader.agent.test.md",
        ".git",
    ] {
        let home = tempfile::tempdir().unwrap();
        let agents = owner(home.path());
        agents.sample().open().unwrap();
        let root = home.path().join("Orka/agents/web-reader");
        let original = root.join(leaf);
        let outside = home.path().join("outside");
        fs::rename(&original, &outside).unwrap();
        symlink(&outside, original).unwrap();
        assert_eq!(
            saved_files(&agents, false).files[0].state,
            "unreadable",
            "{leaf}"
        );
    }
    let home = tempfile::tempdir().unwrap();
    let agents = owner(home.path());
    agents.sample().open().unwrap();
    fs::write(
        home.path()
            .join("Orka/agents/web-reader/.bridge-agent-folder"),
        b"other owner",
    )
    .unwrap();
    assert_eq!(saved_files(&agents, false).files[0].state, "unreadable");
}
#[test]
fn projection_excludes_paths_source_bytes_and_diagnostics() {
    let home = tempfile::tempdir().unwrap();
    let agents = owner(home.path());
    let pair = agents.sample().open().unwrap();
    agents
        .sample()
        .save(format!("{}\nPRIVATE PROMPT BODY", pair.agent))
        .unwrap();
    let value = serde_json::to_string(&saved_files(&agents, false)).unwrap();
    for (index, forbidden) in [
        "PRIVATE PROMPT BODY",
        "path",
        home.path().to_str().unwrap(),
        "schemaVersion",
        "agent-test",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            !value.contains(forbidden),
            "Forbidden Overview field {index}"
        );
    }
}
#[test]
fn missing_is_not_seeded() {
    let home = tempfile::tempdir().unwrap();
    let files = saved_files(&owner(home.path()), false);
    assert!(files.files.is_empty());
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 0);
}
