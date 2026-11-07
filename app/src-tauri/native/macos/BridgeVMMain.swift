import Foundation

@main
private struct BridgeVMMain {
    static func main() {
        let queue = DispatchQueue(label: "ai.orka.bridge.vm")
        let arguments = Array(CommandLine.arguments.dropFirst())
        queue.async {
            HelperBootstrap.run(arguments: arguments, queue: queue,
                                makeDriver: HelperBootstrap.makeDriver,
                                finish: { exit($0) })
        }
        dispatchMain()
    }
}
