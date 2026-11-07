#[path = "../../../app/src-tauri/src/runtime/windows/qmp.rs"]
mod qmp;
use qmp::{Qmp, Received, Transport};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Default)]
struct State {
    writes: Vec<Vec<u8>>,
    actions: Vec<&'static str>,
}
struct Wire {
    rx: VecDeque<Received>,
    state: Arc<Mutex<State>>,
    stop: Instant,
}
impl Transport for Wire {
    fn receive(&mut self, _: Instant) -> io::Result<Received> {
        self.rx
            .pop_front()
            .ok_or_else(|| io::ErrorKind::UnexpectedEof.into())
    }
    fn send(&mut self, bytes: &[u8], _: Instant) -> io::Result<()> {
        let mut s = self.state.lock().unwrap();
        s.actions.push("send");
        s.writes.push(bytes.to_vec());
        Ok(())
    }
    fn prepare_stop(&mut self) -> io::Result<Instant> {
        self.state.lock().unwrap().actions.push("prepare");
        Ok(self.stop)
    }
    fn wait_for_stop(&mut self) -> io::Result<()> {
        self.state.lock().unwrap().actions.push("observed");
        Ok(())
    }
    fn abort(&mut self) {
        self.state.lock().unwrap().actions.push("abort");
    }
}
fn wire(frames: &[&[u8]]) -> (Wire, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    (
        Wire {
            rx: frames
                .iter()
                .map(|b| Received {
                    bytes: b.to_vec(),
                    failure: None,
                })
                .collect(),
            state: state.clone(),
            stop: Instant::now() + Duration::from_secs(45),
        },
        state,
    )
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
const HELLO: &[u8] = b"{\"QMP\":{\"version\":{},\"capabilities\":[]}}\r\n";
const CAPS: &[u8] = b"{\"return\":{},\"id\":1}\n";
#[test]
fn guest_shutdown_prepares_the_original_budget_without_sending_quit() {
    let (wire, state) = wire(&[HELLO, CAPS]);
    let original = wire.stop;
    let mut q = Qmp::negotiate(wire, deadline()).unwrap();
    assert_eq!(q.prepare_stop().unwrap(), original);
    assert_eq!(state.lock().unwrap().actions, ["send", "prepare"]);
}
#[test]
fn cpu_control_is_correlated_and_observes_requested_state() {
    let (wire, state) = wire(&[
        HELLO,
        CAPS,
        b"{\"return\":{},\"id\":2}\n",
        b"{\"return\":{\"status\":\"running\",\"running\":true},\"id\":3}\n",
        b"{\"return\":{},\"id\":4}\n",
        b"{\"return\":{\"status\":\"paused\",\"running\":false},\"id\":5}\n",
    ]);
    let mut q = Qmp::negotiate(wire, deadline()).unwrap();
    q.set_running(true, deadline()).unwrap();
    q.set_running(false, deadline()).unwrap();
    let s = state.lock().unwrap();
    let requests: Vec<serde_json::Value> = s
        .writes
        .iter()
        .map(|b| serde_json::from_slice(b).unwrap())
        .collect();
    assert_eq!(
        requests
            .iter()
            .map(|v| v["execute"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "qmp_capabilities",
            "cont",
            "query-status",
            "stop",
            "query-status"
        ]
    );
}
#[test]
fn cpu_control_cannot_succeed_on_unobserved_running_state() {
    let (wire, state) = wire(&[
        HELLO,
        CAPS,
        b"{\"return\":{},\"id\":2}\n",
        b"{\"return\":{\"status\":\"paused\",\"running\":false},\"id\":3}\n",
    ]);
    let mut q = Qmp::negotiate(wire, deadline()).unwrap();
    assert!(q.set_running(true, deadline()).is_err());
    assert!(state.lock().unwrap().actions.contains(&"abort"));
}
#[test]
fn negotiates_correlates_events_status_and_prepares_before_quit() {
    let (wire, state) = wire(&[
        HELLO,
        b"{\"event\":\"RESET\"}\n",
        CAPS,
        b"{\"return\":{\"status\":\"prelaunch\",\"running\":false},\"id\":2}\n",
        b"{\"event\":\"SHUTDOWN\"}\n",
        b"{\"return\":{},\"id\":3}\n",
    ]);
    let mut q = Qmp::negotiate(wire, deadline()).unwrap();
    let status = q.query_status(deadline()).unwrap();
    assert_eq!(status.status, "prelaunch");
    assert!(!status.running);
    q.quit(deadline()).unwrap();
    assert!(q.query_status(deadline()).is_err());
    let s = state.lock().unwrap();
    let requests: Vec<serde_json::Value> = s
        .writes
        .iter()
        .map(|b| serde_json::from_slice(b).unwrap())
        .collect();
    assert_eq!(
        requests
            .iter()
            .map(|v| v["execute"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["qmp_capabilities", "query-status", "quit"]
    );
    assert_eq!(s.actions, ["send", "send", "prepare", "send", "observed"]);
}
#[test]
fn wrong_id_or_error_is_terminal_and_raw_rx_is_retained_before_parse() {
    for frame in [
        b"{\"return\":{},\"id\":99}\n".as_slice(),
        b"not-json\n",
        b"{\"error\":{},\"id\":2}\n",
        b"{\"return\":{},\"id\":2}",
    ] {
        let (wire, state) = wire(&[HELLO, CAPS, frame]);
        let mut q = Qmp::negotiate(wire, deadline()).unwrap();
        assert!(q.query_status(deadline()).is_err());
        assert!(q.transcript().rx.ends_with(frame));
        let count = state.lock().unwrap().writes.len();
        assert!(q.query_status(deadline()).is_err());
        assert_eq!(state.lock().unwrap().writes.len(), count);
        assert!(state.lock().unwrap().actions.contains(&"abort"));
    }
}
#[test]
fn expired_original_stop_deadline_sends_no_quit_and_aborts() {
    let (mut wire, state) = wire(&[HELLO, CAPS]);
    wire.stop = Instant::now();
    let mut q = Qmp::negotiate(wire, deadline()).unwrap();
    assert!(q.quit(deadline()).is_err());
    let s = state.lock().unwrap();
    assert_eq!(s.writes.len(), 1);
    assert!(s.actions.contains(&"abort"));
}
#[test]
fn fragmented_and_oversized_frames_are_bounded_by_real_reader() {
    let raw = b"{\"return\":{},\"id\":1}\n";
    let mut reader = io::BufReader::with_capacity(2, io::Cursor::new(raw));
    assert_eq!(qmp::read_frame(&mut reader).unwrap().bytes, raw);
    let mut reader = io::BufReader::new(io::Cursor::new(vec![b'x'; qmp::MAX_FRAME + 10]));
    assert_eq!(
        qmp::read_frame(&mut reader).unwrap().bytes.len(),
        qmp::MAX_FRAME + 1
    );
}
#[test]
fn readonly_transport_diagnostics_survive_an_exchange_failure() {
    let (wire, state) = wire(&[HELLO, CAPS, b"invalid\n"]);
    let mut q = Qmp::negotiate(wire, deadline()).unwrap();
    assert!(q.query_status(deadline()).is_err());
    let transport: &Wire = q.transport();
    assert!(Arc::ptr_eq(&transport.state, &state));
}
#[test]
fn known_transport_failure_captures_bytes_without_accepting_a_valid_reply() {
    let (mut wire, state) = wire(&[HELLO, CAPS]);
    let bytes = b"{\"return\":{\"status\":\"running\",\"running\":true},\"id\":2}\n".to_vec();
    wire.rx.push_back(Received {
        bytes: bytes.clone(),
        failure: Some(io::ErrorKind::BrokenPipe.into()),
    });
    let mut q = Qmp::negotiate(wire, deadline()).unwrap();
    assert_eq!(
        q.query_status(deadline()).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert!(q.transcript().rx.ends_with(&bytes));
    assert!(state.lock().unwrap().actions.contains(&"abort"));
}
#[test]
fn refill_error_keeps_the_consumed_fragment() {
    struct Broken {
        consumed: bool,
    }
    impl io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            unreachable!()
        }
    }
    impl io::BufRead for Broken {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            if self.consumed {
                Err(io::ErrorKind::Interrupted.into())
            } else {
                Ok(b"partial")
            }
        }
        fn consume(&mut self, count: usize) {
            assert_eq!(count, 7);
            self.consumed = true;
        }
    }
    let result = qmp::read_frame(&mut Broken { consumed: false });
    assert!(
        result.is_ok(),
        "consumed bytes must survive the refill error"
    );
    let received = result.unwrap();
    assert_eq!(received.bytes, b"partial");
    assert_eq!(received.failure.unwrap().kind(), io::ErrorKind::Interrupted);
}
#[test]
fn transcript_limit_is_terminal_without_unbounded_capture() {
    let (mut wire, _) = wire(&[HELLO, CAPS]);
    for _ in 0..40 {
        let mut bytes =
            serde_json::to_vec(&serde_json::json!({"event":"NOISE","data":"x".repeat(65536)}))
                .unwrap();
        bytes.push(b'\n');
        wire.rx.push_back(Received {
            bytes,
            failure: None,
        });
    }
    let mut q = Qmp::negotiate(wire, deadline()).unwrap();
    assert!(q.query_status(deadline()).is_err());
    assert!(q.transcript().rx.len() + q.transcript().tx.len() <= qmp::MAX_TRANSCRIPT);
}
