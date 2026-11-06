import Foundation
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
private func wait(_ semaphore: DispatchSemaphore) -> Bool {
    semaphore.wait(timeout: .now() + 5) == .success
}
private func readBytes(_ fd: Int32, _ count: Int) -> [UInt8] {
    var p = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
    guard poll(&p, 1, 200) > 0 else { return [] }
    var bytes = [UInt8](repeating: 0, count: count)
    let n = Darwin.read(fd, &bytes, count)
    return n > 0 ? Array(bytes.prefix(n)) : []
}
private func termSentinel(_ number: Int32) {}
private final class Received { var bytes: [UInt8] = []; var lost = 0; var closeOK = false }
private final class WeakCore { weak var value: BridgeVM? }

// The fake driver owns only the guest endpoint, never a VZ instance or image.
// Its callbacks leave the initiating stack and use the core's one serial queue.
private final class GuestFakeDriver: MachineDriver {
    var state = MachineState.stopped
    var canStart: Bool { state == .stopped }
    var canPause: Bool { state == .running }
    var canResume: Bool { state == .paused }
    var canStop: Bool { state == .running || state == .paused }
    var didEnd: (() -> Void)?
    private let queue: DispatchQueue
    private let guest: FileHandle
    private var disposed = false
    init(_ fd: Int32, _ queue: DispatchQueue) throws {
        self.queue = queue
        guest = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        var one: Int32 = 1
        guard setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size)) == 0 else {
            throw SupervisorIOError.system
        }
    }
    private func marker(_ byte: UInt8) {
        dispatchPrecondition(condition: .onQueue(queue))
        var value = byte
        precondition(Darwin.send(guest.fileDescriptor, &value, 1, MSG_DONTWAIT) == 1)
    }
    func start(_ completion: @escaping (Bool) -> Void) {
        queue.async { self.state = .running; self.marker(0xAA); completion(true) }
    }
    func pause(_ completion: @escaping (Bool) -> Void) {
        queue.async { self.state = .paused; completion(true) }
    }
    func resume(_ completion: @escaping (Bool) -> Void) {
        queue.async { self.state = .running; completion(true) }
    }
    func stop(_ completion: @escaping (Bool) -> Void) {
        queue.async { self.state = .stopped; completion(true) }
    }
    func dispose() {
        dispatchPrecondition(condition: .onQueue(queue))
        precondition(!disposed)
        disposed = true
        marker(0xBB)
        try? guest.close()
    }
}

@main
private struct IOTests {
    static func main() {
        if CommandLine.arguments.dropFirst().first == "--synthetic-peer" { child(); return }
        let q = DispatchQueue(label: "supervisor-io-tests")
        descriptors()
        socket(q)
        signalRestoration(q)
        print("Swift supervisor I/O: \(assertions) assertions, \(failures) failures")
        exit(failures == 0 ? 0 : 1)
    }

    private static func descriptors() {
        let control = pair(), guest = pair()
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        precondition(FileManager.default.createFile(atPath: url.path, contents: Data(), attributes: [.posixPermissions: 0o600]))
        let owner = try! FileHandle(forUpdating: url)
        defer {
            try? owner.close(); try? FileManager.default.removeItem(at: url)
            for fd in control + guest { _ = close(fd) }
        }
        let args = ["--supervisor-fd", String(control[0]), "--guest-fd", String(guest[0]),
                    "--ownership-fd", String(owner.fileDescriptor)]
        let borrowed = control + guest + [owner.fileDescriptor]
        let parsed = try? InheritedDescriptors.parse(args)
        check(parsed?.supervisor == control[0] && parsed?.guest == guest[0] && parsed?.ownership == owner.fileDescriptor,
              "valid descriptor triple")
        check(borrowed.allSatisfy { fcntl($0, F_GETFD) >= 0 }, "success closed borrowed FDs")
        let reordered = Array(args.suffix(2)) + Array(args.prefix(4))
        check((try? InheritedDescriptors.parse(reordered))?.supervisor == control[0], "descriptor argument order changed validity")
        let dead = dup(control[0]); precondition(dead >= 0); _ = close(dead)
        let invalid = [Array(args.dropLast()), args + ["--extra", "1"], args + Array(args.prefix(2)),
                       ["--supervisor-fd", "-1"] + Array(args.dropFirst(2)),
                       ["--supervisor-fd", "+1"] + Array(args.dropFirst(2)),
                       ["--supervisor-fd", "1x"] + Array(args.dropFirst(2)),
                       ["--supervisor-fd", "2147483648"] + Array(args.dropFirst(2)),
                       ["--supervisor-fd", String(dead)] + Array(args.dropFirst(2)),
                       Array(args.prefix(2)) + ["--guest-fd", String(control[0])] + Array(args.suffix(2)),
                       ["--guest-fd", String(control[0])] + Array(args.dropFirst(2)),
                       ["--supervisor-fd", String(owner.fileDescriptor)] + Array(args.dropFirst(2)),
                       Array(args.prefix(4)) + ["--ownership-fd", String(guest[1])]]
        for value in invalid {
            check((try? InheritedDescriptors.parse(value)) == nil, "invalid descriptor accepted: \(value)")
            check(borrowed.allSatisfy { fcntl($0, F_GETFD) >= 0 }, "error closed borrowed FDs")
        }
        var datagrams: [Int32] = [-1, -1]
        precondition(socketpair(AF_UNIX, SOCK_DGRAM, 0, &datagrams) == 0)
        defer { for fd in datagrams { _ = close(fd) } }
        check((try? InheritedDescriptors.parse(["--supervisor-fd", String(datagrams[0])] + Array(args.dropFirst(2)))) == nil,
              "non-stream socket accepted")
        check((borrowed + datagrams).allSatisfy { fcntl($0, F_GETFD) >= 0 }, "type error closed borrowed FDs")
    }

