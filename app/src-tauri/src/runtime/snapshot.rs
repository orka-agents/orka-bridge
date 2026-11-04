use bridge_runtime_protocol::*;
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};
pub struct AgentDocument {
    pub agent: PathBuf,
    pub tests: PathBuf,
}
pub struct Snapshot {
    agent: Vec<u8>,
    tests: Vec<u8>,
}
fn invalid() -> io::Error {
    io::Error::from(io::ErrorKind::InvalidData)
}
fn admitted(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    if relative.as_os_str().is_empty() {
        return Err(invalid());
    }
    let mut path = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(invalid());
        };
        path.push(name);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(invalid());
        }
    }
    if !fs::canonicalize(&path)?.starts_with(root) {
        return Err(invalid());
    }
    Ok(path)
}
fn read(root: &Path, path: &Path) -> io::Result<Vec<u8>> {
    let full = admitted(root, path)?;
    if !fs::symlink_metadata(&full)?.is_file() {
        return Err(invalid());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let mut f = options.open(&full)?;
    if !f.metadata()?.is_file() {
        return Err(invalid());
    }
    let mut b = Vec::new();
    f.by_ref().take((MAX_FILE + 1) as u64).read_to_end(&mut b)?;
    if b.len() > MAX_FILE || admitted(root, path)? != full {
        return Err(invalid());
    }
    Ok(b)
}
impl Snapshot {
    pub fn capture(root: &Path, doc: &AgentDocument) -> io::Result<Self> {
        let root = fs::canonicalize(root)?;
        Self::collect(|slot| {
            read(
                &root,
                match slot {
                    FileSlot::Agent => &doc.agent,
                    FileSlot::Tests => &doc.tests,
                },
            )
        })
    }
    fn collect(mut read: impl FnMut(FileSlot) -> io::Result<Vec<u8>>) -> io::Result<Self> {
        let agent = read(FileSlot::Agent)?;
        let tests = read(FileSlot::Tests)?;
        if agent != read(FileSlot::Agent)? || tests != read(FileSlot::Tests)? {
            return Err(invalid());
        }
        Ok(Self { agent, tests })
    }
    pub fn id(&self) -> SnapshotId {
        SnapshotId::of(&self.agent, &self.tests)
    }
    pub fn file(&self, slot: FileSlot) -> &[u8] {
        match slot {
            FileSlot::Agent => &self.agent,
            FileSlot::Tests => &self.tests,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edited_during_capture_is_rejected() {
        let mut n = 0;
        assert!(Snapshot::collect(|_| {
            n += 1;
            Ok(vec![n])
        })
        .is_err());
    }
}
