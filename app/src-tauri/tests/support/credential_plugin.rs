// Synthetic child executable compiled by credential tests, never a real login plugin.
use std::{env, io::{self, Read, Write}, time::Duration};
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetConsoleProcessList(processes: *mut u32, count: u32) -> u32;
}
fn main() {
    if let Ok(path) = env::var("RECORD_FILE") {
        std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap().write_all(b"x").unwrap();
    }
    if let Ok(path) = env::var("PID_FILE") { std::fs::write(path, std::process::id().to_string()).unwrap(); }
    match env::args().nth(1).as_deref() {
        Some("hang") => loop { std::thread::sleep(Duration::from_secs(60)); },
        Some("overflow") => { io::stdout().write_all(&vec![b'x'; 1024 * 1024 + 1]).unwrap(); },
        Some("stderr") => { io::stderr().write_all(&vec![b'x'; 65537]).unwrap(); },
        Some("fail") => { eprintln!("fixture-private-diagnostic"); std::process::exit(1); },
        _ => {
            #[cfg(windows)]
            if env::args().nth(1).as_deref() == Some("no-console") {
                let mut pid = 0;
                assert_eq!(unsafe { GetConsoleProcessList(&mut pid, 1) }, 0, "plugin has a console");
            }
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).unwrap();
            assert!(input.is_empty());
            if let Ok(expected) = env::var("EXPECTED_INFO") {
                assert_eq!(env::var("KUBERNETES_EXEC_INFO").unwrap(), expected);
            }
            if let Ok(name) = env::var("EXPECT_ABSENT") { assert!(env::var_os(name).is_none()); }
            print!("{}", env::var("FIXTURE_RESPONSE").unwrap());
        }
    }
}
