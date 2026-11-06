import Foundation
import Darwin

enum SupervisorIOError: Error { case invalidDescriptors, system }
struct InheritedDescriptors {
    let supervisor: Int32
    let guest: Int32
    let ownership: Int32
    static func parse(_ args: [String]) throws -> InheritedDescriptors { throw SupervisorIOError.invalidDescriptors }
}