    private static func socket(_ q: DispatchQueue) {
        let sockets = pair()
        let originalFlags = fcntl(sockets[0], F_GETFL)
        var connection: SupervisorSocket? = q.sync { try! SupervisorSocket(owningFD: sockets[0], queue: q) }
        defer { _ = close(sockets[1]) }
        let flags = fcntl(sockets[0], F_GETFL)
        check(flags & O_NONBLOCK != 0 && flags & ~O_NONBLOCK == originalFlags & ~O_NONBLOCK,
              "nonblocking setup lost existing flags")
        var option: Int32 = 0; var length = socklen_t(MemoryLayout<Int32>.size)
        check(getsockopt(sockets[0], SOL_SOCKET, SO_NOSIGPIPE, &option, &length) == 0 && option == 1,
              "socket SIGPIPE protection missing")
        let frame: [UInt8] = [75, 1, 0, 0, 0, 0, 0, 0, 0]
        check(q.sync { connection!.send(frame) }, "full frame write refused")
        check(readBytes(sockets[1], frame.count) == frame, "full frame not sent")
        if flags & O_NONBLOCK != 0 {
            var bytes = [UInt8](repeating: 7, count: 4096)
            while Darwin.send(sockets[0], &bytes, bytes.count, MSG_DONTWAIT) > 0 {}
            check(errno == EAGAIN || errno == EWOULDBLOCK, "buffer was not filled")
            check(!q.sync { connection!.send([9, 9, 9]) }, "full buffer reported full write")
            // Drain and wait another queue turn: false must not schedule replay.
            while !readBytes(sockets[1], 4096).isEmpty {}
            q.sync {}
            check(readBytes(sockets[1], 3).isEmpty, "failed write was replayed")
        }
        let data = Received(), received = DispatchSemaphore(value: 0), eof = DispatchSemaphore(value: 0)
        q.sync {
            connection!.start(receive: { data.bytes += $0; received.signal() },
                              peerLost: { data.lost += 1; eof.signal() })
        }
        let fragments: [[UInt8]] = [[4], [5, 6]]
        for fragment in fragments {
            var bytes = fragment
            check(Darwin.send(sockets[1], &bytes, bytes.count, MSG_DONTWAIT) == bytes.count, "fragment send")
            check(wait(received), "read source did not deliver fragment")
        }
        check(q.sync { data.bytes == [4, 5, 6] }, "read source corrupted fragments")
        _ = shutdown(sockets[1], SHUT_WR)
        check(wait(eof), "EOF not delivered")
        let closed = DispatchSemaphore(value: 0), closedAgain = DispatchSemaphore(value: 0)
        q.sync {
            connection!.close { ok in data.closeOK = ok; closed.signal() }
            check(!connection!.send([42]), "send during cancellation succeeded")
            connection!.close { _ in closedAgain.signal() }
        }
        check(wait(closed) && wait(closedAgain), "cancel-handler completions did not drain")
        check(q.sync { data.closeOK && data.lost == 1 }, "close failed or duplicate peer loss")
        let didClose = fcntl(sockets[0], F_GETFD) == -1
        check(didClose, "socket not closed after cancellation")
        if didClose {
            let replacement = pair()
            precondition(dup2(replacement[0], sockets[0]) == sockets[0])
            check(!q.sync { connection!.send([42]) }, "send after cancellation succeeded")
            check(readBytes(replacement[1], 1).isEmpty, "send reached reused FD")
            q.sync { connection = nil }
            q.sync {}
            check(fcntl(sockets[0], F_GETFD) >= 0, "deallocation closed reused FD")
            for fd in Set(replacement + [sockets[0]]) { _ = close(fd) }
        }

        let broken = pair()
        let output = q.sync { try! SupervisorSocket(owningFD: broken[0], queue: q) }
        _ = close(broken[1])
        let previous = signal(SIGPIPE, SIG_DFL)
        check(!q.sync { output.send([1]) }, "closed peer write succeeded under default SIGPIPE")
        _ = signal(SIGPIPE, previous)
        let outputClosed = DispatchSemaphore(value: 0)
        q.sync { output.close { _ in outputClosed.signal() } }
        check(wait(outputClosed), "never-started socket close did not finish")
        check(fcntl(broken[0], F_GETFD) == -1, "never-started socket remained open")

        // Normal deinit must also cancel an unstarted source without a retain cycle.
        let abandoned = pair()
        q.sync { _ = try! SupervisorSocket(owningFD: abandoned[0], queue: q) }
        let end = Date().addingTimeInterval(5)
        while fcntl(abandoned[0], F_GETFD) >= 0 && Date() < end { Thread.sleep(forTimeInterval: 0.002) }
        check(fcntl(abandoned[0], F_GETFD) == -1, "normal socket deinit leaked FD")
        _ = close(abandoned[1])
    }

