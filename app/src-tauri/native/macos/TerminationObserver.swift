import Foundation
import Darwin

final class TerminationObserver {
    init(queue: DispatchQueue, onTermination: @escaping () -> Void) throws {}
    func close(_ completion: @escaping (Bool) -> Void) { completion(true) }
}
