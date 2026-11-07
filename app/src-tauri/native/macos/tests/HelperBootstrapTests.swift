import Foundation
import Virtualization
import Darwin

private var failures = 0
private var assertions = 0
private func check(_ value: @autoclosure () -> Bool, _ label: String, line: Int = #line) {
    assertions += 1
    if !value() { failures += 1; print("FAIL \(line): \(label)") }
}
private func pair() -> [Int32] {
    var fds: [Int32] = [-1, -1]
    precondition(socketpair(AF_UNIX, SOCK_STREAM, 0, &fds) == 0)
    return fds
}
private func wait(_ semaphore: DispatchSemaphore) {
    check(semaphore.wait(timeout: .now() + 5) == .success, "bootstrap completion timeout")
}
private func receive(_ fd: Int32, _ count: Int) -> [UInt8] {
    let deadline = DispatchTime.now().uptimeNanoseconds + 5_000_000_000
    var result: [UInt8] = []
    while result.count < count {
        let now = DispatchTime.now().uptimeNanoseconds
        guard now < deadline else { check(false, "supervisor read timeout"); return result }
        var event = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
        let remaining = Int32((deadline - now + 999_999) / 1_000_000)
        let ready = poll(&event, 1, remaining)
        if ready < 0 && errno == EINTR { continue }
        guard ready > 0 else { check(false, "supervisor read timeout/error"); return result }
        var bytes = [UInt8](repeating: 0, count: count - result.count)
        let n = read(fd, &bytes, bytes.count)
        if n < 0 && errno == EINTR { continue }
        guard n >= 0 else { check(false, "supervisor read error"); return result }
        if n == 0 { return result }
        result.append(contentsOf: bytes.prefix(n))
    }
    return result
}
private func sentinel(_ signal: Int32) {}
private final class Trace {
    var disposal = 0
    var statuses: [Int32] = []
    weak var driver: FakeDriver?
}
private final class Fixture {
    let root: URL
    let control = pair(), guest = pair()
    let owner: Int32, config: Int32
    init() throws {
        root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        owner = open(root.appendingPathComponent("owner").path, O_CREAT | O_RDWR, 0o600)
        let object: [String: Any] = ["version": 1, "installationPath": root.path,
            "osDiskPath": root.appendingPathComponent("os.disk").path,
            "stateDiskPath": root.appendingPathComponent("state.disk").path,
            "machineId": Array(VZGenericMachineIdentifier().dataRepresentation),
            "mac": [2, 1, 2, 3, 4, 5], "createEFI": true]
        let path = root.appendingPathComponent("config")
        try JSONSerialization.data(withJSONObject: object).write(to: path)
        config = open(path.path, O_RDONLY)
        precondition(owner >= 3 && config >= 3)
        precondition(unlink(path.path) == 0)
    }
    var args: [String] {
        ["--config-fd", String(config), "--supervisor-fd", String(control[0]),
         "--guest-fd", String(guest[0]), "--ownership-fd", String(owner)]
    }
    // Bootstrap owns the other endpoints after valid parsing.
    deinit {
        _ = close(control[1]); _ = close(guest[1])
        try? FileManager.default.removeItem(at: root)
    }
}
private final class FakeDriver: MachineDriver {
    var state = MachineState.stopped
    var canStart: Bool { state == .stopped }
    var canPause: Bool { state == .running }
    var canResume: Bool { state == .paused }
    var canStop: Bool { state == .running || state == .paused }
    var didEnd: (() -> Void)?
    private let guest: FileHandle
    private let queue: DispatchQueue
    private let trace: Trace
    private let owner: Int32
    private let termAtStart: Bool
    init(guest: Int32, owner: Int32, queue: DispatchQueue, trace: Trace, termAtStart: Bool = false) {
        self.guest = FileHandle(fileDescriptor: guest, closeOnDealloc: true)
        self.owner = owner; self.queue = queue; self.trace = trace; self.termAtStart = termAtStart
    }
    func start(_ completion: @escaping (Bool) -> Void) {
        if termAtStart {
            precondition(kill(getpid(), SIGTERM) == 0)
            queue.asyncAfter(deadline: .now() + 0.05) {
                check(self.trace.disposal == 0 && fcntl(self.owner, F_GETFD) >= 0, "pending start released ownership")
                self.state = .running; completion(true)
            }
        } else { queue.async { self.state = .running; completion(true) } }
    }
    func pause(_ completion: @escaping (Bool) -> Void) { queue.async { self.state = .paused; completion(true) } }
    func resume(_ completion: @escaping (Bool) -> Void) { queue.async { self.state = .running; completion(true) } }
    func stop(_ completion: @escaping (Bool) -> Void) { queue.async { self.state = .stopped; completion(true) } }
    func dispose() {
        check(fcntl(owner, F_GETFD) >= 0, "driver disposed after owner close")
        trace.disposal += 1
        try? guest.close()
        didEnd = nil
    }
}

