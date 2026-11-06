import Foundation

enum MachineState { case stopped, running, paused, transitioning, failed }
protocol MachineDriver: AnyObject {
    var state: MachineState { get }
    var canStart: Bool { get }
    var canPause: Bool { get }
    var canResume: Bool { get }
    var canStop: Bool { get }
    var didEnd: (() -> Void)? { get set }
    func start(_ completion: @escaping (Bool) -> Void)
    func pause(_ completion: @escaping (Bool) -> Void)
    func resume(_ completion: @escaping (Bool) -> Void)
    func stop(_ completion: @escaping (Bool) -> Void)
    func dispose()
}

// Compilable inert seam for the first native behavioural RED, not a VM launcher.
final class BridgeVM {
    private let owner: FileHandle
    init(driver: MachineDriver, ownershipFD: Int32, queue: DispatchQueue,
         output: @escaping ([UInt8]) -> Bool, terminal: @escaping (Bool) -> Void) {
        owner = FileHandle(fileDescriptor: ownershipFD, closeOnDealloc: true)
    }
    func start() {}
    func receive(_ bytes: [UInt8]) {}
    func peerClosed() {}
    static func successor(_ id: UInt64) -> UInt64? { nil }
}
