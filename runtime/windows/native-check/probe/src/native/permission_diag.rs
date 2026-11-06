//! Read-only comparison against the ACTUAL suspended child's token.
//! No target token, DACL, launch flag or inherited handle is changed. These
//! observations are diagnostic only and never substitute for a native PASS.
use serde_json::{json, Value};
use std::{
    fs, io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES, Storage::FileSystem::NtOpenDirectoryObject,
        System::Memory::NtOpenSection,
    },
    Win32::{
        Foundation::*,
        Security::{Authorization::ConvertSecurityDescriptorToStringSecurityDescriptorW, *},
        Storage::FileSystem::*,
        System::Threading::*,
    },
};

fn boolean(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn descriptor(handle: HANDLE) -> Value {
    unsafe {
        let flags =
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut size = 0;
        GetKernelObjectSecurity(handle, flags, std::ptr::null_mut(), 0, &mut size);
        if size == 0 || size > 65536 {
            return json!({"error":GetLastError()});
        }
        let mut bytes = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        if GetKernelObjectSecurity(handle, flags, bytes.as_mut_ptr().cast(), size, &mut size) == 0 {
            return json!({"error":GetLastError()});
        }
        let mut raw = std::ptr::null_mut();
        let mut chars = 0;
        if ConvertSecurityDescriptorToStringSecurityDescriptorW(
            bytes.as_ptr().cast_mut().cast(),
            1,
            flags,
            &mut raw,
            &mut chars,
        ) == 0
        {
            return json!({"error":GetLastError()});
        }
        let result = if raw.is_null() || chars == 0 || chars > 32768 {
            json!({"error":"descriptor length"})
        } else {
            let units = std::slice::from_raw_parts(raw, chars as usize);
            json!({"sddl":String::from_utf16_lossy(units).trim_end_matches('\0')})
        };
        if !raw.is_null() {
            LocalFree(raw.cast());
        }
        result
    }
}
fn file_open(path: &Path) -> Value {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let raw = CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_EXECUTE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        );
        if raw == INVALID_HANDLE_VALUE {
            return json!({"win32":GetLastError()});
        }
        let handle = OwnedHandle::from_raw_handle(raw);
        json!({"win32":0,"security":descriptor(handle.as_raw_handle())})
    }
}
fn nt_open(name: &str, rights: u32, directory: bool, security: bool) -> Value {
    let mut wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut text = UNICODE_STRING {
        Length: ((wide.len() - 1) * 2) as u16,
        MaximumLength: (wide.len() * 2) as u16,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: std::ptr::null_mut(),
        ObjectName: &mut text,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: std::ptr::null_mut(),
        SecurityQualityOfService: std::ptr::null_mut(),
    };
    unsafe {
        let mut raw = std::ptr::null_mut();
        let status = if directory {
            NtOpenDirectoryObject(&mut raw, rights, &attributes)
        } else {
            NtOpenSection(&mut raw, rights, &attributes)
        };
        if status < 0 {
            return json!({"ntstatus":format!("{:#010x}",status as u32)});
        }
        if raw.is_null() {
            return json!({"error":"successful NT open returned null"});
        }
        let handle = OwnedHandle::from_raw_handle(raw);
        if security {
            json!({"ntstatus":"0x00000000","security":descriptor(handle.as_raw_handle())})
        } else {
            json!({"ntstatus":"0x00000000"})
        }
    }
}
/// Observe the security query API without changing the existing gate or ACL.
pub(super) fn pipe_security(handle: HANDLE) {
    use windows_sys::Win32::Security::Authorization::*;
    unsafe {
        eprintln!("[PIPE-DIAG] GetKernelObjectSecurity={}", descriptor(handle));
        for (label, kind) in [("kernel", SE_KERNEL_OBJECT), ("file", SE_FILE_OBJECT)] {
            let mut sd = std::ptr::null_mut();
            let flags = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
            let status = GetSecurityInfo(
                handle,
                kind,
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut sd,
            );
            let mut control = 0;
            let mut revision = 0;
            let valid =
                !sd.is_null() && GetSecurityDescriptorControl(sd, &mut control, &mut revision) != 0;
            let mut text = std::ptr::null_mut();
            let mut chars = 0;
            let sddl = if valid
                && ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    sd, 1, flags, &mut text, &mut chars,
                ) != 0
                && !text.is_null()
                && chars > 0
                && chars <= 32768
            {
                String::from_utf16_lossy(std::slice::from_raw_parts(text, chars as usize))
                    .trim_end_matches('\0')
                    .to_owned()
            } else {
                String::new()
            };
            eprintln!(
                "[PIPE-DIAG] {}",
                json!({"type":label,"GetSecurityInfo":status,"controlValid":valid,"control":format!("{control:#x}"),"sddl":sddl})
            );
            if !text.is_null() {
                LocalFree(text.cast());
            }
            if !sd.is_null() {
                LocalFree(sd);
            }
        }
    }
}
struct Impersonation;
impl Impersonation {
    fn begin(primary: &OwnedHandle) -> io::Result<Self> {
        unsafe {
            // Fail rather than replacing an unexpected pre-existing thread token.
            let mut existing = std::ptr::null_mut();
            if OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut existing) != 0 {
                drop(OwnedHandle::from_raw_handle(existing));
                return Err(io::Error::other("diagnostic thread already impersonates"));
            }
            if GetLastError() != ERROR_NO_TOKEN {
                return Err(io::Error::last_os_error());
            }
            // This is the same authenticated identity, with REDUCED access. No
            // privilege enablement, token adjustment, or target modification.
            boolean(ImpersonateLoggedOnUser(primary.as_raw_handle()))?;
            Ok(Self)
        }
    }
}
impl Drop for Impersonation {
    fn drop(&mut self) {
        if unsafe { RevertToSelf() } == 0 {
            eprintln!("[LOADER-DIAG] FATAL: cannot revert diagnostic impersonation");
            std::process::abort();
        }
    }
}

