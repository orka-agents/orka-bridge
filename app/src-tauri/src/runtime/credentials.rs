//! Host-only stored-sign-in capture. No renderer binding or guest delivery.

use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncReadExt;

const MAX_CAPTURE: usize = 16 * 1024;
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq, Eq)]
pub enum CaptureError {
    Unavailable,
    Failed,
    InvalidOutput,
    TimedOut,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Supported GitHub CLI is unavailable",
            Self::Failed => "Stored GitHub sign-in capture failed",
            Self::InvalidOutput => "Stored GitHub sign-in output is invalid",
            Self::TimedOut => "Stored GitHub sign-in capture timed out",
        })
    }
}
impl std::error::Error for CaptureError {}

/// Private process output, deliberately neither serializable nor cloneable.
/// Only trusted Rust storage/delivery code should access the bytes.
pub struct CapturedGithubToken(Vec<u8>);

impl std::fmt::Debug for CapturedGithubToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CapturedGithubToken([REDACTED])")
    }
}

impl CapturedGithubToken {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

fn parse(raw: Vec<u8>) -> Result<CapturedGithubToken, CaptureError> {
    if raw.len() > MAX_CAPTURE {
        return Err(CaptureError::InvalidOutput);
    }
    let token = std::str::from_utf8(&raw)
        .map_err(|_| CaptureError::InvalidOutput)?
        .trim();
    if token.is_empty() || !token.bytes().all(|b| (33..=126).contains(&b)) {
        return Err(CaptureError::InvalidOutput);
    }
    Ok(CapturedGithubToken(token.as_bytes().to_vec()))
}

/// Explicit stored-account capture only: no login, installation, inference or fallback
/// after execution failure. Dropping this future kills the owned direct child;
/// it does not provide process-tree containment or wait for cleanup at app exit.
pub async fn capture_stored_github_token() -> Result<CapturedGithubToken, CaptureError> {
    let path = system_path()?;
    run_capture(&path, CAPTURE_TIMEOUT).await
}

fn capture_command(path: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(path);
    command
        .args(["auth", "token", "--hostname", "github.com"])
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    command
}

async fn run_capture(path: &Path, timeout: Duration) -> Result<CapturedGithubToken, CaptureError> {
    tokio::time::timeout(timeout, async {
        let mut child = capture_command(path).spawn().map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                CaptureError::Unavailable
            } else {
                CaptureError::Failed
            }
        })?;
        let stdout = child.stdout.take().ok_or(CaptureError::Failed)?;
        let mut raw = Vec::new();
        stdout
            .take((MAX_CAPTURE + 1) as u64)
            .read_to_end(&mut raw)
            .await
            .map_err(|_| CaptureError::Failed)?;
        if raw.len() > MAX_CAPTURE {
            return Err(CaptureError::InvalidOutput);
        }
        let status = child.wait().await.map_err(|_| CaptureError::Failed)?;
        if !status.success() {
            return Err(CaptureError::Failed);
        }
        parse(raw)
    })
    .await
    .map_err(|_| CaptureError::TimedOut)?
}

fn system_path() -> Result<PathBuf, CaptureError> {
    #[cfg(target_os = "macos")]
    {
        mac_path(|path| std::fs::symlink_metadata(path).map(|_| ()))
    }
    #[cfg(windows)]
    {
        program_files().map(|folder| windows_path(&folder))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err(CaptureError::Unavailable)
    }
}

#[cfg(any(target_os = "macos", test))]
fn mac_path(mut probe: impl FnMut(&Path) -> io::Result<()>) -> Result<PathBuf, CaptureError> {
    let primary = Path::new("/opt/homebrew/bin/gh");
    match probe(primary) {
        Ok(()) => Ok(primary.into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let secondary = Path::new("/usr/local/bin/gh");
            probe(secondary).map_err(|_| CaptureError::Unavailable)?;
            Ok(secondary.into())
        }
        Err(_) => Err(CaptureError::Unavailable),
    }
}

#[cfg(any(windows, test))]
fn windows_path(folder: &Path) -> PathBuf {
    folder.join("GitHub CLI").join("gh.exe")
}

#[cfg(windows)]
fn program_files() -> Result<PathBuf, CaptureError> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, ptr};
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_ProgramFilesX64, SHGetKnownFolderPath},
    };
    let mut raw = ptr::null_mut();
    // The OS owns the null-terminated UTF-16 allocation until CoTaskMemFree.
    let status =
        unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramFilesX64, 0, ptr::null_mut(), &mut raw) };
    if status < 0 || raw.is_null() {
        unsafe {
            CoTaskMemFree(raw.cast());
        }
        return Err(CaptureError::Unavailable);
    }
    let mut len = 0;
    unsafe {
        while *raw.add(len) != 0 {
            len += 1;
        }
    }
    let folder = unsafe { OsString::from_wide(std::slice::from_raw_parts(raw, len)) };
    unsafe {
        CoTaskMemFree(raw.cast());
    }
    Ok(folder.into())
}

#[cfg(test)]
#[path = "credential_tests.rs"]
mod tests;