    private static func signalRestoration(_ q: DispatchQueue) {
        // A harmless prior handler makes the inert seam fail assertions, not die.
        var original = sigaction(), before = sigaction(), after = sigaction()
        precondition(Darwin.sigaction(SIGTERM, nil, &original) == 0)
        _ = signal(SIGTERM, termSentinel)
        defer { _ = Darwin.sigaction(SIGTERM, &original, nil) }
        precondition(Darwin.sigaction(SIGTERM, nil, &before) == 0)
        let data = Received(), delivered = DispatchSemaphore(value: 0)
        let observer = q.sync {
            try! TerminationObserver(queue: q, onTermination: { data.lost += 1; delivered.signal() })
        }
        precondition(kill(getpid(), SIGTERM) == 0)
        check(wait(delivered), "TERM source did not deliver")
        precondition(kill(getpid(), SIGTERM) == 0)
        q.sync {}
        let restored = DispatchSemaphore(value: 0)
        q.sync { observer.close { ok in data.closeOK = ok; restored.signal() } }
        check(wait(restored), "signal cancellation did not complete")
        check(q.sync { data.closeOK && data.lost == 1 }, "signal close failed or repeated delivery")
        precondition(Darwin.sigaction(SIGTERM, nil, &after) == 0)
        check(unsafeBitCast(before.__sigaction_u.__sa_handler, to: UInt.self) ==
              unsafeBitCast(after.__sigaction_u.__sa_handler, to: UInt.self) &&
              before.sa_flags == after.sa_flags && before.sa_mask == after.sa_mask,
              "prior TERM disposition not restored")
        q.sync { _ = try! TerminationObserver(queue: q, onTermination: {}) }
        q.sync {}
        let end = Date().addingTimeInterval(5)
        repeat {
            precondition(Darwin.sigaction(SIGTERM, nil, &after) == 0)
            if unsafeBitCast(before.__sigaction_u.__sa_handler, to: UInt.self) ==
                unsafeBitCast(after.__sigaction_u.__sa_handler, to: UInt.self) { break }
            Thread.sleep(forTimeInterval: 0.002)
        } while Date() < end
        check(unsafeBitCast(before.__sigaction_u.__sa_handler, to: UInt.self) ==
              unsafeBitCast(after.__sigaction_u.__sa_handler, to: UInt.self), "normal observer deinit did not restore TERM")
    }

    private static func child() {
        let q = DispatchQueue(label: "synthetic-helper")
        q.async {
            var driver: GuestFakeDriver?
            var socket: SupervisorSocket?
            var observer: TerminationObserver?
            var ownership: Int32?
            do {
                let fd = try InheritedDescriptors.parse(Array(CommandLine.arguments.dropFirst(2)))
                ownership = fd.ownership
                let machine = try GuestFakeDriver(fd.guest, q)
                driver = machine
                let io = try SupervisorSocket(owningFD: fd.supervisor, queue: q)
                socket = io
                let reference = WeakCore()
                let term = try TerminationObserver(queue: q, onTermination: { reference.value?.peerClosed() })
                observer = term
                // No throwable setup remains after transferring ownership to core.
                let core = BridgeVM(driver: machine, ownershipFD: fd.ownership, queue: q,
                                    output: { io.send($0) }, terminal: { success in
                    io.close { socketOK in
                        term.close { signalOK in exit(success && socketOK && signalOK ? 0 : 1) }
                    }
                })
                ownership = nil
                reference.value = core
                io.start(receive: { reference.value?.receive($0) }, peerLost: { reference.value?.peerClosed() })
                core.start()
            } catch {
                driver?.dispose() // Configured, unstarted driver closes guest before owner.
                if let fd = ownership { _ = close(fd) }
                let finish = { exit(Int32(1)) }
                if let io = socket {
                    io.close { _ in
                        if let term = observer { term.close { _ in finish() } } else { finish() }
                    }
                } else { finish() }
            }
        }
        dispatchMain()
    }
}
