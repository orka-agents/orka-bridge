# Private Apple helper core

`BridgeVM` owns lifecycle and the existing private v1 supervisor policy. It is
production helper code, not an app backend registration or executable launcher.
`VirtualizationDriver` validates a complete native configuration before creating
a VZ VM, on the same serial queue used by the core. Tests use deferred fake VM
operations; no test creates a valid native VM or boots a guest.

Run `bash scripts/test-vm-helper-macos.sh` from the repository root on Apple silicon.
It compiles production sources independently, then runs the actual-source tests
with a macOS 13 minimum target and warnings treated as errors.

The future verified bootstrap must transfer a configured, unstarted driver and
an inherited ownership FD, retaining no other VM/configuration references. It
uses `InheritedDescriptors` to borrow and validate the parent's three FD arguments.
`SupervisorSocket` takes the supervisor FD, enables nonblocking writes and suppresses
SIGPIPE. Its sink returns true only for a completed full write; partial writes fail
without replay. Create the socket and helper-only `TerminationObserver` before
transferring ownership to the configured core. Bind the read and peer-loss callbacks,
then use `observer.whenRegistered { core.start() }` to wait for kernel signal
registration; resuming a dispatch source alone is not that observation. The readiness
callback holds the unstarted core until registration and is cleared before use or
on cancellation. All components receive the same serial queue; the supervisor never
reads the guest endpoint.

The socket cancel handler closes its FD only after active read handlers drain.
The TERM observer restores the prior disposition after its handlers drain. Terminal
composition waits for both close completions before exiting; close or restoration
failure is unclean. Create both adapters before transferring the ownership FD to the
core. A failed construction must dispose any configured, unstarted driver before
closing that ownership FD. These libraries do not provide the executable bootstrap.
The terminal callback may exit zero only for a completed successful S request;
that is not a guest clean-shutdown acknowledgment. Failed or unobserved teardown
retains ownership until terminal observation or the parent's bounded process kill.

Image materialization/binding, stable identity, EFI/device configuration, guest
channel extraction, fixed executable resolution, signing and bundling are not
implemented here. There is no image-path fallback, new guest protocol, host-folder
mount, renderer interface or VM activation. Host sleep/death reason events require
a later protocol revision before activation. Native VM/disk-close and process-death
qualification remain separate from fake-driver and native compilation evidence.
**Actual VZ attachment and callback/resource-release validation must pass before
backend activation.** The native tests use socketpairs, signals and a fake driver;
they do not instantiate a valid VZ VM, attach disks or boot a guest.
