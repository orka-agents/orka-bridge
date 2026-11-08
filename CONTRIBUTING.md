# Contributing to Bridge

See the [README contribution terms](README.md#contributing) for the Microsoft
CLA and Code of Conduct.

## Development

Use Node 26.5.0 and npm 11.17.0. Rust is pinned to 1.98.1 in
`app/src-tauri/rust-toolchain.toml`. Install the
[Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/) for
your operating system.

From `app/`:

```sh
npm ci
npm run tauri dev
```

Dependency lifecycle scripts are disabled in `.npmrc`. The pinned optional
dependencies supply platform binaries; do not enable scripts globally to install.

## Engine diagnostics

Clusters reports engine availability, not cluster readiness. The default VM
transport is not implemented yet; opening the app does not require kmx.
For development only, set `BRIDGE_DEV_KMX_PATH` to a kmx executable before
`npm run tauri dev`. Release builds ignore this override and cannot select
the development process transport. No helper is discovered or downloaded.

The native engine bridge accepts only version and read-only Agent list requests. No-argument setup commands read state and build requirements, retain the local workspace choice and start its configured development runtime, or read the fixed example. Build requirements come from the target's image pins and shared launch allocation; release builds return no engineering image requirements. They accept no paths, targets or credentials from the renderer.
Replies contain validated version or identity fields, not raw diagnostics.
No credential bytes, paths or model selectors are accepted from the renderer. A no-argument development command explicitly uses the stored GitHub CLI sign-in for the running local VM. Closing the main window signals cancellation. While the app's
async runtime is running, cancellation kills and reaps the owned direct process;
app exit does not wait for that cleanup. Neither grandchildren nor remote work
are guaranteed to stop.

Run debug engine tests and clippy as well as release checks: process transport
code is compiled only when debug assertions are enabled.

## Native Run core

Rust callers can parse an exact agent/test byte pair and use `engine::run` to
submit fresh Orka Agents and Tasks, then observe acknowledged identities using
GET only. Targets and bearer credentials must come from trusted native config,
not the renderer; there is no ambient cluster or provider fallback. The fixed
example's Run and status commands take no arguments.
Supports schemaVersion 1 source with tool entries declared `kind: builtin`, plus
`contains` and `toolCalled`. Unknown fields refuse before writes. This narrow
parser expects unindented backtick `yaml agent-test` fences; it is not a general
Markdown reader or an effective-tool policy check. Completed attributable tool
events are positive evidence only; missing/unavailable events stay unknown. A failed
Task cannot pass, even when individual assertions match. Requests and observation
are bounded; timeout/drop does not stop remote work. Retain the in-memory handle
and error's acknowledged/possibly-created identities. No POST retry, overwrite,
adoption, deletion or persisted history is provided.

In a development app with a running local VM and an acknowledged model connection,
Designer can run the unchanged web-reader pair. The same serialized VM client carries
Run requests; no host Orka port or second VM client is opened. The result shows the
answer, attributable completed tool names, `contains` and `toolCalled` verdicts.
Opening the example or reading Run status does not submit work. Duplicate Run calls
while it is active only observe it. A dropped page or IPC waiter does not lose the
native receipts; uncertain submissions retain acknowledged and possibly-created names.

The guest image must include the current Orka request handler. Older images cannot
run this path and report an unconfirmed result rather than replaying writes. On first
Run, the guest creates or verifies its `bridge-native-run` ServiceAccount, namespaced
Role and RoleBinding in `orka-system`. The Role permits create/get on Agents and Tasks
and get on Tools and Providers, not Secrets. A ten-minute Kubernetes bearer stays in
guest memory and is renewed before expiry. The guest owns a loopback-only forwarding
process to its Orka service; request diagnostics and bearer values are never returned
to the app. Result bodies, including answers, use the existing guest operation cache.
Closing waits for admitted client I/O and native shutdown; it is not remote Task cancellation.

## Image transport

The native image transport accepts Rust-owned pins, not renderer URLs. It fetches
at most 8 MiB of manifest bytes and verifies the exact SHA-256 before returning
bytes to the manifest reader. HTTPS uses native OS trust, explicit redirect
origins and identity encoding; proxy discovery, automatic retries and transparent
content decoding are disabled. Closing the supplied cancellation channel stops
the transfer. Transport errors omit URLs, queries and server bodies.

The tests generate their CA, certificates and private keys entirely in memory at
runtime. They never write keys or alter the host trust store. This transport does
not yet select a live image catalog, populate a cache or activate an image.

## Verified image cache

Native Rust callers can acquire a resolved pinned image into an app-local-data
cache. Complete chunks resume only for the same manifest and are rehashed before
reuse. Progress starts with a cache verification check, reports cumulative
available asset bytes for any acquisition, including reused chunks, then final
verification and image completion. It does not report setup readiness.
A complete cached set is revalidated and reused without HTTP, including after a
restart. Corrupt cached bytes fail closed rather than triggering a fallback.

The cache uses nonblocking per-pin ownership, no-follow handles, bounded gzip
member decoding and free-space checks. It flushes files and directory metadata,
publishes a complete generation without replacing an existing one, and writes
the final selector last. Interrupted owned staging can resume; nothing is evicted
or garbage-collected. Read capabilities do not freeze other writers, so future
installation code must verify its private destination copies. This API does not
materialize an installation, start a VM or select a production image catalog.
On Windows, schedule cached manifest reads before acquisition: a concurrent
staging read can cause a sharing failure during publication, with staging retained.
The no-follow directory protections are not weakened to avoid this contention.

## Private installed images

The native lifecycle owner can lend its held installation directory and ownership
file to the private image installer. Only that owner can assert live creation
permission. Materialization copies and verifies the private OS, kernel and initrd,
then commits a separate version-one image binding after a no-replace generation
publication. Incomplete work is not boot-ready and never restores creation authority.

Reopen checks the current installation, platform, state compatibility and immutable
boot files, and flushes the generation and parent directories before returning. The initial OS hash remains provenance because the guest can mutate
its private disk. Installed handoff clones contain metadata, not installation locks
or exclusive disk handles; the backend must revalidate under current ownership
and retain its normal container inspection and lifetime controls. This API does not
wire the runtime, start a VM, migrate journals or select a production image.

## Native image handoff

Native callers can use `runtime::setup_adapter::prepare_pinned_image` with a
Rust-owned pin, host facts, cache and HTTP client. It loads the pinned manifest,
acquires verified files and returns a `PreparedImage`. A complete cache is reused
without HTTP. Debug callers can instead use `prepare_development_image` on the
existing verified local source; release builds contain no local-source bridge.

Finish source preparation before taking installation ownership. Then call
`PreparedImage::materialize` with the owner's borrowed installation target and
cancellation receiver. Both sources use the same verified private copies and
binding publication. The actual blocking job retains the owner's counted Arc,
not a duplicated ownership file. The result remains metadata-only.

Engineering identity is explicit: the common manifest-hash slot identifies its
raw pinned boot receipt, not a release manifest. Its compatibility ID is
`engineering-afc68ae1` on Mac and `engineering-a41d68bb` on Windows. Neither source preparation nor materialization starts a
VM or reports whole-setup readiness. Backend selection, launch validation and
the visible setup connection remain the native runtime caller's responsibility.

## Private Apple session

Trusted Mac Rust callers can compose the installed Apple image, journal and
bundled helper with `runtime::apple_session::AppleVmSession`. It verifies serving
Ready before supplying the existing VM engine transport. Pause, resume and stop
observe native helper completion; failed cleanup retains ownership. One revocable
guest descriptor serves one client. The local development factory obtains SDK
identity only when creating a journal, then retains it for reopen. Model and Run
activation remain separate. Real VZ disk-handle release remains unqualified by
these synthetic CI tests.

## Local development image

Debug builds can read a local engineering image with `BRIDGE_DEV_IMAGE_DIR`.
Set it to a folder containing `boot-assets.json`, `os.kernel`, `os.initrd`, and
`os.raw` on Apple silicon Mac or `os.qcow2` on Windows x64, then run
`npm run tauri dev`. Assemble all four matching files in that folder; do not
use partial boot exports or convert the disk format.

This temporary source accepts the pinned `afc68ae1` Mac and `a41d68bb`
Windows engineering sets with the `orka-7916cd1c-native/v1` restricted native profile.
After Continue on the local workspace choice, it checks the exact receipt and payload lengths and
SHA-256 on a background worker, then installs verified private copies under the
existing journal owner before boot. The factory is never opened for writing.
A bad set fails without download or alternate-image fallback. Selection and
installation are not readiness: the screen reports a running runtime only after
the delivered native session establishes the guest connection.

Windows development builds also need the authenticated portable QEMU tree at
`native/qemu` beside the executable. Its `provenance/runtime-manifest.json` is
pinned to 117615 bytes and SHA-256
`a1dead19c0d1dcef558008e556cd9bd4a8c5082daf4ecea6f531d03173748385`.
That hash authenticates the inventory before all 416 files, including the
manifest, are verified and retained. The app uses WHPX, not PATH discovery, a
host QEMU installation, WSL or an accelerator fallback. CI's Windows artifact
includes an embedded-frontend development executable under `development/`;
stage the matching portable tree before launching it.

The selector, verifier and engineering pins are compiled out when debug
assertions are disabled. Release builds cannot enable this source through an
environment variable or feature flag. This is not the production image
acquisition path or publication approval. No model credentials or inference
are exercised by starting this local runtime. Windows development pins now use the rebuilt guest model handler and kmx Connect fix; Mac pins remain unchanged and its model step stays locked pending a matching image. Existing installations with the old Windows compatibility ID are retained and refused, not reset or upgraded.

## Checks and builds

Run the commands in [AGENTS.md](AGENTS.md#checks). Also run `npm audit` to
inspect development-tool advisories; `npm audit --omit=dev` checks the shipped
frontend dependency graph. Do not apply forced major-version audit fixes blindly.

For a Windows x64 executable:

```sh
npm run tauri -- build --no-bundle -- --locked
```

For an Apple silicon Mac app, install the Xcode Swift compiler first. Mac Cargo
builds, tests, clippy and Tauri development compile and ad-hoc sign the bundled
Apple helper before resource processing; no toolchain is installed automatically.
The arm64 helper is also included in Intel Mac shell builds; this does not add
an Intel VM backend. Building or opening the app does not start a VM.
See [the native helper contract](app/src-tauri/native/macos/README.md).

For the app bundle:

```sh
npm run tauri -- build --bundles app --no-sign --no-binary-patching -- --locked
```

The Mac bundle includes readable notices in its Resources directory; CI also
places them beside the portable Windows executable. Both builds embed them in
the frontend. Preserve these files when redistributing builds.

The app opens Clusters with a one-time welcome followed by a workspace chooser. Get started records only a webview presentation preference; an existing native local choice bypasses the welcome. The cluster route is unavailable on both platforms. Selecting the local radio has no effect; Continue retains an empty preference marker at `setup-v1/local-vm` under the app data directory and starts the configured development runtime. A saved choice resumes at native initialization, reusing its committed installed image, journal identity and state disk without needing the factory source. Interrupted installations without a committed image remain errors; no reset, recreation or guessed retry is performed. Closing or quitting bars work and waits for startup, active engine I/O and the session's shutdown attempt. Failed shutdown remains a failure with the existing backend retention behavior. Selection is never readiness. The fixed web-reader pair can be opened read-only in Designer; opening it does not submit Agent or Task resources. A running development VM unlocks Use GitHub sign-in. It captures the existing github.com account through the supported GitHub CLI, stores it natively and connects copilot/gpt-5.6-terra in the same guest session. It does not open browser login, install GitHub CLI or use ambient token overrides. The screen reports a Connect acknowledgment or the finite failing stage, never raw provider diagnostics. Other builds keep the model step locked. Trusted native Rust callers can associate a captured sign-in with an installation-bound OS-store slot, deliver it over the private guest channel and invoke the fixed kmx Connect operation. Setup stores only opaque association and acknowledged lifecycle metadata, never credential bytes. Uncertain completion is retained without automatic capture or replay; explicit sign-out acknowledges Disconnect and file clear before removing the host slot. These calls require a rebuilt guest handler and matching kmx bundle; a Connect acknowledgment does not prove model inference. Connected or uncertain bindings cannot yet be reused or removed after a new VM lease; confirmed-owned-stop recovery remains a follow-up. A previously acknowledged clear still permits host-only cleanup. Engine diagnostics remain available in Clusters;
native webviews can still create their usual cache/profile directories under
the app identifier `ai.orka.bridge`.

CI artifacts expire after three days. They are test builds, not a release:
no installer, Developer-ID signing, notarization or updater is provided.
Browser-downloaded Mac artifacts may be blocked by Gatekeeper. Windows needs
WebView2. Mac archives preserve executable permissions.

## Native smoke walk

The Mac CI job launches its extracted app bundle, uses pointer and keyboard
navigation, checks captured page headings and resizes the window to its
minimum dimensions. On first run it advances from the welcome to the workspace choice, then uses Continue. In Clusters it checks that release setup reports unavailable workspace integration and a locked model step, and verifies the native choice remains selected without a welcome after relaunch. Relaunch checks only the preference, not VM reopen. The walk runs in both system themes, temporarily enables
macOS Keyboard Navigation, then restores the runner's prior preferences. Captures contain only the app window.
Review visible changes in these captures and report anything not exercised,
such as platform input feel or installer behavior.
