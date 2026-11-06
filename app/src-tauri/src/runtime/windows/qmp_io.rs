use crate::runtime::windows::{
    job::OwnedQemuProcess,
    qmp::{read_frame, Transport, MAX_TRANSCRIPT},
};
use std::{
    fs::File,
    io::{self, BufReader, Read, Write},
    os::windows::io::AsRawHandle,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{Threading::WaitForSingleObject, IO::CancelSynchronousIo},
};
type Frame = io::Result<Vec<u8>>;
type WriteRequest = (Vec<u8>, SyncSender<io::Result<()>>);
pub struct NativeTransport {
    owner: Arc<OwnedQemuProcess>,
    incoming: Option<Receiver<Frame>>,
    outgoing: Option<SyncSender<WriteRequest>>,
    workers: Vec<JoinHandle<()>>,
    stopping: Arc<AtomicBool>,
    quitting: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    stderr: Arc<Mutex<Vec<u8>>>,
}
fn stdout_pump(file: File, tx: SyncSender<Frame>, mut fault: impl FnMut(&io::Error)) {
    let mut reader = BufReader::new(file);
    loop {
        match read_frame(&mut reader) {
            Ok(bytes) => {
                if tx.try_send(Ok(bytes)).is_err() {
                    fault(&io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "QMP output queue unavailable",
                    ));
                    break;
                }
            }
            Err(error) => {
                fault(&error);
                let _ = tx.try_send(Err(error));
                break;
            }
        }
    }
}
fn stderr_pump(mut file: File, capture: Arc<Mutex<Vec<u8>>>, mut fault: impl FnMut(&io::Error)) {
    let mut bytes = [0; 4096];
    loop {
        match file.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => match capture.lock() {
                Ok(mut output) => {
                    let count = n.min(MAX_TRANSCRIPT - output.len());
                    output.extend_from_slice(&bytes[..count]);
                }
                Err(_) => {
                    fault(&io::Error::other("QMP stderr capture poisoned"));
                    break;
                }
            },
            Err(error) => {
                fault(&error);
                break;
            }
        }
    }
}
// A cancellation before entry to synchronous I/O can miss it. Kernel waits and
// repeated cancellation share the existing stop deadline, never a new timeout.
fn finish_workers(workers: &mut Vec<JoinHandle<()>>, until: Instant) -> bool {
    for worker in workers.iter() {
        loop {
            let handle = worker.as_raw_handle();
            unsafe {
                CancelSynchronousIo(handle);
            }
            let left = until.saturating_duration_since(Instant::now());
            let result = unsafe {
                WaitForSingleObject(
                    handle,
                    left.min(Duration::from_millis(50)).as_millis() as u32,
                )
            };
            if result == WAIT_OBJECT_0 {
                break;
            }
            if result != WAIT_TIMEOUT || left.is_zero() {
                return false;
            }
        }
    }
    // Only joined after the kernel reports complete thread exit, including TLS.
    while let Some(worker) = workers.pop() {
        let _ = worker.join();
    }
    true
}
impl NativeTransport {
    /// Take these noninheritable stdio copies before wrapping the process in Arc.
    /// Resume the retained child before negotiating QMP; never rediscover pipes.
    pub fn new(owner: Arc<OwnedQemuProcess>, stdio: (File, File, File)) -> io::Result<Self> {
        let (mut stdin, stdout, stderr) = stdio;
        let (frames, receive) = mpsc::sync_channel(32);
        let (send, requests) = mpsc::sync_channel::<WriteRequest>(1);
        let mut io = Self {
            owner,
            incoming: Some(receive),
            outgoing: Some(send),
            workers: Vec::new(),
            stopping: Arc::new(AtomicBool::new(false)),
            quitting: Arc::new(AtomicBool::new(false)),
            failed: Arc::new(AtomicBool::new(false)),
            stderr: Arc::new(Mutex::new(Vec::new())),
        };
        let owner = io.owner.clone();
        let stopping = io.stopping.clone();
        let quitting = io.quitting.clone();
        let failed = io.failed.clone();
        io.spawn("qmp-output", move || {
            stdout_pump(stdout, frames, move |error| {
                if !stopping.load(Ordering::Acquire)
                    && !(quitting.load(Ordering::Acquire)
                        && error.kind() == io::ErrorKind::UnexpectedEof)
                {
                    failed.store(true, Ordering::Release);
                    let _ = owner.abort();
                }
            })
        })?;
        let owner = io.owner.clone();
        let stopping = io.stopping.clone();
        let failed = io.failed.clone();
        io.spawn("qmp-input", move || {
            while let Ok((bytes, reply)) = requests.recv() {
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                let result = stdin.write_all(&bytes).and_then(|_| stdin.flush());
                let broken = result.is_err();
                let _ = reply.try_send(result);
                if broken {
                    if !stopping.load(Ordering::Acquire) {
                        failed.store(true, Ordering::Release);
                        let _ = owner.abort();
                    }
                    break;
                }
            }
        })?;
        let owner = io.owner.clone();
        let stopping = io.stopping.clone();
        let capture = io.stderr.clone();
        let failed = io.failed.clone();
        io.spawn("qmp-stderr", move || {
            stderr_pump(stderr, capture, move |_| {
                if !stopping.load(Ordering::Acquire) {
                    failed.store(true, Ordering::Release);
                    let _ = owner.abort();
                }
            })
        })?;
        Ok(io)
    }
    fn spawn(&mut self, name: &str, action: impl FnOnce() + Send + 'static) -> io::Result<()> {
        let owner = self.owner.clone();
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _owner = owner;
                action();
            })?;
        self.workers.push(thread);
        Ok(())
    }
    pub fn stderr(&self) -> Arc<Mutex<Vec<u8>>> {
        self.stderr.clone()
    }
}
fn left(until: Instant) -> io::Result<Duration> {
    let left = until.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Err(io::ErrorKind::TimedOut.into())
    } else {
        Ok(left)
    }
}
impl Transport for NativeTransport {
    fn receive(&mut self, until: Instant) -> io::Result<Vec<u8>> {
        if self.failed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let result = self
            .incoming
            .as_ref()
            .ok_or(io::ErrorKind::BrokenPipe)?
            .recv_timeout(left(until)?)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => io::ErrorKind::TimedOut,
                mpsc::RecvTimeoutError::Disconnected => io::ErrorKind::UnexpectedEof,
            })?;
        if self.failed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        result
    }
    fn send(&mut self, bytes: &[u8], until: Instant) -> io::Result<()> {
        if self.failed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        left(until)?;
        let (reply, result) = mpsc::sync_channel(1);
        self.outgoing
            .as_ref()
            .ok_or(io::ErrorKind::BrokenPipe)?
            .try_send((bytes.to_vec(), reply))
            .map_err(|_| io::ErrorKind::BrokenPipe)?;
        result
            .recv_timeout(left(until)?)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => io::ErrorKind::TimedOut,
                mpsc::RecvTimeoutError::Disconnected => io::ErrorKind::BrokenPipe,
            })?
    }
    fn prepare_stop(&mut self) -> io::Result<Instant> {
        self.owner.prepare_graceful_stop()?;
        self.quitting.store(true, Ordering::Release);
        Ok(self.owner.stop_deadline())
    }
    fn wait_for_stop(&mut self) -> io::Result<()> {
        self.owner.wait_for_stop()
    }
    fn abort(&mut self) {
        let _ = self.owner.abort();
    }
}
impl Drop for NativeTransport {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        drop(self.outgoing.take());
        drop(self.incoming.take());
        let stopped = self.owner.abort().is_ok();
        let finished = finish_workers(&mut self.workers, self.owner.stop_deadline());
        if !stopped || !finished {
            // Keep the entire original bundle if either process or I/O teardown
            // is unobserved. Pending workers also keep their exact pipe copies.
            std::mem::forget(self.owner.clone());
            std::mem::forget(std::mem::take(&mut self.workers));
        }
    }
}
#[cfg(test)]
#[path = "qmp_io_tests.rs"]
mod tests;
