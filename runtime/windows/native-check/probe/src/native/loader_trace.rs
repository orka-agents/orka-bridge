//! CI-only diagnostic debugger. No privilege enablement, token adjustment,
//! memory writes, extra ResumeThread, or ACL changes. Never qualification evidence.
use crate::runtime::windows::security::Handle;
use std::{
    io,
    mem::zeroed,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{Diagnostics::Debug::*, Threading::GetProcessId},
};

pub(super) fn enabled() -> io::Result<bool> {
    std::env::current_exe()?
        .with_file_name("loader-trace.enabled")
        .try_exists()
}
pub(super) fn start(process: Arc<Handle>) -> io::Result<()> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || unsafe {
        let pid = GetProcessId(process.raw());
        if DebugActiveProcess(pid) == 0 {
            let _ = tx.send(Err(io::Error::last_os_error()));
            return;
        }
        // Keep default debugger kill-on-exit: an aborted worker must not leave
        // its debuggee running. Existing inner/outer kill-on-close Jobs also stay.
        let _ = tx.send(Ok(()));
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut bytes_logged = 0usize;
        loop {
            if Instant::now() >= deadline {
                eprintln!("[LOADER-TRACE] deadline; detaching owned debuggee");
                break;
            }
            let mut event: DEBUG_EVENT = zeroed();
            if WaitForDebugEventEx(&mut event, 200) == 0 {
                let error = GetLastError();
                if error == ERROR_SEM_TIMEOUT {
                    continue;
                }
                eprintln!("[LOADER-TRACE] wait Win32={error}");
                break;
            }
            let mut disposition = DBG_CONTINUE;
            let mut exited = false;
            match event.dwDebugEventCode {
                CREATE_PROCESS_DEBUG_EVENT => {
                    let info = event.u.CreateProcessInfo;
                    if !info.hFile.is_null() && info.hFile != INVALID_HANDLE_VALUE {
                        CloseHandle(info.hFile);
                    }
                    // Debug-system process/thread handles close on their EXIT
                    // event continuation; don't double-close them ourselves.
                    eprintln!("[LOADER-TRACE] debugger attached to owned PID={pid}");
                }
                LOAD_DLL_DEBUG_EVENT => {
                    let info = event.u.LoadDll;
                    if !info.hFile.is_null() && info.hFile != INVALID_HANDLE_VALUE {
                        CloseHandle(info.hFile);
                    }
                }
                OUTPUT_DEBUG_STRING_EVENT => {
                    let info = event.u.DebugString;
                    let units = info.nDebugStringLength as usize;
                    let bytes = units.saturating_mul(if info.fUnicode != 0 { 2 } else { 1 });
                    if bytes > 0
                        && bytes <= 16384
                        && bytes_logged.saturating_add(bytes) <= 49152
                        && !info.lpDebugStringData.is_null()
                    {
                        let mut data = vec![0u8; bytes];
                        let mut read = 0;
                        if ReadProcessMemory(
                            process.raw(),
                            info.lpDebugStringData.cast(),
                            data.as_mut_ptr().cast(),
                            bytes,
                            &mut read,
                        ) != 0
                            && read == bytes
                        {
                            let message = if info.fUnicode != 0 {
                                let wide: Vec<u16> = data
                                    .as_chunks::<2>()
                                    .0
                                    .iter()
                                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                                    .collect();
                                String::from_utf16_lossy(&wide)
                            } else {
                                String::from_utf8_lossy(&data).into_owned()
                            };
                            bytes_logged += bytes;
                            eprintln!("[LOADER-TRACE] {}", message.trim_end_matches('\0'));
                        }
                    }
                }
                EXCEPTION_DEBUG_EVENT => {
                    let info = event.u.Exception;
                    let code = info.ExceptionRecord.ExceptionCode;
                    eprintln!(
                        "[LOADER-TRACE] exception={:#010x}, firstChance={}",
                        code as u32, info.dwFirstChance
                    );
                    // Consume only the debugger's breakpoint. All other faults
                    // keep normal unhandled-exception disposition.
                    if code != EXCEPTION_BREAKPOINT {
                        disposition = DBG_EXCEPTION_NOT_HANDLED;
                    }
                }
                EXIT_PROCESS_DEBUG_EVENT => {
                    eprintln!(
                        "[LOADER-TRACE] process exit={:#010x}",
                        event.u.ExitProcess.dwExitCode
                    );
                    exited = true;
                }
                _ => {}
            }
            if ContinueDebugEvent(event.dwProcessId, event.dwThreadId, disposition) == 0 {
                eprintln!("[LOADER-TRACE] continue Win32={}", GetLastError());
                break;
            }
            if exited {
                return;
            }
        }
        if DebugActiveProcessStop(pid) == 0 {
            eprintln!("[LOADER-TRACE] detach Win32={}", GetLastError());
        }
    });
    rx.recv_timeout(Duration::from_secs(10))
        .map_err(io::Error::other)?
}
