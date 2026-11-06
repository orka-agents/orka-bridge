use super::*;
use std::os::windows::io::FromRawHandle;
fn pipe() -> (File, File) {
    unsafe {
        let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
        assert_ne!(
            windows_sys::Win32::System::Pipes::CreatePipe(
                &mut read,
                &mut write,
                std::ptr::null(),
                4096
            ),
            0
        );
        (File::from_raw_handle(read), File::from_raw_handle(write))
    }
}
#[test]
fn real_pipe_frames_and_eof_reach_the_bounded_receiver() {
    let (read, mut write) = pipe();
    let (tx, rx) = mpsc::sync_channel(2);
    let faults = Arc::new(AtomicBool::new(false));
    let observed = faults.clone();
    let worker = std::thread::spawn(move || {
        stdout_pump(read, tx, move |_| {
            observed.store(true, Ordering::Release);
        })
    });
    write.write_all(b"{\"QMP\":").unwrap();
    write.write_all(b"{}}\n").unwrap();
    drop(write);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap(),
        b"{\"QMP\":{}}\n"
    );
    assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().is_err());
    worker.join().unwrap();
    assert!(faults.load(Ordering::Acquire));
}
#[test]
fn saturated_output_queue_terminates_instead_of_waiting_for_a_reader() {
    let (read, mut write) = pipe();
    write.write_all(b"{}\n{}\n").unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let fault = Arc::new(AtomicBool::new(false));
    let seen = fault.clone();
    let worker = std::thread::spawn(move || {
        stdout_pump(read, tx, move |_| {
            seen.store(true, Ordering::Release);
        })
    });
    let until = Instant::now() + Duration::from_secs(5);
    // Do not cancel the read here: that could cause a fault before the queue
    // fills and would test cancellation rather than nonblocking overflow.
    let exited = unsafe { WaitForSingleObject(worker.as_raw_handle(), 5000) };
    let _ = finish_workers(&mut vec![worker], until);
    assert_eq!(exited, WAIT_OBJECT_0);
    assert!(fault.load(Ordering::Acquire));
    assert_eq!(rx.try_recv().unwrap().unwrap(), b"{}\n");
    drop(write);
}
#[test]
fn synchronous_read_is_cancelled_with_writer_still_open() {
    let (mut read, write) = pipe();
    let worker = std::thread::spawn(move || {
        let _ = read.read(&mut [0; 1]);
    });
    assert!(finish_workers(
        &mut vec![worker],
        Instant::now() + Duration::from_secs(5)
    ));
    drop(write);
}
#[test]
fn stderr_is_drained_after_capture_is_full() {
    let (read, mut write) = pipe();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = captured.clone();
    let worker = std::thread::spawn(move || stderr_pump(read, output, |_| {}));
    write.write_all(&vec![b'x'; MAX_TRANSCRIPT + 4096]).unwrap();
    drop(write);
    worker.join().unwrap();
    assert_eq!(captured.lock().unwrap().len(), MAX_TRANSCRIPT);
}
