import Foundation
import Darwin

// Owns only the supervisor socket. The core, driver and adapters share this queue.
final class SupervisorSocket {
    private enum Phase { case idle, running, closing, closed }
    private let owner: FileHandle
    private let queue: DispatchQueue
    private var phase = Phase.idle
    private var source: DispatchSourceRead?
    private var retainedDuringClose: SupervisorSocket?
    private var completions: [(Bool) -> Void] = []
    private var closeOK = false
    private var reportedLoss = false

    // Ownership transfers even on setup failure; no other capability is closed.
    init(owningFD: Int32, queue: DispatchQueue) throws {
        dispatchPrecondition(condition: .onQueue(queue))
        self.queue = queue
        owner = FileHandle(fileDescriptor: owningFD, closeOnDealloc: true)
        var one: Int32 = 1
        let flags = fcntl(owningFD, F_GETFL)
        guard flags >= 0,
              setsockopt(owningFD, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size)) == 0,
              fcntl(owningFD, F_SETFL, flags | O_NONBLOCK) == 0 else { throw SupervisorIOError.system }
    }

    func start(receive: @escaping ([UInt8]) -> Void, peerLost: @escaping () -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        precondition(phase == .idle)
        let readSource = DispatchSource.makeReadSource(fileDescriptor: owner.fileDescriptor, queue: queue)
        let descriptor = owner
        readSource.setEventHandler { [weak self] in self?.readOnce(receive, peerLost) }
        // Cancellation is the sole close owner after handlers using the FD drain.
        readSource.setCancelHandler { [weak self] in
            let ok: Bool
            do { try descriptor.close(); ok = true } catch { ok = false }
            self?.finishedClose(ok)
        }
        source = readSource
        phase = .running
        readSource.resume()
    }

    func send(_ bytes: [UInt8]) -> Bool {
        dispatchPrecondition(condition: .onQueue(queue))
        guard phase == .idle || phase == .running else { return false }
        return bytes.withUnsafeBytes {
            Darwin.send(owner.fileDescriptor, $0.baseAddress, bytes.count, 0) == bytes.count
        }
    }

    private func readOnce(_ receive: ([UInt8]) -> Void, _ peerLost: () -> Void) {
        guard phase == .running else { return }
        // One bounded read per turn; flooded input cannot park native callbacks.
        var bytes = [UInt8](repeating: 0, count: 9)
        let count = Darwin.read(owner.fileDescriptor, &bytes, bytes.count)
        if count > 0 { receive(Array(bytes.prefix(count))); return }
        if count < 0 && [EAGAIN, EWOULDBLOCK, EINTR].contains(errno) { return }
        guard !reportedLoss else { return }
        reportedLoss = true
        close { _ in }
        peerLost()
    }

    func close(_ completion: @escaping (Bool) -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        if phase == .closed { completion(closeOK); return }
        completions.append(completion)
        guard phase != .closing else { return }
        phase = .closing
        retainedDuringClose = self
        if let source { source.cancel() }
        else {
            do { try owner.close(); finishedClose(true) } catch { finishedClose(false) }
        }
    }

    private func finishedClose(_ ok: Bool) {
        phase = .closed
        closeOK = ok
        source = nil
        let callbacks = completions
        completions.removeAll()
        retainedDuringClose = nil
        for callback in callbacks { callback(ok) }
    }

    deinit {
        if let source { source.cancel() }
        else if phase != .closed { try? owner.close() }
    }
}
