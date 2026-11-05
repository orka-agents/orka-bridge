use std::{io::{Read, Write}, time::Duration};

fn main() {
    assert_eq!(std::env::args().skip(1).collect::<Vec<_>>(), ["auth", "token", "--hostname", "github.com"]);
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).unwrap();
    assert!(input.is_empty());
    let exe = std::env::current_exe().unwrap();
    let pending = exe.with_extension("pending");
    std::fs::write(&pending, std::process::id().to_string()).unwrap();
    std::fs::rename(pending, exe.with_extension("pid")).unwrap();
    let name = exe.file_stem().unwrap().to_str().unwrap();
    match name {
        "nonzero" => {
            print!("FIXTURE_ONLY_credential");
            eprintln!("FIXTURE_ONLY_stderr");
            std::process::exit(7);
        }
        "oversize" => {
            let _ = std::io::stdout().write_all(&vec![b'x'; 16385]);
            std::thread::sleep(Duration::from_secs(60));
        }
        "hang" => std::thread::sleep(Duration::from_secs(60)),
        "hang-exit" => {
            print!("FIXTURE_ONLY_credential");
            std::io::stdout().flush().unwrap();
            close_stdout();
            std::thread::sleep(Duration::from_secs(60));
        }
        "invalid" => { std::io::stdout().write_all(b"fixture\0token").unwrap(); }
        _ => {
            std::io::stderr().write_all(&vec![b'x'; 1024 * 1024]).unwrap();
            print!(" \tFIXTURE_ONLY_credential\r\n");
        }
    }
}

#[cfg(unix)]
fn close_stdout() {
    unsafe extern "C" { fn close(fd: i32) -> i32; }
    unsafe { close(1); }
}
#[cfg(windows)]
fn close_stdout() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    unsafe { CloseHandle(GetStdHandle((-11i32) as u32)); }
}
