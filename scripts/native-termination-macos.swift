// Run from the repository root: swift scripts/native-termination-macos.swift
// Builds a real Tauri subprocess; never logs out the host or starts a VM.
import Foundation
import Darwin

func require(_ condition: Bool, _ message: String) {
    if !condition { fatalError(message) }
}
require(Thread.isMainThread, "driver must start on the main thread")
let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
let manifest = root.appendingPathComponent("app/src-tauri")
let scratch = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
try FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: scratch) }

func run(_ args: [String], _ name: String, timeout: Double = 900,
         environment: [String: String]? = nil) throws -> (Int32, String) {
    let output = scratch.appendingPathComponent(name)
    _ = FileManager.default.createFile(atPath: output.path, contents: nil)
    let handle = try FileHandle(forWritingTo: output)
    defer { try? handle.close() }
    let process = Process()
    let inherited = environment ?? ProcessInfo.processInfo.environment
    let cargoHome = inherited["CARGO_HOME"] ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".cargo").path
    let executable = args[0].hasPrefix("/") ? args[0] : "\(cargoHome)/bin/\(args[0])"
    process.executableURL = URL(fileURLWithPath: executable)
    process.arguments = Array(args.dropFirst())
    process.environment = inherited
    process.standardOutput = handle
    process.standardError = handle
    try process.run()
    let end = Date().addingTimeInterval(timeout)
    while process.isRunning && Date() < end { Thread.sleep(forTimeInterval: 0.02) }
    if process.isRunning {
        process.terminate()
        Thread.sleep(forTimeInterval: 0.2)
        if process.isRunning { kill(process.processIdentifier, SIGKILL) }
        process.waitUntilExit()
        let log = try String(contentsOf: output, encoding: .utf8)
        fatalError("\(name) timed out: \(log)")
    }
    process.waitUntilExit()
    return (process.terminationStatus, try String(contentsOf: output, encoding: .utf8))
}

let (built, buildLog) = try run([
    "cargo", "+1.98.1", "build", "--manifest-path", manifest.appendingPathComponent("Cargo.toml").path,
    "--locked", "--lib", "--bin", "orka-bridge", "--features", "custom-protocol", "--message-format=json"
], "build.log")
require(built == 0, buildLog)
let records = buildLog.split(separator: "\n").compactMap {
    try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any]
}
let crates = ["bridge_engine", "tauri", "objc2", "objc2_app_kit", "core_foundation", "tokio"]
var externs: [String] = []
var deps: String?
for name in crates {
    let files = records.filter { ($0["target"] as? [String: Any])?["name"] as? String == name }
        .flatMap { $0["filenames"] as? [String] ?? [] }.filter { $0.hasSuffix(".rlib") }
    require(files.count == 1, "expected one \(name) rlib, got \(files)")
    externs += ["--extern", "\(name)=\(files[0])"]
    if name == "tauri" { deps = URL(fileURLWithPath: files[0]).deletingLastPathComponent().path }
}
let buildScript = records.first {
    $0["reason"] as? String == "build-script-executed" &&
    ($0["package_id"] as? String ?? "").contains("#orka-bridge@")
}
let outDir = buildScript?["out_dir"] as? String
require(outDir != nil, "missing Tauri build-script output directory")
let source = scratch.appendingPathComponent("fixture.rs")
// Import the adapter itself, not a Swift reimplementation of its policy.
try """
#[path = "\(manifest.path)/src/lifecycle/macos.rs"] mod macos;
fn main() { macos::native_tests::run(tauri::generate_context!("\(manifest.path)/tauri.conf.json")); }
""".write(to: source, atomically: true, encoding: .utf8)
let binary = scratch.appendingPathComponent("native-termination")
var environment = ProcessInfo.processInfo.environment
environment["CARGO_MANIFEST_DIR"] = manifest.path
environment["OUT_DIR"] = outDir!
environment["CARGO_PKG_NAME"] = "native-termination-fixture"
environment["CARGO_PKG_VERSION"] = "0.0.0"
environment["CARGO_PKG_AUTHORS"] = ""
environment["CARGO_PKG_DESCRIPTION"] = "Native termination fixture"
for pair in buildScript?["env"] as? [[String]] ?? [] {
    environment[pair[0]] = pair[1]
}
let (compiled, compilerLog) = try run([
    "rustc", "+1.98.1", "--edition=2021", "--cfg", "test", "--crate-name", "native_termination_fixture",
    source.path, "-o", binary.path, "-L", "dependency=\(deps!)"
] + externs, "compile.log", environment: environment)
require(compiled == 0, compilerLog)

for scenario in ["clean", "failed", "existing", "deadline", "completed", "missing-delegate"] {
    let start = Date()
    let (status, log) = try run([binary.path, scenario], "\(scenario).log", timeout: 40)
    print("\(scenario): status=\(status)\n\(log)")
    require(status == 0, "\(scenario) crashed or failed") // NOT evidence of clean VM cleanup.
    if scenario == "missing-delegate" {
        require(log.contains("native-test:install-rejected"), "missing delegate accepted")
        continue
    }
    func count(_ marker: String) -> Int { log.components(separatedBy: marker).count - 1 }
    require(count("native-test:worker") == 1, "cleanup did not have exactly one worker")
    require(count("native-test:Exit/LoopDestroyed") == 1, "not exactly one Tao final exit")
    require(count("native-test:reply=") == 1, "augmented delegate not called exactly once")
    require(log.contains("native-test:reply=\(scenario == "completed" ? 1 : 2)"), "wrong termination reply")
    if scenario != "completed" {
        require(log.contains("native-test:modal-poll"), "termination reply did not run in AppKit modal mode")
    }
    if scenario == "deadline" {
        require(Date().timeIntervalSince(start) >= 29, "outer deadline fired prematurely")
        require(log.contains("unconfirmed"), "deadline was relabelled clean")
        require(!log.contains("native-test:finished="), "deadline fixture unexpectedly completed")
    } else {
        require(log.contains("native-test:finished=\(scenario != "failed")"), "wrong cleanup result")
        if scenario == "failed" { require(log.contains("unclean"), "failed cleanup lacked diagnostic") }
    }
}
print("Native termination passed: real terminate:, modal reply, one final exit; no host logout or VM.")
