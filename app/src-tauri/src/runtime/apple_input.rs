//! Native-owned ephemeral configuration and stdio-only SDK identity input.
use super::{spawn_guard, types::*};
use bridge_runtime_protocol::Uuid;
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Sender},
    time::{Duration, Instant},
};

pub(crate) fn helper_path(root: &Path) -> Result<PathBuf, RuntimeError> {
    let path = root.join("native/BridgeVM");
    let info = fs::symlink_metadata(&path)?;
    if !info.is_file() || info.permissions().mode() & 0o111 == 0 {
        return Err(RuntimeError::Unavailable);
    }
    Ok(path)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Configuration<'a> {
    version: u8,
    memory_bytes: u64,
    installation_path: &'a Path,
    os_disk_path: PathBuf,
    state_disk_path: &'a Path,
    machine_id: &'a [u8],
    mac: [u8; 6],
    #[serde(rename = "createEFI")]
    create_efi: bool,
}
pub(crate) fn configuration_file(
    spec: &LaunchSpec,
    create_efi: bool,
) -> Result<File, RuntimeError> {
    let path = spec.root.join(format!(".config-{}", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&path)?;
    fs::remove_file(&path)?;
    let bytes = serde_json::to_vec(&Configuration {
        version: 1,
        memory_bytes: bridge_runtime_protocol::MEMORY_BYTES,
        installation_path: &spec.root,
        os_disk_path: spec.root.join(spec.image.disk_path()),
        state_disk_path: &spec.state_path,
        machine_id: &spec.machine_id,
        mac: spec.mac,
        create_efi,
    })
    .map_err(|_| RuntimeError::RetainedState)?;
    if bytes.len() > 65536 {
        return Err(RuntimeError::RetainedState);
    }
    file.write_all(&bytes)?;
    file.rewind()?;
    Ok(file)
}
/// Future setup factory input only: no installation ownership or VM is acquired.
pub fn machine_identifier(resource_root: &Path) -> Result<Vec<u8>, RuntimeError> {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err(RuntimeError::Unavailable);
    }
    identifier_with_deadline(resource_root, Instant::now() + Duration::from_secs(120))
}
pub(crate) fn identifier_with_deadline(root: &Path, end: Instant) -> Result<Vec<u8>, RuntimeError> {
    let path = helper_path(root)?;
    let reaper = identifier_waiter()?; // Thread creation must succeed before any Child exists.
    let (mut child, mut pipe) = spawn_guard::run_blocking(|| -> io::Result<_> {
        let mut child = Command::new(path)
            .arg("--machine-identifier")
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let pipe = child.stdout.take().unwrap();
        // Set nonblocking before returning the pipe; no read/wait holds the spawn guard.
        let fd = pipe.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            let error = io::Error::last_os_error();
            let _ = child.kill();
            return Ok((child, Err(error)));
        }
        Ok((child, Ok(pipe)))
    })?;
    let result = (|| -> io::Result<Vec<u8>> {
        let pipe = pipe
            .as_mut()
            .map_err(|e| io::Error::new(e.kind(), "identity pipe"))?;
        let mut bytes = Vec::new();
        loop {
            let remaining = end
                .checked_duration_since(Instant::now())
                .ok_or(io::ErrorKind::TimedOut)?;
            let mut event = libc::pollfd {
                fd: pipe.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let millis = remaining.as_millis().max(1).min(i32::MAX as u128) as i32;
            let ready = unsafe { libc::poll(&mut event, 1, millis) };
            if ready < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if ready == 0 {
                continue;
            }
            let mut buffer = [0; 4097];
            match pipe.read(&mut buffer[..4097 - bytes.len()]) {
                Ok(0) => break,
                Ok(n) => {
                    bytes.extend_from_slice(&buffer[..n]);
                    if bytes.len() > 4096 {
                        return Err(io::ErrorKind::InvalidData.into());
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) =>
                {
                    continue
                }
                Err(e) => return Err(e),
            }
        }
        if bytes.is_empty() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        loop {
            if let Some(status) = child.try_wait()? {
                return if status.success() {
                    Ok(bytes)
                } else {
                    Err(io::ErrorKind::InvalidData.into())
                };
            }
            if Instant::now() >= end {
                return Err(io::ErrorKind::TimedOut.into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        reap_identifier(child, Instant::now() + Duration::from_secs(10), reaper)?;
    }
    result.map_err(RuntimeError::Io)
}
fn identifier_waiter() -> io::Result<Sender<Child>> {
    let (sender, receiver) = mpsc::channel::<Child>();
    std::thread::Builder::new().spawn(move || {
        if let Ok(mut child) = receiver.recv() {
            let _ = child.wait();
        }
    })?;
    Ok(sender)
}
fn reap_identifier(
    mut child: Child,
    cleanup: Instant,
    reaper: Sender<Child>,
) -> Result<(), RuntimeError> {
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
            Err(error) => {
                reaper
                    .send(child)
                    .expect("identity waiter owns its receiver until transfer");
                return Err(RuntimeError::Io(error));
            }
        }
        if Instant::now() >= cleanup {
            reaper
                .send(child)
                .expect("identity waiter owns its receiver until transfer");
            return Err(RuntimeError::RetainedState);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[cfg(test)]
#[path = "apple_input_tests.rs"]
pub(crate) mod tests;
