unsafe extern "C" { fn fcntl(fd: i32, command: i32, ...) -> i32; }
fn main() {
    let a: Vec<_> = std::env::args().skip(1).collect();
    assert!(a == ["version"] || a == ["auth", "token", "--hostname", "github.com"]);
    let path = std::env::current_exe().unwrap();
    let fd: i32 = path.file_name().unwrap().to_str().unwrap().strip_prefix("probe-").unwrap().parse().unwrap();
    if unsafe { fcntl(fd, 1) } != -1 { print!("INHERITED_PRIVATE_FD"); }
    else { print!("FIXTURE_ONLY_clean"); }
}
