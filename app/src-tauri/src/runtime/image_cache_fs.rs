use super::*;

pub(super) fn sync(dir: &Dir) -> Result<(), ImageError> {
    #[cfg(unix)]
    let file = dir.open(".").map_err(storage)?.into_std();
    #[cfg(windows)]
    let file = {
        use cap_std::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .follow(FollowSymlinks::No)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
        dir.open_with(".", &options).map_err(storage)?.into_std()
    };
    not_reparse(&file)?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // Namespace flush: F_FULLFSYNC is for regular data files on Apple, not directories.
        if unsafe { libc::fsync(file.as_raw_fd()) } != 0 {
            return Err(storage(io::Error::last_os_error()));
        }
        Ok(())
    }
    #[cfg(windows)]
    file.sync_all().map_err(storage)
}
pub(super) fn link(dir: &Dir, from: &str, to: &str) -> Result<(), ImageError> {
    #[cfg(not(windows))]
    return dir.hard_link(from, dir, to).map_err(storage);
    #[cfg(windows)]
    set_name(dir, from, to, 11)
}
pub(super) fn publish(dir: &Dir) -> Result<(), ImageError> {
    #[cfg(windows)]
    return set_name(dir, "staging", "generation-1", 10);
    #[cfg(unix)]
    {
        use std::{ffi::CString, os::fd::AsRawFd};
        let from = CString::new("staging").unwrap();
        let to = CString::new("generation-1").unwrap();
        #[cfg(target_os = "linux")]
        // Both fixed leaves are relative to the retained parent; never replace a generation.
        let result = unsafe {
            libc::renameat2(
                dir.as_raw_fd(),
                from.as_ptr(),
                dir.as_raw_fd(),
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                dir.as_raw_fd(),
                from.as_ptr(),
                dir.as_raw_fd(),
                to.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return Err(ImageError::Unsupported);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if result != 0 {
            return Err(storage(io::Error::last_os_error()));
        }
        Ok(())
    }
}
#[cfg(windows)]
fn set_name(dir: &Dir, from: &str, to: &str, class: i32) -> Result<(), ImageError> {
    use cap_std::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::{
        Wdk::Storage::FileSystem::NtSetInformationFile,
        Win32::{
            Foundation::{RtlNtStatusToDosError, HANDLE},
            Storage::FileSystem::{DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ},
            System::IO::IO_STATUS_BLOCK,
        },
    };
    // Legacy FILE_LINK_INFORMATION / FILE_RENAME_INFORMATION share this fixed header.
    #[repr(C)]
    struct NameInfo {
        replace: u8,
        root: HANDLE,
        bytes: u32,
        name: [u16; 80],
    }
    let mut info = NameInfo {
        replace: 0,
        root: dir.as_raw_handle(),
        bytes: 0,
        name: [0; 80],
    };
    let name: Vec<_> = to.encode_utf16().collect();
    if name.len() > info.name.len() {
        return Err(ImageError::UnsafePath);
    }
    info.name[..name.len()].copy_from_slice(&name);
    info.bytes = (name.len() * 2) as u32;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .access_mode(FILE_GENERIC_READ | DELETE)
        .follow(FollowSymlinks::No)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    let file = dir.open_with(from, &options).map_err(storage)?.into_std();
    not_reparse(&file)?;
    let mut status = IO_STATUS_BLOCK::default();
    // RootDirectory anchors the destination to the held parent, not a reconstructed path.
    let result = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            &mut status,
            (&info as *const NameInfo).cast(),
            (std::mem::offset_of!(NameInfo, name) + name.len() * 2) as u32,
            class,
        )
    };
    if result != 0 {
        return Err(storage(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        )));
    }
    Ok(())
}
