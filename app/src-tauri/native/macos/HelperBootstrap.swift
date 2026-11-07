import Foundation
import Virtualization
import Darwin

private final class BootstrapCoreReference { weak var value: BridgeVM? }

// Standalone helper composition only. No app registration or guest handshake.
enum HelperBootstrap {
    static func machineIdentifier() -> Data { VZGenericMachineIdentifier().dataRepresentation }
    static func terminalExitCode(cleanStop: Bool, socketClosed: Bool, signalRestored: Bool) -> Int32 {
        cleanStop && socketClosed && signalRestored ? 0 : 1
    }

    static func run(arguments: [String], queue: DispatchQueue,
                    makeDriver: (HelperConfiguration, Int32, DispatchQueue) throws -> MachineDriver,
                    finish: @escaping (Int32) -> Void) {
        dispatchPrecondition(condition: .onQueue(queue))
        // Rejected input is borrowed: never close arbitrary argument-selected FDs.
        guard let input = try? HelperArguments.parse(arguments) else { finish(1); return }
        var supervisor: Int32? = input.descriptors.supervisor
        var guest: Int32? = input.descriptors.guest
        var ownership: Int32? = input.descriptors.ownership
        var driver: MachineDriver?
        var socket: SupervisorSocket?
        var observer: TerminationObserver?
        do {
            let configuration = try HelperConfiguration.read(owningFD: input.configFD)
            // The factory adopts guest even on throw; no second cleanup owner.
            guest = nil
            let machine = try makeDriver(configuration, input.descriptors.guest, queue)
            driver = machine
            supervisor = nil // SupervisorSocket adopts on invocation, including failure.
            let io = try SupervisorSocket(owningFD: input.descriptors.supervisor, queue: queue)
            socket = io
            let reference = BootstrapCoreReference()
            let term = try TerminationObserver(queue: queue, onTermination: { reference.value?.peerClosed() })
            observer = term
            // No throwable construction remains after the ownership transfer.
            let core = BridgeVM(driver: machine, ownershipFD: input.descriptors.ownership, queue: queue,
                                output: { io.send($0) }, terminal: { clean in
                closeAdapters(socket: io, observer: term) { socketOK, signalOK in
                    finish(terminalExitCode(cleanStop: clean, socketClosed: socketOK, signalRestored: signalOK))
                }
            })
            ownership = nil
            driver = nil
            reference.value = core
            io.start(receive: { reference.value?.receive($0) }, peerLost: { reference.value?.peerClosed() })
            // The registration callback holds the unstarted core until the kernel
            // monitor is ready; DispatchSource.resume alone is not that evidence.
            term.whenRegistered { core.start() }
        } catch {
            // An unstarted native VM and its guest handles go before ownership.
            driver?.dispose()
            driver = nil
            if let fd = guest { _ = close(fd) }
            if let fd = supervisor { _ = close(fd) }
            if let fd = ownership { _ = close(fd) }
            closeAdapters(socket: socket, observer: observer) { _, _ in finish(1) }
        }
    }

    static func makeDriver(configuration: HelperConfiguration, guestFD: Int32,
                           queue: DispatchQueue) throws -> MachineDriver {
        dispatchPrecondition(condition: .onQueue(queue))
        let guest = FileHandle(fileDescriptor: guestFD, closeOnDealloc: true)
        var diagnostic: FileHandle?
        do {
            let sink = try FileHandle(forWritingTo: URL(fileURLWithPath: "/dev/null"))
            diagnostic = sink
            let native = try configuration.makeConfiguration(guest: guest, diagnostic: sink, queue: queue)
            return try VirtualizationDriver(configuration: native, queue: queue, ownedHandles: [guest, sink])
        } catch {
            try? guest.close()
            try? diagnostic?.close()
            throw HelperConfigurationError.system
        }
    }

    private static func closeAdapters(socket: SupervisorSocket?, observer: TerminationObserver?,
                                      completion: @escaping (Bool, Bool) -> Void) {
        let afterSocket: (Bool) -> Void = { socketOK in
            if let observer { observer.close { completion(socketOK, $0) } }
            else { completion(socketOK, true) }
        }
        if let socket { socket.close(afterSocket) }
        else { afterSocket(true) }
    }
}
