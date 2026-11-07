use crate::runtime::apple_channel::*;
impl GuestSlot {
    fn active(&self) -> bool {
        self.inner.state.lock().unwrap().active
    }
}
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};
#[test]
fn surviving_wrapper_has_no_fd_and_blocked_read_stops_without_io_lock_deadlock() {
    let (socket, mut peer) = UnixStream::pair().unwrap();
    let slot = GuestSlot::new(socket);
    let mut client = slot.channel();
    let reader = std::thread::spawn(move || {
        let mut b = [0];
        client.read(&mut b)
    });
    // Synchronize on actual scoped I/O, not a guessed sleep.
    while !slot.active() {
        std::thread::yield_now();
    }
    let mut second = slot.channel();
    assert_eq!(
        second.write(b"x").unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    slot.seal();
    slot.revoke(Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert!(reader.join().unwrap().is_ok());
    assert_eq!(
        second.read(&mut [0]).unwrap_err().kind(),
        std::io::ErrorKind::NotConnected
    );
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
}
#[test]
fn scoped_reference_timeout_never_reports_success() {
    let (socket, _) = UnixStream::pair().unwrap();
    let slot = GuestSlot::new(socket);
    let scoped = slot.acquire().unwrap();
    slot.seal();
    assert_eq!(
        slot.revoke(Instant::now()).unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
    drop(scoped);
    slot.revoke(Instant::now() + Duration::from_secs(1))
        .unwrap();
}
#[test]
fn drip_reads_do_not_renew_total_deadline() {
    let (socket, mut peer) = UnixStream::pair().unwrap();
    let slot = GuestSlot::new(socket);
    let mut channel = slot.channel();
    channel.deadline = Some(Instant::now() + Duration::from_millis(60));
    let writer = std::thread::spawn(move || {
        for _ in 0..20 {
            if peer.write_all(b"x").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    });
    assert_eq!(
        channel.read_exact(&mut [0; 20]).unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
    slot.seal();
    slot.revoke(Instant::now() + Duration::from_secs(1))
        .unwrap();
    writer.join().unwrap();
}
