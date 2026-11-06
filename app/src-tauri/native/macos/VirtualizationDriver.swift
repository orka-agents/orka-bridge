import Virtualization

// Inert mapping seam for native behavioural RED; no native VM instance.
final class VirtualizationDriver {
    static func classify(_ state: VZVirtualMachine.State) -> MachineState { .transitioning }
}
