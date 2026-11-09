use bridge_engine::agent_folder::AgentFolder;
use bridge_runtime_protocol::{FileSlot, MAX_FILE};
use std::{fs, path::Path, sync::Arc};

const AGENT: &str = "web-reader.agent.md";
const TESTS: &str = "web-reader.agent.test.md";
fn root(home: &Path) -> std::path::PathBuf {
    home.join("Orka/agents/web-reader")
}
fn committed(root: &Path, name: &str) -> Vec<u8> {
    let repo = git2::Repository::open(root).unwrap();
    let tree = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .tree()
        .unwrap();
    let id = tree.get_name(name).unwrap().id();
    let blob = repo.find_blob(id).unwrap();
    blob.content().to_vec()
}
#[test]
fn initializes_without_vm_and_reopens_exact_drafts_with_committed_bytes() {
    let home = tempfile::tempdir().unwrap();
    let folder = AgentFolder::new(Some(home.path().into()));
    let pair = folder.open().unwrap();
    assert!(pair.agent.contains("web_fetch"));
    assert!(pair.tests.contains("toolCalled"));
    let (path, doc) = folder.source().unwrap();
    assert_eq!(path, root(home.path()));
    assert_eq!(doc.agent, Path::new(AGENT));
    let draft = "\r\nnot schema 🦀\n";
    assert_eq!(folder.save(draft.into()).unwrap().agent, draft);
    assert_eq!(committed(&path, AGENT), draft.as_bytes());
    assert_eq!(committed(&path, TESTS), pair.tests.as_bytes());
    let reopened = AgentFolder::new(Some(home.path().into()));
    assert_eq!(reopened.open().unwrap().agent, draft);
    assert_eq!(
        reopened.capture().unwrap().file(FileSlot::Agent),
        draft.as_bytes()
    );
    assert_eq!(reopened.save(String::new()).unwrap().agent, "");
    assert_eq!(committed(&path, AGENT), b"");
}
#[test]
fn unrelated_directory_and_missing_home_never_overwrite_existing_data() {
    assert!(AgentFolder::new(None).open().is_err());
    let home = tempfile::tempdir().unwrap();
    let path = root(home.path());
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join(AGENT), b"my work").unwrap();
    let folder = AgentFolder::new(Some(home.path().into()));
    assert!(folder.open().is_err());
    assert!(folder.save("replacement".into()).is_err());
    assert_eq!(fs::read(path.join(AGENT)).unwrap(), b"my work");
    assert!(!path.join(".git").exists());
}
#[test]
fn bounded_utf8_noop_and_explicit_identity_ignore_git_filters_and_config() {
    let home = tempfile::tempdir().unwrap();
    let folder = AgentFolder::new(Some(home.path().into()));
    let original = folder.open().unwrap();
    let path = root(home.path());
    let head = || {
        git2::Repository::open(&path)
            .unwrap()
            .head()
            .unwrap()
            .target()
            .unwrap()
    };
    let first = head();
    folder.save(original.agent.clone()).unwrap();
    assert_eq!(head(), first);
    assert!(folder.save("x".repeat(MAX_FILE + 1)).is_err());
    assert_eq!(folder.open().unwrap().agent, original.agent);
    // These are never interpreted: blob/tree/commit operations see exact bytes.
    fs::write(
        path.join(".gitattributes"),
        "* filter=untrusted text eol=lf\n",
    )
    .unwrap();
    fs::write(path.join(".git/config"), "[core]\n bare=false\n[filter \"untrusted\"]\n clean=never-run-this-command\n[user]\n name=Not Bridge\n email=other@example.com\n").unwrap();
    let draft = "exact\r\n🦀";
    folder.save(draft.into()).unwrap();
    let repo = git2::Repository::open(&path).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(commit.author().name().unwrap(), "Bridge");
    assert_eq!(commit.author().email().unwrap(), "bridge@orka.ai");
    assert_eq!(commit.parent_count(), 1);
    assert_eq!(committed(&path, AGENT), draft.as_bytes());
    assert_eq!(
        fs::read(path.join(TESTS)).unwrap(),
        original.tests.as_bytes()
    );
}
#[test]
fn dirty_reopen_commits_disk_not_bundled_or_head_and_failed_commit_retries() {
    let home = tempfile::tempdir().unwrap();
    let folder = AgentFolder::new(Some(home.path().into()));
    folder.open().unwrap();
    let path = root(home.path());
    fs::write(path.join(AGENT), "external draft").unwrap();
    assert_eq!(folder.open().unwrap().agent, "external draft");
    assert_eq!(committed(&path, AGENT), b"external draft");
    let head = fs::read(path.join(".git/HEAD")).unwrap();
    fs::remove_file(path.join(".git/index")).unwrap();
    fs::create_dir(path.join(".git/index")).unwrap();
    assert!(folder.save("retry draft".into()).is_err());
    assert_eq!(fs::read(path.join(AGENT)).unwrap(), b"retry draft");
    assert_eq!(fs::read(path.join(".git/HEAD")).unwrap(), head);
    fs::remove_dir(path.join(".git/index")).unwrap();
    folder.save("retry draft".into()).unwrap();
    assert_eq!(committed(&path, AGENT), b"retry draft");
}
#[test]
fn concurrent_saves_are_serial_and_every_success_is_committed() {
    let home = tempfile::tempdir().unwrap();
    let folder = Arc::new(AgentFolder::new(Some(home.path().into())));
    folder.open().unwrap();
    let threads: Vec<_> = (0..8)
        .map(|i| {
            let folder = folder.clone();
            std::thread::spawn(move || folder.save(format!("draft {i}")).unwrap())
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let repo = git2::Repository::open(root(home.path())).unwrap();
    let mut walk = repo.revwalk().unwrap();
    walk.push_head().unwrap();
    assert_eq!(walk.count(), 9);
    assert_eq!(
        committed(&root(home.path()), AGENT),
        folder.open().unwrap().agent.as_bytes()
    );
}
#[cfg(unix)]
#[test]
fn redirected_ancestors_source_and_git_never_write_outside() {
    use std::os::unix::fs::symlink;
    for redirected in ["Orka", "Orka/agents", "Orka/agents/web-reader"] {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = home.path().join(redirected);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(outside.path(), &link).unwrap();
        assert!(AgentFolder::new(Some(home.path().into()))
            .save("no".into())
            .is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
    for redirected in [AGENT, TESTS, ".git", ".git/objects", ".git/HEAD"] {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let folder = AgentFolder::new(Some(home.path().into()));
        folder.open().unwrap();
        let link = root(home.path()).join(redirected);
        let is_dir = link.is_dir();
        if is_dir {
            fs::remove_dir_all(&link).unwrap();
        } else {
            fs::remove_file(&link).unwrap();
        }
        let target = if is_dir {
            outside.path().to_path_buf()
        } else {
            let target = outside.path().join("work");
            fs::write(&target, b"outside").unwrap();
            target
        };
        symlink(&target, &link).unwrap();
        assert!(folder.save("no".into()).is_err(), "{redirected}");
        if !is_dir {
            assert_eq!(fs::read(target).unwrap(), b"outside");
        } else {
            assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        }
    }
}
#[test]
fn hardlinked_source_and_head_cannot_modify_outside_files() {
    for name in [AGENT, TESTS, ".git/HEAD"] {
        let home = tempfile::tempdir().unwrap();
        let folder = AgentFolder::new(Some(home.path().into()));
        folder.open().unwrap();
        let path = root(home.path());
        let outside = home.path().join("outside");
        let before = fs::read(path.join(name)).unwrap();
        fs::hard_link(path.join(name), &outside).unwrap();
        assert!(folder.save("no".into()).is_err());
        assert_eq!(fs::read(outside).unwrap(), before);
    }
}

#[test]
fn ordinary_git_fsck_log_show_and_status_agree_with_saved_document() {
    let home = tempfile::tempdir().unwrap();
    let folder = AgentFolder::new(Some(home.path().into()));
    folder.save("exact Git bytes\\r\\n🦀".into()).unwrap();
    let path = root(home.path());
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    };
    git(&["fsck", "--full"]);
    assert_eq!(
        git(&["log", "-1", "--format=%an <%ae>"]),
        b"Bridge <bridge@orka.ai>\n"
    );
    assert_eq!(
        git(&["show", "HEAD:web-reader.agent.md"]),
        b"exact Git bytes\\r\\n\xf0\x9f\xa6\x80"
    );
    assert_eq!(git(&["status", "--porcelain"]), b"");
}
