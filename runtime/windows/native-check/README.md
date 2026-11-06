# Standalone Windows native-premises check

This driver is sealed to source `bb477ac73ec2dfc003bb9df9f89ff83ed9182fd1`. CI builds Rust 1.98.1 MSVC x64 with static CRT. The operator needs no Rust, Visual Studio, Python or helper script.

It uses only original runtime producer 42120608528 at `006b30f02a3c9cec61f8f3cc32fde6b0d248fc34`. It verifies the authenticated producer selection, both ZIP inventories and the unchanged external package/source locks before extraction. It never dispatches a producer or patches QEMU. Hashes are engineering verification, not remote attestation or proof of binary-to-source equivalence.

## Run the full check

Use an unelevated PowerShell session on the accepted Windows x64 API baseline, with absolute paths:

```powershell
& $Exe check $Runtime $Pin $NewWorkRoot
if ($LASTEXITCODE -ne 0) { throw 'Native check failed; retain its diagnostics' }
```

`$NewWorkRoot` must be new, with an existing parent. Choose a short local path, such as a new directory under `C:\bridge-try`. AF_UNIX has a 108-byte UTF-8 field, including its terminating NUL. The driver fails with a clear error if the complete endpoint exceeds 107 bytes. Do not change transport, repair an ACL, broaden sharing or elevate to bypass a failure.

Success requires exit zero, one final JSON receipt with `status: passed`, the exact source head, an unelevated caller, all four cases and the `PASS` line:

- **smoke:** restricted atomic launch, exclusion of unrelated handles, protected AF directory and exact socket owner/ACL/tag checks, kernel peer PID bound to the retained QEMU, correlated successful QMP greeting/capability/status/quit replies and observed exit;
- **hold-created:** parent death before the suspended child is resumed;
- **hold-pipe:** the retained historical case name now holds an established, validated AF connection while the parent is killed;
- **hold-guard-only:** the parent successfully closes its known ownership-file handle; the explicit child inheritance is checked and a competing launcher sees Busy while the child is alive. The owned Job is then stopped while the parent remains alive, and the same file must become free before the parent exits. This tests that the child, not a remaining parent handle, held the file. Incidental parent handle 0x48 metadata is logged for information only; complete parent PSS metadata is not required.

Smoke requires exact QMP status `prelaunch` with `running: false`, quit acknowledgment and exit code zero. It also starts a second identical no-VM process, then drops its live control connection and requires owned-Job teardown with observed exit. Bridge never reconnects after a terminal channel event. This does not promise to beat QEMU's internal re-listen race against a hostile same-user actor.

The invocation is fixed: `--preconfig -machine none -accel qtest -S`. No kernel, initrd, disk, guest or WHPX partition is launched. qtest is not a production accelerator fallback. Inert guest fixtures satisfy opaque input verification but are never boot arguments. Guest assets, real-token behavior, WHPX usability and release readiness are not qualified here.

## CI-only account challenge

CI runs the same four premises under a fresh standard account, adding mandatory `--ci-access-proof`. A second distinct unelevated account must connect to the separate positive-control socket with the correct worker PID, then receive WSAEACCES 10013 for the actual private endpoint. Evidence is bound to the same source and endpoint before the owner consumes QMP. A skipped challenge or generic connection failure is not a pass.

The default operator CLI does not create accounts, authenticate, elevate or install anything. CI's account challenge adds proof; it does not replace any metadata, token, Job, PID or ownership check, nor Tom's matching four-case run.

The controller owns a noninherited kill-on-close Job that remains alive while each worker-death test observes the inner Job. Production QEMU still uses suspended atomic Job-list assignment and an explicit handle list. Terminal I/O, connection Drop and process Drop share one original 45-second stop budget. Failed observation quarantines the complete bundle, including original stdio, endpoint and socket metadata. Standard-I/O consumers receive noninheritable working duplicates once.

## Raw evidence and artifacts

Raw received QMP lines are written before parsing; command bytes are recorded before their write attempt, with successful delivery proved by correlated replies. Smoke and disconnect RX/TX logs plus the correlated reply/quit summary are retained. Failed CI runs retain any raw files that were actually produced. A separate diagnostics-export directory permits only the primary CI account, SYSTEM and staging administrator; private endpoints and runtime inputs are never widened for upload. Operator logs remain beneath the operator's own private workroot.

The flat artifact `bridge-windows-native-check-<driver-sha>-<attempt>` lasts seven days. It contains unchanged runtime/source ZIPs, original checksums, executable, sealed pin, provenance, independent verification and actual diagnostics. `operator-sha256sums.txt` hashes only explicit files that exist. Missing-file or failed qualification artifacts are evidence, not ready kits. No credentials, toolchain or private endpoint tree is uploaded.

After a complete successful run, select its actual run ID, artifact name and reviewed checksums. An existing authenticated WSL GitHub login can download it:

```powershell
wsl.exe -- gh run download $RunId --repo orka-agents/orka-bridge --name $ArtifactName --dir $NewWslDestination
if ($LASTEXITCODE -ne 0) { throw 'Download failed' }
```

Use a new destination mapped to a Windows directory. Verify the selected run/head/conclusion, every operator manifest entry and both original ZIP checksums before extraction. Preserve the source ZIP and notices. Run the matching prebuilt executable unelevated and retain its receipt and raw logs.

No run ID or digest is invented here. PR 51 may merge only after complete four-case CI and Tom's matching check pass on the same final source/runtime. Neither application tests nor earlier failed AF experiments clear that gate.