/// Query both contexts while QEMU is still suspended. All temporary handles
/// close before returning. Files are opened only read/execute, never modified.
pub(super) fn run(process: HANDLE, runtime: &Path) -> io::Result<()> {
    let mut files = Vec::new();
    for entry in fs::read_dir(runtime.join("bin"))? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("dll") || s.eq_ignore_ascii_case("exe"))
        {
            files.push(path);
        }
    }
    if files.len() > 512 {
        return Err(io::Error::other("diagnostic file bound"));
    }
    let system =
        std::env::var_os("SystemRoot").ok_or_else(|| io::Error::other("SystemRoot missing"))?;
    for name in [
        "ntdll.dll",
        "kernel32.dll",
        "kernelbase.dll",
        "user32.dll",
        "win32u.dll",
        "advapi32.dll",
        "ucrtbase.dll",
        "WinHvPlatform.dll",
    ] {
        files.push(Path::new(&system).join("System32").join(name));
    }
    files.sort();
    let baseline: Vec<_> = files.iter().map(|p| file_open(p)).collect();
    let mut raw = std::ptr::null_mut();
    boolean(unsafe { OpenProcessToken(process, TOKEN_QUERY | TOKEN_DUPLICATE, &mut raw) })?;
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut session = 0u32;
    let mut bytes = 0;
    boolean(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenSessionId,
            (&mut session as *mut u32).cast(),
            size_of::<u32>() as u32,
            &mut bytes,
        )
    })?;
    let directories = [
        "\\KnownDlls".to_owned(),
        format!("\\Sessions\\{session}\\BaseNamedObjects"),
        format!("\\Sessions\\{session}\\BaseNamedObjects\\Restricted"),
    ];
    let sections = [
        "ntdll.dll",
        "kernel32.dll",
        "kernelbase.dll",
        "user32.dll",
        "win32u.dll",
        "advapi32.dll",
        "ucrtbase.dll",
    ]
    .map(|s| format!("\\KnownDlls\\{s}"));
    let mut objects = Vec::new();
    for name in &directories {
        for rights in [1u32, 2, READ_CONTROL] {
            objects.push((name.as_str(), rights, true));
        }
    }
    for name in &sections {
        // Query/read/execute separately: READ_CONTROL is diagnostic and isn't
        // allowed to confound a successful loader mapping-rights check.
        for rights in [1u32, 4, 8, READ_CONTROL] {
            objects.push((name.as_str(), rights, false));
        }
    }
    let base_objects: Vec<_> = objects
        .iter()
        .map(|(n, r, d)| nt_open(n, *r, *d, *r == READ_CONTROL))
        .collect();
    if unsafe { IsTokenRestricted(token.as_raw_handle()) } == 0 {
        return Err(io::Error::other(
            "diagnostic target token is not restricted",
        ));
    }
    let reduced = Impersonation::begin(&token)?;
    let restricted: Vec<_> = files.iter().map(|p| file_open(p)).collect();
    let restricted_objects: Vec<_> = objects
        .iter()
        .map(|(n, r, d)| nt_open(n, *r, *d, false))
        .collect();
    drop(reduced);
    // Emit ONLY comparisons or loader object rights, never environment values,
    // file contents, token bytes, credentials, or other processes' resources.
    let mut failures = 0;
    for ((path, base), child) in files.iter().zip(&baseline).zip(&restricted) {
        if base["win32"] != child["win32"] || child["win32"] != 0 {
            failures += 1;
            eprintln!(
                "[LOADER-DIAG] {}",
                json!({"file":path,"baseline":base,"restricted":child})
            );
        }
    }
    eprintln!(
        "[LOADER-DIAG] {}",
        json!({"filesCompared":files.len(),"nonzeroOrDifferent":failures,"targetSession":session,"targetTokenUnchanged":true,"launchUnchanged":true})
    );
    for (((name, rights, directory), base), child) in
        objects.iter().zip(&base_objects).zip(&restricted_objects)
    {
        eprintln!(
            "[LOADER-DIAG] {}",
            json!({"object":name,"directory":directory,"rights":format!("{rights:#x}"),"baseline":base,"restricted":child})
        );
    }
    Ok(())
}
