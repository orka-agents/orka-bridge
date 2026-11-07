//! Revocable, single-descriptor guest capability. Wrappers never retain an FD.
use std::{
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
#[derive(Clone)]
pub(crate) struct GuestSlot {
    inner: Arc<Inner>,
}
struct Inner {
    state: Mutex<State>,
    idle: Condvar,
}
struct State {
    socket: Option<Arc<UnixStream>>,
    sealed: bool,
    active: bool,
}
struct Scoped {
    slot: GuestSlot,
    socket: Option<Arc<UnixStream>>,
}
impl Drop for Scoped {
    fn drop(&mut self) {
        // Drop the scoped FD reference BEFORE notifying the final-close observer.
        self.socket.take();
        let mut state = self.slot.inner.state.lock().unwrap();
        state.active = false;
        self.slot.inner.idle.notify_all();
    }
}
impl GuestSlot {
    pub(crate) fn new(socket: UnixStream) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    socket: Some(Arc::new(socket)),
                    sealed: false,
                    active: false,
                }),
                idle: Condvar::new(),
            }),
        }
    }
    pub(crate) fn channel(&self) -> Channel {
        Channel {
            slot: self.clone(),
            deadline: None,
        }
    }
    fn acquire(&self) -> io::Result<Scoped> {
        let mut state = self.inner.state.lock().unwrap();
        if state.sealed {
            return Err(io::ErrorKind::NotConnected.into());
        }
        if state.active {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let socket = state.socket.clone().ok_or(io::ErrorKind::NotConnected)?;
        state.active = true;
        Ok(Scoped {
            slot: self.clone(),
            socket: Some(socket),
        })
    }
    pub(crate) fn seal(&self) {
        self.inner.state.lock().unwrap().sealed = true;
    }
    pub(crate) fn revoke(&self, end: Instant) -> io::Result<()> {
        self.seal();
        let socket = self.inner.state.lock().unwrap().socket.take();
        // No mutex is held during shutdown; this wakes a blocked scoped read.
        if let Some(socket) = socket {
            // Darwin reports ENOTCONN after the observed helper has closed its end.
            if let Err(error) = socket.shutdown(Shutdown::Both) {
                if error.kind() != io::ErrorKind::NotConnected {
                    return Err(error);
                }
            }
            drop(socket);
        }
        let mut state = self.inner.state.lock().unwrap();
        while state.active {
            let remaining = end
                .checked_duration_since(Instant::now())
                .ok_or(io::ErrorKind::TimedOut)?;
            state = self.inner.idle.wait_timeout(state, remaining).unwrap().0;
        }
        Ok(())
    }
}
pub(crate) struct Channel {
    slot: GuestSlot,
    pub(crate) deadline: Option<Instant>,
}
impl Channel {
    fn budget(&self, seconds: u64) -> io::Result<Duration> {
        let budget = Duration::from_secs(seconds);
        match self.deadline {
            Some(end) => end
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .map(|d| d.min(budget))
                .ok_or_else(|| io::ErrorKind::TimedOut.into()),
            None => Ok(budget),
        }
    }
    fn io(
        &self,
        write: bool,
        action: impl FnOnce(&UnixStream) -> io::Result<usize>,
    ) -> io::Result<usize> {
        let scoped = self.slot.acquire()?;
        let socket = scoped.socket.as_ref().unwrap();
        if write {
            socket.set_write_timeout(Some(self.budget(30)?))?;
        } else {
            socket.set_read_timeout(Some(self.budget(120)?))?;
        }
        action(socket).map_err(|e| {
            if e.kind() == io::ErrorKind::WouldBlock {
                io::ErrorKind::TimedOut.into()
            } else {
                e
            }
        })
    }
}
impl Read for Channel {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.io(false, |mut s| s.read(bytes))
    }
}
impl Write for Channel {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.io(true, |mut s| s.write(bytes))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.io(true, |_| Ok(0)).map(|_| ())
    }
}
#[cfg(test)]
#[path = "apple_channel_tests.rs"]
mod tests;
