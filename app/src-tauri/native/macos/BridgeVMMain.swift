import Foundation

@main
private struct BridgeVMMain {
    static func main() {
        let queue = DispatchQueue(label: "ai.orka.bridge.vm")
        let arguments = Array(CommandLine.arguments.dropFirst())
        if arguments.first == "--machine-identifier" {
            guard arguments == ["--machine-identifier"] else { exit(1) }
            let data = HelperBootstrap.machineIdentifier()
            guard (1...4096).contains(data.count) else { exit(1) }
            do { try FileHandle.standardOutput.write(contentsOf: data); exit(0) }
            catch { exit(1) }
        }
        queue.async {
            HelperBootstrap.run(arguments: arguments, queue: queue,
                                makeDriver: HelperBootstrap.makeDriver,
                                finish: { exit($0) })
        }
        dispatchMain()
    }
}
