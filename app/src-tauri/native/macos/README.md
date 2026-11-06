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
must supply a nonblocking output sink: true means all bytes were actually written,
not queued. Partial writes fail without replay. The I/O adapter must suppress
SIGPIPE, handle supervisor EOF and TERM, and call the core on its serial queue.
The terminal callback may exit zero only for a completed successful S request;
that is not a guest clean-shutdown acknowledgment. Failed or unobserved teardown
retains ownership until terminal observation or the parent's bounded process kill.

Image materialization/binding, stable identity, EFI/device configuration, guest
channel extraction, fixed executable resolution, signing and bundling are not
implemented here. There is no image-path fallback, new guest protocol, host-folder
mount, renderer interface or VM activation. Host sleep/death reason events require
a later protocol revision before activation. Native VM/disk-close and process-death
qualification remain separate from fake-driver and native compilation evidence.
