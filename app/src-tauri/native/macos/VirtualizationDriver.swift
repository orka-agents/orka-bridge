import Foundation
import Virtualization

// A later verified bootstrap supplies the complete configuration. This adapter
// never selects image paths, creates state/EFI, or exposes the VM to a renderer.
final class VirtualizationDriver: NSObject, MachineDriver, VZVirtualMachineDelegate {
    private let queue: DispatchQueue
    private var machine: VZVirtualMachine?
    private var ownedHandles: [FileHandle]
    var didEnd: (() -> Void)?

    init(configuration: VZVirtualMachineConfiguration, queue: DispatchQueue, ownedHandles: [FileHandle] = []) throws {
        dispatchPrecondition(condition: .onQueue(queue))
        try configuration.validate()
        self.queue = queue
        self.ownedHandles = ownedHandles
        machine = VZVirtualMachine(configuration: configuration, queue: queue)
        super.init()
        machine?.delegate = self
    }

    // Leave the SDK callback/delegate stack first: it can still retain the VM
    // argument and its attachments. Only a later serial turn may release authority.
    static func afterCallback(on queue: DispatchQueue, _ continuation: @escaping () -> Void) {
        queue.async { continuation() }
    }

    static func classify(_ state: VZVirtualMachine.State) -> MachineState {
        switch state {
        case .stopped: return .stopped
        case .running: return .running
        case .paused: return .paused
        case .error: return .failed
        default: return .transitioning
        }
    }
    var state: MachineState {
        dispatchPrecondition(condition: .onQueue(queue))
        return machine.map { Self.classify($0.state) } ?? .stopped
    }
    var canStart: Bool { dispatchPrecondition(condition: .onQueue(queue)); return machine?.canStart ?? false }
    var canPause: Bool { dispatchPrecondition(condition: .onQueue(queue)); return machine?.canPause ?? false }
    var canResume: Bool { dispatchPrecondition(condition: .onQueue(queue)); return machine?.canResume ?? false }
    var canStop: Bool { dispatchPrecondition(condition: .onQueue(queue)); return machine?.canStop ?? false }

    func start(_ completion: @escaping (Bool) -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        guard let vm = machine, vm.canStart else {
            Self.afterCallback(on: queue) { completion(false) }; return
        }
        vm.start { result in
            let ok = (try? result.get()) != nil
            Self.afterCallback(on: self.queue) { completion(ok) }
        }
    }
    func pause(_ completion: @escaping (Bool) -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        guard let vm = machine, vm.canPause else {
            Self.afterCallback(on: queue) { completion(false) }; return
        }
        vm.pause { result in
            let ok = (try? result.get()) != nil
            Self.afterCallback(on: self.queue) { completion(ok) }
        }
    }
    func resume(_ completion: @escaping (Bool) -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        guard let vm = machine, vm.canResume else {
            Self.afterCallback(on: queue) { completion(false) }; return
        }
        vm.resume { result in
            let ok = (try? result.get()) != nil
            Self.afterCallback(on: self.queue) { completion(ok) }
        }
    }
    func stop(_ completion: @escaping (Bool) -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        guard let vm = machine, vm.canStop else {
            Self.afterCallback(on: queue) { completion(false) }; return
        }
        // VZ stop is destructive. Success is not a guest clean-shutdown receipt.
        vm.stop { error in
            let ok = error == nil
            Self.afterCallback(on: self.queue) { completion(ok) }
        }
    }
    func dispose() {
        dispatchPrecondition(condition: .onQueue(queue))
        precondition(state == .stopped || state == .failed)
        machine?.delegate = nil
        machine = nil
        // The helper owns these endpoints independently of SDK retention. Close
        // after native disposal, before the core releases installation ownership.
        for handle in ownedHandles { try? handle.close() }
        ownedHandles.removeAll()
        didEnd = nil
    }
    func guestDidStop(_ virtualMachine: VZVirtualMachine) {
        dispatchPrecondition(condition: .onQueue(queue))
        Self.afterCallback(on: queue) { [weak self] in self?.didEnd?() }
    }
    func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) {
        dispatchPrecondition(condition: .onQueue(queue))
        // No raw framework error/path/guest output enters diagnostics.
        Self.afterCallback(on: queue) { [weak self] in self?.didEnd?() }
    }
}
