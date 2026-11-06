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
    var event = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
    guard poll(&event, 1, 5000) > 0 else { check(false, "supervisor read timeout"); return [] }
    var bytes = [UInt8](repeating: 0, count: count)
    let n = read(fd, &bytes, count)
    return n > 0 ? Array(bytes.prefix(n)) : []
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
        do {
            try rejectsBeforeDriver(q)
            try failedSetup(q, factoryThrows: false)
            try failedSetup(q, factoryThrows: true)
            try lifecycle(q, term: false)
            try lifecycle(q, term: true)
        } catch { check(false, "unexpected bootstrap fixture failure") }
        print("Swift helper bootstrap: \(assertions) assertions, \(failures) failures")
        exit(failures == 0 ? 0 : 1)
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