@main
private struct BootstrapTests {
    static func main() {
        let q = DispatchQueue(label: "bootstrap-tests")
        for clean in [false, true] {
            for socket in [false, true] {
                for signal in [false, true] {
                    check(HelperBootstrap.terminalExitCode(cleanStop: clean, socketClosed: socket,
                          signalRestored: signal) == (clean && socket && signal ? 0 : 1), "terminal classification")
                }
            }
        }
        let identity = HelperBootstrap.machineIdentifier()
        check((1...4096).contains(identity.count), "bounded raw SDK identity")
        check(VZGenericMachineIdentifier(dataRepresentation: identity) != nil, "SDK identity reopens")
        initializerProcess()
        hungInitializerCleanup()
        fragmentedFrame(q)
        do {
            try rejectsBeforeDriver(q)
            try realFactoryFailure(q)
            try failedSetup(q, factoryThrows: false)
            try failedSetup(q, factoryThrows: true)
            try lifecycle(q, term: false)
            try lifecycle(q, term: true)
        } catch { check(false, "unexpected bootstrap fixture failure") }
        print("Swift helper bootstrap: \(assertions) assertions, \(failures) failures")
        exit(failures == 0 ? 0 : 1)
    }

    private static func initializerProcess() {
        let native = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        let helper = native.deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("gen/apple-helper/BridgeVM")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        do {
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
            defer { try? FileManager.default.removeItem(at: root) }
            for args in [["--machine-identifier"], ["--machine-identifier", "extra"]] {
                let (pid, output) = try spawnInitializer([helper.path] + args, in: root)
                guard let status = try finishInitializer(pid, until: .now() + .seconds(5)) else {
                    check(false, "initializer cleanup exit unobserved"); continue
                }
                let bytes = output.readDataToEndOfFile()
                if args.count == 1 {
                    check(status == 0, "initializer exit success")
                    check((1...4096).contains(bytes.count), "raw stdout identity bound")
                    check(VZGenericMachineIdentifier(dataRepresentation: bytes) != nil, "raw stdout SDK validity")
                } else {
                    check(status != 0 && bytes.isEmpty, "extra initializer arguments refuse")
                }
                let files = try FileManager.default.contentsOfDirectory(atPath: root.path)
                check(files.isEmpty, "initializer wrote no installation files")
            }
        } catch { check(false, "initializer process fixture failed") }
    }

