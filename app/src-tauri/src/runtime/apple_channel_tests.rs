use super::*;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};
#[test]
fn surviving_wrapper_has_no_fd_and_blocked_read_stops_without_io_lock_deadlock() {
    // Bound every wait, so a missed wake fails here and names the wait instead of
    // stalling the test binary until the CI job is cancelled.
    let bound = Duration::from_secs(10);
    let (socket, mut peer) = UnixStream::pair().unwrap();
    peer.set_read_timeout(Some(bound)).unwrap();
    let slot = GuestSlot::new(socket);
    let mut client = slot.channel();
    client.deadline = Some(Instant::now() + bound);
    let (configured, ready) = std::sync::mpsc::channel();
    let (finished, read) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut b = [0];
        let result = client.io(false, |mut socket| {
            configured.send(()).unwrap(); // Timeout is configured before revocation.
            socket.read(&mut b)
        });
        finished.send(result).unwrap();
    });
    ready
        .recv_timeout(bound)
        .expect("the reader configured its read");
    let mut second = slot.channel();
    assert_eq!(
        second.write(b"x").unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    slot.seal();
    // Shutdown must wake the read, but its thread need not be scheduled within
    // a cleanup budget. An active scoped reference may legitimately outlive it.
    if let Err(error) = slot.revoke(Instant::now()) {
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    }
    let woken = read
        .recv_timeout(bound * 2)
        .expect("revocation woke the blocked read");
    assert_eq!(woken.expect("the woken read reported end of file"), 0);
    reader.join().unwrap();
    slot.revoke(Instant::now()).unwrap();
    assert_eq!(
        second.read(&mut [0]).unwrap_err().kind(),
        std::io::ErrorKind::NotConnected
    );
    assert_eq!(
        peer.read(&mut [0])
            .expect("the peer saw end of file after revocation"),
        0
    );
}
#[test]
fn scoped_reference_timeout_never_reports_success() {
    let (socket, _) = UnixStream::pair().unwrap();
    let slot = GuestSlot::new(socket);
    let scoped = slot.acquire().unwrap();
    let socket = Arc::downgrade(scoped.socket.as_ref().unwrap());
    slot.seal();
    assert_eq!(
        slot.revoke(Instant::now()).unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
    assert!(
        socket.upgrade().is_some(),
        "a scoped I/O reference still owns the descriptor"
    );
    drop(scoped);
    slot.revoke(Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert!(
        socket.upgrade().is_none(),
        "the actual UnixStream was dropped and its descriptor closed"
    );
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
