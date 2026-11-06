import Foundation
import Darwin

// Inert seam for native behavioural RED; no VM or process bootstrap.
final class SupervisorSocket {
    private let owner: FileHandle
    init(owningFD: Int32, queue: DispatchQueue) throws { owner = FileHandle(fileDescriptor: owningFD, closeOnDealloc: true) }
    func start(receive: @escaping ([UInt8]) -> Void, peerLost: @escaping () -> Void) {}
    func send(_ bytes: [UInt8]) -> Bool { false }
    func close(_ completion: @escaping (Bool) -> Void) { completion(true) }
}