    // POSIX owns the unreaped direct PID; Foundation Process may reap asynchronously.
    private static func spawnInitializer(_ command: [String], in root: URL) throws -> (pid_t, FileHandle) {
        var output: [Int32] = [-1, -1]
        guard pipe(&output) == 0 else { throw SupervisorIOError.system }
        var actions: posix_spawn_file_actions_t? = nil
        guard posix_spawn_file_actions_init(&actions) == 0 else {
            _ = close(output[0]); _ = close(output[1]); throw SupervisorIOError.system
        }
        defer { posix_spawn_file_actions_destroy(&actions); _ = close(output[1]) }
        guard posix_spawn_file_actions_adddup2(&actions, output[1], STDOUT_FILENO) == 0,
              posix_spawn_file_actions_addclose(&actions, output[0]) == 0,
              posix_spawn_file_actions_addclose(&actions, output[1]) == 0,
              posix_spawn_file_actions_addopen(&actions, STDIN_FILENO, "/dev/null", O_RDONLY, 0) == 0,
              posix_spawn_file_actions_addopen(&actions, STDERR_FILENO, "/dev/null", O_WRONLY, 0) == 0 else {
            _ = close(output[0]); throw SupervisorIOError.system
        }
        var args = (["sh", "-c", "cd \"$1\" && shift && exec \"$@\"", "initializer", root.path] + command)
            .map { strdup($0) } + [nil]
        defer { for arg in args { free(arg) } }
        var environment: [UnsafeMutablePointer<CChar>?] = [nil]
        var pid: pid_t = 0
        guard posix_spawn(&pid, "/bin/sh", &actions, nil, &args, &environment) == 0 else {
            _ = close(output[0]); throw SupervisorIOError.system
        }
        return (pid, FileHandle(fileDescriptor: output[0], closeOnDealloc: true))
    }
    private static func waitInitializer(_ pid: pid_t, until end: DispatchTime) throws -> Int32? {
        var status: Int32 = 0
        while true {
            let result = waitpid(pid, &status, WNOHANG)
            if result == pid { return status }
            if result < 0 {
                if errno == EINTR { continue }
                throw SupervisorIOError.system
            }
            if DispatchTime.now() >= end { return nil }
            usleep(2000)
        }
    }
    private static func finishInitializer(_ pid: pid_t, until end: DispatchTime) throws -> Int32? {
        if let status = try waitInitializer(pid, until: end) { return status }
        guard kill(pid, SIGTERM) == 0 else { throw SupervisorIOError.system }
        var status: Int32 = 0
        while true {
            if waitpid(pid, &status, 0) == pid { return status }
            if errno != EINTR { throw SupervisorIOError.system }
        }
    }
    private static func hungInitializerCleanup() {
        do {
            let (pid, output) = try spawnInitializer(
                ["/bin/sh", "-c", "trap '' TERM; printf ready; while :; do :; done"],
                in: FileManager.default.temporaryDirectory)
            check(receive(output.fileDescriptor, 5) == Array("ready".utf8), "hung child installed TERM-ignore")
            let done = DispatchSemaphore(value: 0)
            DispatchQueue.global().async {
                do { check(try finishInitializer(pid, until: .now()) != nil, "hung child exit observed") }
                catch { check(false, "hung child cleanup error") }
                done.signal()
            }
            let bounded = done.wait(timeout: .now() + .milliseconds(300)) == .success
            if !bounded {
                // The cleanup waiter still owns this unreaped PID, so it cannot be reused.
                _ = kill(pid, SIGKILL)
                wait(done)
            }
            check(bounded, "TERM-ignoring child cleanup returns within its bound")
        } catch { check(false, "hung initializer fixture failed") }
    }

    private static func fragmentedFrame(_ q: DispatchQueue) {
        let sockets = pair(), written = DispatchSemaphore(value: 0)
        defer { _ = close(sockets[0]); _ = close(sockets[1]) }
        var prefix: [UInt8] = [66, 82]
        precondition(write(sockets[1], &prefix, prefix.count) == prefix.count)
        q.async {
            var byte: UInt8 = 0
            let deadline = DispatchTime.now() + .seconds(5)
            // Release the rest only after the reader consumes the first fragment.
            while recv(sockets[0], &byte, 1, MSG_PEEK | MSG_DONTWAIT) > 0 {
                precondition(DispatchTime.now() < deadline, "first fragment was not consumed")
                usleep(1000)
            }
            var suffix: [UInt8] = [86, 77, 1]
            precondition(write(sockets[1], &suffix, suffix.count) == suffix.count)
            written.signal()
        }
        check(receive(sockets[0], 5) == [66, 82, 86, 77, 1], "fragmented supervisor frame")
        wait(written)
    }

    private static func rejectsBeforeDriver(_ q: DispatchQueue) throws {
        let f = try Fixture(), trace = Trace()
        q.sync {
            HelperBootstrap.run(arguments: Array(f.args.dropLast()), queue: q, makeDriver: { _, _, _ in
                check(false, "malformed input reached driver factory"); throw SupervisorIOError.system
            }, finish: { trace.statuses.append($0) })
            check(trace.statuses == [1], "rejection did not fail once")
            for fd in [f.config, f.owner, f.control[0], f.guest[0]] {
                check(fcntl(fd, F_GETFD) >= 0, "rejection closed borrowed capability")
                _ = close(fd)
            }
        }
        check(receive(f.control[1], 5).isEmpty, "rejection emitted readiness")
    }

