import Foundation
import Darwin

enum SupervisorIOError: Error { case invalidDescriptors, system }

// Borrows the trusted parent's capabilities. Parsing never closes or reopens one.
struct InheritedDescriptors {
    let supervisor: Int32
    let guest: Int32
    let ownership: Int32

    static func parse(_ args: [String]) throws -> InheritedDescriptors {
        let names = ["--supervisor-fd", "--guest-fd", "--ownership-fd"]
        guard args.count == 6 else { throw SupervisorIOError.invalidDescriptors }
        var values: [String: Int32] = [:]
        for i in stride(from: 0, to: args.count, by: 2) {
            let name = args[i], number = args[i + 1]
            guard names.contains(name), values[name] == nil, !number.isEmpty,
                  number.utf8.allSatisfy({ $0 >= 48 && $0 <= 57 }),
                  let fd = Int32(number), fd >= 0 else { throw SupervisorIOError.invalidDescriptors }
            values[name] = fd
        }
        guard let supervisor = values[names[0]], let guest = values[names[1]],
              let ownership = values[names[2]], Set(values.values).count == 3,
              stream(supervisor), stream(guest) else { throw SupervisorIOError.invalidDescriptors }
        var info = stat()
        guard fstat(ownership, &info) == 0, info.st_mode & mode_t(S_IFMT) == mode_t(S_IFREG) else {
            throw SupervisorIOError.invalidDescriptors
        }
        return InheritedDescriptors(supervisor: supervisor, guest: guest, ownership: ownership)
    }

    private static func stream(_ fd: Int32) -> Bool {
        var kind: Int32 = 0
        var size = socklen_t(MemoryLayout<Int32>.size)
        return getsockopt(fd, SOL_SOCKET, SO_TYPE, &kind, &size) == 0 && kind == SOCK_STREAM
    }
}
