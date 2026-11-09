//! libgit2 creates objects in memory. Only capability-relative code touches disk.
//! No repository discovery, ambient config, templates, alternates, hooks or remotes.
use super::*;
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use git2::{ObjectType, Odb, Oid, Repository, Signature};
fn git_error(_: git2::Error) -> io::Error {
    invalid()
}
pub(super) fn validate(root: &Dir) -> io::Result<Dir> {
    let git = root.open_dir_nofollow(".git")?;
    git.open_dir_nofollow("objects")?;
    // These are not used by us; refuse misleading externally redirected repositories.
    for name in ["commondir", "gitdir"] {
        match git.symlink_metadata(name) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            _ => return Err(invalid()),
        }
    }
    read(&git, "HEAD", 128)?;
    Ok(git)
}
fn load(git: &Dir, odb: &Odb<'_>, id: Oid) -> io::Result<()> {
    let name = id.to_string();
    let objects = git
        .open_dir_nofollow("objects")?
        .open_dir_nofollow(&name[..2])?;
    let compressed = read(&objects, &name[2..], MAX_FILE + 4096)?;
    let mut bytes = Vec::new();
    ZlibDecoder::new(&compressed[..])
        .take((MAX_FILE + 4097) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE + 4096 {
        return Err(invalid());
    }
    let zero = bytes.iter().position(|b| *b == 0).ok_or_else(invalid)?;
    let header = std::str::from_utf8(&bytes[..zero]).map_err(|_| invalid())?;
    let (kind, size) = header.split_once(' ').ok_or_else(invalid)?;
    let kind = match kind {
        "commit" => ObjectType::Commit,
        "tree" => ObjectType::Tree,
        "blob" => ObjectType::Blob,
        _ => return Err(invalid()),
    };
    let data = &bytes[zero + 1..];
    if size.parse::<usize>().map_err(|_| invalid())? != data.len()
        || odb.write(kind, data).map_err(git_error)? != id
    {
        return Err(invalid());
    }
    Ok(())
}
fn persist(git: &Dir, odb: &Odb<'_>, id: Oid) -> io::Result<()> {
    let object = odb.read(id).map_err(git_error)?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    write!(encoder, "{} {}\0", object.kind().str(), object.len())?;
    encoder.write_all(object.data())?;
    let bytes = encoder.finish()?;
    let name = id.to_string();
    let objects = git.open_dir_nofollow("objects")?;
    let prefix = child(&objects, &name[..2])?;
    match read(&prefix, &name[2..], MAX_FILE + 4096) {
        Ok(_) => load(git, odb, id)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => replace(&prefix, &name[2..], &bytes)?,
        _ => return Err(invalid()),
    }
    // A previous attempt can leave the object visible after its directory flush
    // failed. Retry that namespace flush even when the object bytes already match.
    sync(&prefix)?;
    sync(&objects)
}
pub(super) fn commit(root: &Dir, pair: &AgentPair) -> io::Result<()> {
    let git = validate(root)?;
    let head = read(&git, "HEAD", 128)?;
    let odb = Odb::new().map_err(git_error)?;
    odb.add_new_mempack_backend(1000).map_err(git_error)?;
    let parent_id = if head.is_empty() {
        None
    } else {
        let id = Oid::from_str(std::str::from_utf8(&head).map_err(|_| invalid())?.trim())
            .map_err(git_error)?;
        load(&git, &odb, id)?;
        Some(id)
    };
    let repo = Repository::from_odb(odb).map_err(git_error)?;
    repo.set_config(&git2::Config::new().map_err(git_error)?)
        .map_err(git_error)?;
    let agent = repo.blob(pair.agent.as_bytes()).map_err(git_error)?;
    let tests = repo.blob(pair.tests.as_bytes()).map_err(git_error)?;
    let mut builder = repo.treebuilder(None).map_err(git_error)?;
    builder.insert(AGENT, agent, 0o100644).map_err(git_error)?;
    builder.insert(TESTS, tests, 0o100644).map_err(git_error)?;
    let tree_id = builder.write().map_err(git_error)?;
    let parent = parent_id
        .map(|id| repo.find_commit(id))
        .transpose()
        .map_err(git_error)?;
    if parent.as_ref().is_some_and(|p| p.tree_id() == tree_id) {
        return index(&git, &pair.agent, agent, &pair.tests, tests);
    }
    let tree = repo.find_tree(tree_id).map_err(git_error)?;
    let identity = Signature::now("Bridge", "bridge@orka.ai").map_err(git_error)?;
    let commit = repo
        .commit(
            None,
            &identity,
            &identity,
            "Save agent",
            &tree,
            &parent.iter().collect::<Vec<_>>(),
        )
        .map_err(git_error)?;
    let odb = repo.odb().map_err(git_error)?;
    for id in [agent, tests, tree_id, commit] {
        persist(&git, &odb, id)?;
    }
    index(&git, &pair.agent, agent, &pair.tests, tests)?;
    // Publish only after all referenced objects are flushed. A failed publish is retryable.
    replace(&git, "HEAD", format!("{commit}\n").as_bytes())
}

// Fixed two-entry index v2. Stat fields are deliberately unknown: Git must hash
// working bytes, rather than trust stale metadata. No user index/filter is read.
fn index(git: &Dir, agent: &str, agent_id: Oid, tests: &str, tests_id: Oid) -> io::Result<()> {
    use sha1::{Digest, Sha1};
    let mut bytes = b"DIRC".to_vec();
    bytes.extend_from_slice(&2u32.to_be_bytes());
    bytes.extend_from_slice(&2u32.to_be_bytes());
    for (name, data, id) in [(AGENT, agent, agent_id), (TESTS, tests, tests_id)] {
        let start = bytes.len();
        let mut stat = [0u8; 40];
        stat[24..28].copy_from_slice(&0o100644u32.to_be_bytes());
        stat[36..40].copy_from_slice(&(data.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&stat);
        bytes.extend_from_slice(id.as_bytes());
        bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        while !(bytes.len() - start).is_multiple_of(8) {
            bytes.push(0);
        }
    }
    let checksum = Sha1::digest(&bytes);
    bytes.extend_from_slice(&checksum);
    replace(git, "index", &bytes)
}
