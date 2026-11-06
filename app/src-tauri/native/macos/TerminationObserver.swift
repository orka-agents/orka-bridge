import Foundation
import Darwin

// Exclusive to the standalone helper process, never installed in the app.
// Arm before core.start; pre-registration loss has no running VM and EOF remains
// the independent supervisor-loss path. The parent retains its bounded KILL.
final class TerminationObserver {
    private let queue: DispatchQueue
    private let source: DispatchSourceSignal
    private var retainedDuringClose: TerminationObserver?
    private var completions: [(Bool) -> Void] = []
    private var closing = false
    private var closed = false
    private var closeOK = false
    private var delivered = false
    private var readiness: (() -> Void)?

    init(queue: DispatchQueue, onTermination: @escaping () -> Void) throws {
        dispatchPrecondition(condition: .onQueue(queue))
        self.queue = queue
        var ignored = sigaction(), previous = sigaction()
        ignored.__sigaction_u.__sa_handler = SIG_IGN
        sigemptyset(&ignored.sa_mask)
        guard sigaction(SIGTERM, &ignored, &previous) == 0 else { throw SupervisorIOError.system }
        source = DispatchSource.makeSignalSource(signal: SIGTERM, queue: queue)
        source.setEventHandler { [weak self] in
            guard let self, !self.closing, !self.delivered else { return }
            self.delivered = true
            onTermination()
        }
        // Preserve the prior disposition even if the caller abandons the object.
        source.setCancelHandler { [weak self] in
            var saved = previous
            let ok = sigaction(SIGTERM, &saved, nil) == 0
            self?.finishedClose(ok)
        }
        source.resume()
    }

    // resume() alone is not the kernel-registration observation. Hold the core
    // through this callback, then release it before starting native operations.
    func whenRegistered(_ completion: @escaping () -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        guard !closing else { return }
        precondition(readiness == nil)
        readiness = completion
        source.setRegistrationHandler { [weak self] in
            guard let self else { return }
            let callback = self.readiness
            self.readiness = nil
            if !self.closing { callback?() }
        }
    }

    func close(_ completion: @escaping (Bool) -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        if closed { completion(closeOK); return }
        completions.append(completion)
        guard !closing else { return }
        closing = true
        readiness = nil
        retainedDuringClose = self
        source.cancel()
    }

    private func finishedClose(_ ok: Bool) {
        closed = true
        closeOK = ok
        let callbacks = completions
        completions.removeAll()
        retainedDuringClose = nil
        for callback in callbacks { callback(ok) }
    }

    deinit { source.cancel() }
}