    private static func realFactoryFailure(_ q: DispatchQueue) throws {
        let f = try Fixture()
        let configuration = try HelperConfiguration.read(owningFD: f.config)
        q.sync {
            do {
                _ = try HelperBootstrap.makeDriver(configuration: configuration, guestFD: f.guest[0], queue: q)
                check(false, "missing disks reached native VM construction")
            } catch HelperConfigurationError.system {
                check(true, "real factory rejected missing disks")
            } catch { check(false, "unexpected real factory failure") }
            check(fcntl(f.guest[0], F_GETFD) == -1, "real factory failure leaked adopted guest")
            check(fcntl(f.owner, F_GETFD) >= 0, "real factory failure closed ownership")
            check(fcntl(f.config, F_GETFD) == -1, "real configuration reader leaked config FD")
            check(!FileManager.default.fileExists(atPath: f.root.appendingPathComponent("efi.vars").path),
                  "missing disks created an EFI store")
            _ = close(f.control[0])
            _ = close(f.owner)
        }
    }

    private static func failedSetup(_ q: DispatchQueue, factoryThrows: Bool) throws {
        let f = try Fixture(), trace = Trace(), done = DispatchSemaphore(value: 0)
        q.async {
            HelperBootstrap.run(arguments: f.args, queue: q, makeDriver: { _, guest, queue in
                if factoryThrows {
                    let adopted = FileHandle(fileDescriptor: guest, closeOnDealloc: true)
                    try adopted.close()
                    throw SupervisorIOError.system
                }
                let driver = FakeDriver(guest: guest, owner: f.owner, queue: queue, trace: trace)
                trace.driver = driver
                _ = close(f.control[0]) // Before socket/source construction, never a live dispatch FD.
                return driver
            }, finish: { status in trace.statuses.append(status); done.signal() })
        }
        wait(done)
        q.sync {
            check(trace.statuses == [1], "setup failure status/count")
            check(trace.disposal == (factoryThrows ? 0 : 1), "setup disposal count")
            check(trace.driver == nil, "failed setup retained driver")
            for fd in [f.config, f.owner, f.guest[0], f.control[0]] {
                check(fcntl(fd, F_GETFD) == -1, "failed setup leaked capability")
            }
        }
        check(receive(f.control[1], 5).isEmpty, "failed setup emitted readiness")
    }

    private static func lifecycle(_ q: DispatchQueue, term: Bool) throws {
        let f = try Fixture(), trace = Trace(), done = DispatchSemaphore(value: 0)
        var original = sigaction(), before = sigaction()
        precondition(sigaction(SIGTERM, nil, &original) == 0)
        _ = signal(SIGTERM, sentinel)
        precondition(sigaction(SIGTERM, nil, &before) == 0)
        defer { _ = sigaction(SIGTERM, &original, nil) }
        let expectedHandler = unsafeBitCast(before.__sigaction_u.__sa_handler, to: UInt.self)
        q.async {
            HelperBootstrap.run(arguments: f.args, queue: q, makeDriver: { _, guest, queue in
                let driver = FakeDriver(guest: guest, owner: f.owner, queue: queue,
                                        trace: trace, termAtStart: term)
                trace.driver = driver
                return driver
            }, finish: { status in
                var restored = sigaction()
                precondition(sigaction(SIGTERM, nil, &restored) == 0)
                check(unsafeBitCast(restored.__sigaction_u.__sa_handler, to: UInt.self) == expectedHandler,
                      "finish before TERM restoration")
                check(fcntl(f.control[0], F_GETFD) == -1, "finish before socket cancellation close")
                check(fcntl(f.owner, F_GETFD) == -1 && fcntl(f.guest[0], F_GETFD) == -1, "finish leaked owner/guest")
                trace.statuses.append(status); done.signal()
            })
        }
        if !term {
            check(receive(f.control[1], 5) == [66, 82, 86, 77, 1], "missing native ready")
            var stop: [UInt8] = [83, 1, 0, 0, 0, 0, 0, 0, 0]
            check(write(f.control[1], &stop, stop.count) == stop.count, "stop send")
            check(receive(f.control[1], 9) == [75, 1, 0, 0, 0, 0, 0, 0, 0], "stop ACK")
        }
        wait(done)
        check(receive(f.control[1], 5).isEmpty, "terminal socket not EOF")
        q.sync {
            check(trace.statuses == [term ? 1 : 0], "terminal success/count")
            check(trace.disposal == 1, "terminal disposal count")
            check(trace.driver == nil, "terminal driver retained")
        }
    }
}
