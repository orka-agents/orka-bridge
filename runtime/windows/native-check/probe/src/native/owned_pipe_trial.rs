//! Capability trial only: can the pinned pipe backend attach to a host-owned server?
use super::*;
use std::{
    cell::UnsafeCell,
    os::windows::io::{FromRawHandle, OwnedHandle},
};
use windows_sys::Win32::{
    Security::{Authorization::*, *},
    System::IO::*,
};

pub(super) struct Listener {
    pipe: OwnedHandle,
    event: OwnedHandle,
    overlapped: Box<UnsafeCell<OVERLAPPED>>,
    pending: bool,
}
impl Listener {
    pub(super) fn create(name: &str) -> io::Result<Self> {
        unsafe {
            let (_, sid) = controller::caller()?;
            let sddl = wide(&format!("O:{sid}D:P(A;;GA;;;SY)(A;;GA;;;{sid})"));
            let mut sd = std::ptr::null_mut();
            win(ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut sd,
                std::ptr::null_mut(),
            ))?;
            let sa = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd,
                bInheritHandle: 0,
            };
            let path = wide(&format!("\\\\.\\pipe\\{name}"));
            let raw = CreateNamedPipeW(
                path.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                4096,
                0,
                &sa,
            );
            let result = if raw == INVALID_HANDLE_VALUE {
                Err(io::Error::last_os_error())
            } else {
                Ok(OwnedHandle::from_raw_handle(raw))
            };
            LocalFree(sd);
            let pipe = result?;
            // Reuse the exact imported production validator, not a weaker trial validator.
            crate::proof::check_pipe(pipe.as_raw_handle())?;
            let mut inherit = 0;
            win(GetHandleInformation(pipe.as_raw_handle(), &mut inherit))?;
            check(inherit & HANDLE_FLAG_INHERIT == 0)?;
            let raw = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
            if raw.is_null() {
                return Err(io::Error::last_os_error());
            }
            let event = OwnedHandle::from_raw_handle(raw);
            // The kernel writes completion fields asynchronously: retain the cell
            // through completion and never lend a Rust reference over that mutation.
            let overlapped: Box<UnsafeCell<OVERLAPPED>> = Box::new(UnsafeCell::new(zeroed()));
            (*overlapped.get()).hEvent = event.as_raw_handle();
            let ok = ConnectNamedPipe(pipe.as_raw_handle(), overlapped.get());
            let error = if ok == 0 { GetLastError() } else { 0 };
            if ok == 0 && error != ERROR_IO_PENDING && error != ERROR_PIPE_CONNECTED {
                return Err(io::Error::from_raw_os_error(error as i32));
            }
            Ok(Self {
                pipe,
                event,
                overlapped,
                pending: error == ERROR_IO_PENDING,
            })
        }
    }
    pub(super) fn connected(&mut self, qemu: HANDLE) -> io::Result<bool> {
        unsafe {
            if self.pending {
                let mut bytes = 0;
                if GetOverlappedResult(
                    self.pipe.as_raw_handle(),
                    self.overlapped.get(),
                    &mut bytes,
                    0,
                ) == 0
                {
                    let error = GetLastError();
                    if error == ERROR_IO_INCOMPLETE {
                        return Ok(false);
                    }
                    if error == ERROR_OPERATION_ABORTED {
                        self.pending = false;
                    }
                    return Err(io::Error::from_raw_os_error(error as i32));
                }
                self.pending = false;
            }
            let mut pid = 0;
            win(GetNamedPipeClientProcessId(
                self.pipe.as_raw_handle(),
                &mut pid,
            ))?;
            check(pid == GetProcessId(qemu))?;
            Ok(true)
        }
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        if !self.pending {
            return;
        }
        unsafe {
            // CancelIoEx is not a buffer-lifetime barrier. Observe terminal completion
            // before allowing either the OVERLAPPED or its event/pipe to be freed.
            if CancelIoEx(self.pipe.as_raw_handle(), self.overlapped.get()) == 0
                && GetLastError() != ERROR_NOT_FOUND
            {
                std::process::abort();
            }
            if WaitForSingleObject(self.event.as_raw_handle(), 5000) != WAIT_OBJECT_0 {
                std::process::abort();
            }
            let mut bytes = 0;
            if GetOverlappedResult(
                self.pipe.as_raw_handle(),
                self.overlapped.get(),
                &mut bytes,
                0,
            ) == 0
                && GetLastError() != ERROR_OPERATION_ABORTED
            {
                std::process::abort();
            }
            self.pending = false;
        }
    }
}
