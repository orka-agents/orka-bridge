//! Serialized QMP exchanges. Guest protocol bytes never use this channel.
use serde_json::{json, Value};
use std::{
    io::{self, BufRead},
    time::Instant,
};
pub const MAX_FRAME: usize = 128 * 1024;
pub const MAX_TRANSCRIPT: usize = 2 * 1024 * 1024;
pub struct Received {
    pub bytes: Vec<u8>,
    pub failure: Option<io::Error>,
}
pub trait Transport {
    fn receive(&mut self, deadline: Instant) -> io::Result<Received>;
    fn send(&mut self, bytes: &[u8], deadline: Instant) -> io::Result<()>;
    fn prepare_stop(&mut self) -> io::Result<Instant>;
    fn wait_for_stop(&mut self) -> io::Result<()>;
    fn abort(&mut self);
}
#[derive(Default)]
pub struct Transcript {
    pub rx: Vec<u8>,
    pub tx: Vec<u8>,
}
#[derive(Debug, PartialEq)]
pub struct Status {
    pub status: String,
    pub running: bool,
}
pub struct Qmp<T: Transport> {
    io: T,
    next: u64,
    active: bool,
    transcript: Transcript,
}
fn invalid() -> io::Error {
    io::ErrorKind::InvalidData.into()
}
fn remaining(until: Instant) -> io::Result<()> {
    if Instant::now() >= until {
        Err(io::ErrorKind::TimedOut.into())
    } else {
        Ok(())
    }
}
/// Retain a partial or oversized frame too; the caller records it before validation.
pub fn read_frame(reader: &mut impl BufRead) -> io::Result<Received> {
    let mut bytes = Vec::new();
    while bytes.len() <= MAX_FRAME {
        let chunk = match reader.fill_buf() {
            Ok(chunk) if !chunk.is_empty() => chunk,
            result => {
                let error = result
                    .err()
                    .unwrap_or_else(|| io::ErrorKind::UnexpectedEof.into());
                return if bytes.is_empty() {
                    Err(error)
                } else {
                    Ok(Received {
                        bytes,
                        failure: Some(error),
                    })
                };
            }
        };
        let count = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map_or(chunk.len(), |i| i + 1)
            .min(MAX_FRAME + 1 - bytes.len());
        let done = chunk[count - 1] == b'\n';
        bytes.extend_from_slice(&chunk[..count]);
        reader.consume(count);
        if done {
            break;
        }
    }
    if bytes.is_empty() {
        Err(io::ErrorKind::UnexpectedEof.into())
    } else {
        Ok(Received {
            bytes,
            failure: None,
        })
    }
}
impl<T: Transport> Qmp<T> {
    pub fn negotiate(io: T, until: Instant) -> io::Result<Self> {
        let mut q = Self {
            io,
            next: 1,
            active: true,
            transcript: Transcript::default(),
        };
        let result = (|| {
            let greeting = q.frame(until)?;
            if !greeting.get("QMP").is_some_and(Value::is_object) {
                return Err(invalid());
            }
            q.command("qmp_capabilities", until)?;
            Ok(())
        })();
        q.checked(result)?;
        Ok(q)
    }
    /// Diagnostics remain accessible without exposing mutable exchanges.
    pub fn transport(&self) -> &T {
        &self.io
    }
    pub fn transcript(&self) -> &Transcript {
        &self.transcript
    }
    fn checked<R>(&mut self, result: io::Result<R>) -> io::Result<R> {
        if result.is_err() && self.active {
            self.active = false;
            self.io.abort();
        }
        result
    }
    fn capture(&mut self, bytes: &[u8], tx: bool) -> io::Result<()> {
        let left = MAX_TRANSCRIPT - self.transcript.rx.len() - self.transcript.tx.len();
        let target = if tx {
            &mut self.transcript.tx
        } else {
            &mut self.transcript.rx
        };
        target.extend_from_slice(&bytes[..bytes.len().min(left)]);
        if bytes.len() > left {
            Err(invalid())
        } else {
            Ok(())
        }
    }
    fn usable(&self, until: Instant) -> io::Result<()> {
        if !self.active {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        remaining(until)
    }
    fn frame(&mut self, until: Instant) -> io::Result<Value> {
        self.usable(until)?;
        let received = self.io.receive(until)?;
        let bytes = received.bytes;
        self.capture(&bytes, false)?;
        if let Some(error) = received.failure {
            return Err(error);
        }
        if bytes.len() > MAX_FRAME || bytes.last() != Some(&b'\n') {
            return Err(invalid());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if !value.is_object() {
            return Err(invalid());
        }
        Ok(value)
    }
    fn command(&mut self, command: &str, until: Instant) -> io::Result<Value> {
        self.usable(until)?;
        let id = self.next;
        self.next = self.next.checked_add(1).ok_or_else(invalid)?;
        let mut bytes =
            serde_json::to_vec(&json!({"execute":command,"id":id})).map_err(|_| invalid())?;
        bytes.push(b'\n');
        self.capture(&bytes, true)?;
        self.io.send(&bytes, until)?;
        loop {
            let frame = self.frame(until)?;
            if frame.get("event").and_then(Value::as_str).is_some() && frame.get("id").is_none() {
                continue;
            }
            if frame.get("id").and_then(Value::as_u64) != Some(id) {
                return Err(invalid());
            }
            match (frame.get("return"), frame.get("error")) {
                (Some(value), None) => return Ok(value.clone()),
                (None, Some(_)) => return Err(io::Error::other("QMP command refused")),
                _ => return Err(invalid()),
            }
        }
    }
    pub fn query_status(&mut self, until: Instant) -> io::Result<Status> {
        let result = (|| {
            let value = self.command("query-status", until)?;
            Ok(Status {
                status: value
                    .get("status")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(invalid)?
                    .into(),
                running: value
                    .get("running")
                    .and_then(Value::as_bool)
                    .ok_or_else(invalid)?,
            })
        })();
        self.checked(result)
    }
    pub fn quit(&mut self, until: Instant) -> io::Result<()> {
        let result = (|| {
            self.usable(until)?;
            let original = self.io.prepare_stop()?;
            self.command("quit", until.min(original))?;
            self.io.wait_for_stop()
        })();
        self.checked(result)?;
        self.active = false;
        Ok(())
    }
}
impl<T: Transport> Drop for Qmp<T> {
    fn drop(&mut self) {
        if self.active {
            self.io.abort();
        }
    }
}
