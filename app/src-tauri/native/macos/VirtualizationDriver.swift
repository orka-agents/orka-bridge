import Foundation
import Virtualization

// A later verified bootstrap supplies the complete configuration. This adapter
// never selects image paths, creates state/EFI, or exposes the VM to a renderer.
final class VirtualizationDriver: NSObject, MachineDriver, VZVirtualMachineDelegate {
    private let queue: DispatchQueue
    private var machine: VZVirtualMachine?
    var didEnd: (() -> Void)?

    init(configuration: VZVirtualMachineConfiguration, queue: DispatchQueue) throws {
        dispatchPrecondition(condition: .onQueue(queue))
        try configuration.validate()
        self.queue = queue
        machine = VZVirtualMachine(configuration: configuration, queue: queue)
        super.init()
        machine?.delegate = self
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
        guard let vm = machine, vm.canStart else { completion(false); return }
        vm.start { result in completion((try? result.get()) != nil) }
    }
    func pause(_ completion: @escaping (Bool) -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        guard let vm = machine, vm.canPause else { completion(false); return }
        vm.pause { result in completion((try? result.get()) != nil) }
    }
    func resume(_ completion: @escaping (Bool) -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        guard let vm = machine, vm.canResume else { completion(false); return }
        vm.resume { result in completion((try? result.get()) != nil) }
    }
    func stop(_ completion: @escaping (Bool) -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        guard let vm = machine, vm.canStop else { completion(false); return }
        // VZ stop is destructive. Success is not a guest clean-shutdown receipt.
        vm.stop { error in completion(error == nil) }
    }
    func dispose() {
        dispatchPrecondition(condition: .onQueue(queue))
        precondition(state == .stopped || state == .failed)
        machine?.delegate = nil
        machine = nil
        didEnd = nil
    }
    func guestDidStop(_ virtualMachine: VZVirtualMachine) {
        dispatchPrecondition(condition: .onQueue(queue))
        didEnd?()
    }
    func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) {
        dispatchPrecondition(condition: .onQueue(queue))
        didEnd?() // No raw framework error/path/guest output enters diagnostics.
    }
}
