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

The native engine bridge accepts only version and read-only Agent list requests. Two separate no-argument setup commands read availability and retain the local VM choice; neither starts a runtime or handles credentials.
Replies contain validated version or identity fields, not raw diagnostics.
No credential input channel is provided here; runtime integration must define
that separately. Closing the main window signals cancellation. While the app's
async runtime is running, cancellation kills and reaps the owned direct process;
app exit does not wait for that cleanup. Neither grandchildren nor remote work
are guaranteed to stop.

Run debug engine tests and clippy as well as release checks: process transport
code is compiled only when debug assertions are enabled.

## Native Run core

Rust callers can parse an exact agent/test byte pair and use `engine::run` to
submit fresh Orka Agents and Tasks, then observe acknowledged identities using
GET only. Targets and bearer credentials must come from trusted native config,
not the renderer; there is no ambient cluster, provider fallback or new IPC.
Only schemaVersion 1, Orka builtin tools, `contains` and `toolCalled` are supported.
Unsupported semantics refuse before writes. Completed attributable tool events
are positive evidence only; missing/unavailable events stay unknown. A failed
Task cannot pass, even when individual assertions match. Requests and observation
are bounded; timeout/drop does not stop remote work. Retain the in-memory handle
and error's acknowledged/possibly-created identities. No POST retry, overwrite,
adoption, deletion or persisted history is provided. VM/chooser hookup is separate.

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

## Local development image

Debug builds can read a local engineering image with `BRIDGE_DEV_IMAGE_DIR`.
Set it to a folder containing `boot-assets.json`, `os.kernel`, `os.initrd`, and
`os.raw` on Apple silicon Mac or `os.qcow2` on Windows x64, then run
`npm run tauri dev`. Assemble all four matching files in that folder; do not
use partial boot exports or convert the disk format.

This temporary source accepts only the pinned `afc68ae1` engineering sets
with the `orka-7916cd1c-native/v1` restricted native profile.
It checks the exact receipt and each payload's length and SHA-256 using
read-only handles on a background worker. Its fixed console message reports
verification, not a running VM. A bad set is unavailable, with no download or
alternate-image fallback. The factory is never opened for writing. The app
retains verified source handles for future integration; no backend consumes
them yet. A future consumer must verify its private installed copies before
booting, because a read-only handle does not freeze another writer's changes.

The selector, verifier and engineering pins are compiled out when debug
assertions are disabled. Release builds cannot enable this source through an
environment variable or feature flag. This is not the production image
acquisition path, publication approval, VM startup, or credential delivery;
the runtime chooser and model step remain unchanged.

## Checks and builds

Run the commands in [AGENTS.md](AGENTS.md#checks). Also run `npm audit` to
inspect development-tool advisories; `npm audit --omit=dev` checks the shipped
frontend dependency graph. Do not apply forced major-version audit fixes blindly.

For a Windows x64 executable:

```sh
npm run tauri -- build --no-bundle -- --locked
```

For an Apple silicon Mac app:

```sh
npm run tauri -- build --bundles app --no-sign --no-binary-patching -- --locked
```

The Mac bundle includes readable notices in its Resources directory; CI also
places them beside the portable Windows executable. Both builds embed them in
the frontend. Preserve these files when redistributing builds.

The app opens Clusters with a runtime chooser and a locked model step. Existing cluster is unavailable on both platforms. Local VM selection writes only an empty preference marker at `setup-v1/local-vm` under the app data directory. It does not download, start or reopen a VM, save agents, connect a model or cluster, or run tests. Selection is never readiness. Model connection stays locked pending supported restricted worker access; there is no guest token channel. Engine diagnostics remain available in Clusters. No runtime or credential state is stored by setup;
native webviews can still create their usual cache/profile directories under
the app identifier `ai.orka.bridge`.

CI artifacts expire after three days. They are test builds, not a release:
no installer, Developer-ID signing, notarization or updater is provided.
Browser-downloaded Mac artifacts may be blocked by Gatekeeper. Windows needs
WebView2. Mac archives preserve executable permissions.

## Native smoke walk

The Mac CI job launches its extracted app bundle, uses pointer and keyboard
navigation, checks captured page headings and resizes the window to its
minimum dimensions. In Clusters it also checks the actual release setup IPC reports unavailable VM integration and a locked model step, and verifies a native choice remains selected after relaunch. Relaunch checks only the preference, not VM reopen. The walk runs in both system themes, temporarily enables
macOS Keyboard Navigation, then restores the runner's prior preferences. Captures contain only the app window.
Review visible changes in these captures and report anything not exercised,
such as platform input feel or installer behavior.
