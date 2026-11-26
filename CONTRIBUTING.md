# Contributing to Bridge

See the [README contribution terms](README.md#contributing) for the Microsoft
CLA and Code of Conduct.

## Build and run

Install Node 26.5.0, npm 11.17.0, Rust 1.98.1 and the
[Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/) for your
operating system. Install Rust through rustup so the `cargo +1.98.1` checks work.
Mac builds also need the Xcode Swift compiler.
Rust is pinned in `app/src-tauri/rust-toolchain.toml`.

From the repository root:

```sh
cd app
npm ci
npm run tauri dev
```

The default window is 1280×800, with a 540×640 minimum. The native smoke walk also captures a 540×900 content area in both system themes.

This opens the native app with frontend hot reload. You can edit the UI without
a VM; running an agent needs the local development image described below.
Dependency lifecycle scripts are disabled in `app/.npmrc`. Do not enable them
globally to install dependencies.

## Preview the UI in a browser

For quick UI work without a VM, start the dev server from `app/` with `npm run dev` and open `http://127.0.0.1:1420/?scenario=fresh`. In development builds only, a `scenario` query loads a scripted stand-in for the native commands (`app/src/preview/backend.ts`), so screens can be walked and captured in any browser. Scenarios are `fresh`, `ready`, `connected`, `restart`, `ghMissing`, `ghSignedOut`, `runFails` and `vmFailed`. Add `speed=20` to shorten waits, `clean` to hide the scenario toolbar and `page=Home`, `Overview`, `Designer` or `Clusters` to open that page. Like the app, the preview opens on Welcome while nothing is set up (`fresh`, `ghMissing`, `ghSignedOut`, `vmFailed`), on Clusters while setup is in progress (`restart`), and on Home once a workspace is ready (`ready`, `connected`, `runFails`, `clusterReady`). The preview is not native verification: it never runs a VM, an agent or a model.

These scenarios open states the ordinary walk can't reach. The workspace ones open on This computer, `memoryLow` and the `first…` ones open on Welcome in the guided first run, and the rest open on Home; the steps say where the state appears, starting from Clusters unless they name another page.

| Scenario | State | Where it appears |
| --- | --- | --- |
| `memoryLow` | Not enough installed memory | Get started → Set up |
| `wsStopped` | Workspace stopped | Opens there |
| `wsCouldntStart` | Workspace couldn't start | Opens there |
| `wsForced` | Workspace forcibly stopped | Opens there |
| `wsUpdate` | This version needs a new workspace | Opens there |
| `wsRestore` | Interrupted update, restore offered | Opens there |
| `wsRecovery` | Interrupted update, setup incomplete | Opens there |
| `wsStarting` | First setup, the workspace starting with its progress | Opens there |
| `firstDownloadFails` | First run: the workspace download fails partway, then Try again finishes it | Get started → Set up → Continue |
| `firstTestFails` | First run: the first agent's test fails | Get started → Set up → Continue, then Connect the AI |
| `firstTestStale` | First run: the first test's result is from an earlier edit, so it needs running again | Get started → Set up → Continue, then Connect the AI |
| `firstTestUnknown` | First run: the first test's result can't be confirmed | Get started → Set up → Continue, then Connect the AI |
| `keyRejected` | API key not accepted | This computer → Manage → Your own API key, enter a key starting `sk-`, Connect the AI |
| `aiMismatch` | Saved AI connection from an earlier setup | This computer → Manage → Connect the AI |
| `clusterDenied` | Cluster access denied | `team-dev` → Use this cluster |
| `clusterUnreachable` | Can't reach the cluster | `team-dev` |
| `clusterContexts` | kubectl contexts with one Orka installation (`team-dev`), two (`team-shared`), none (`kind-scratch`) and one that needs a ServiceAccount (`team-restricted`) | Add a cluster → kubectl context, then choose one; `team-restricted` asks for a ServiceAccount after Use this cluster |
| `sendBlocked` | Send blocked by destination checks | `team-dev` → Use this cluster → Open your agent |
| `sendRunning` | Send running on the cluster (until cancelled) | As `sendBlocked`, then Send and run tests |
| `sendFailed` | Send finished with a failing test | As `sendRunning` |
| `sendPassed` | Send finished with every test passing | As `sendRunning` |
| `sendLocalFailed` | Local test failed, cluster ready to send | As `sendBlocked`, then Run tests, then Send and run tests for the warning |
| `testIncomplete` | First test is incomplete | Designer, or add `page=Designer` |
| `runStale` | Local result from before your last edit | Designer, or add `page=Designer` |
| `overviewEmpty` | No saved agents yet | Overview, or add `page=Overview` |
| `navThinking` | The Navigator thinking about an answer, with Stop | Opens there |
| `navReplyFailed` | The Navigator couldn't answer | Opens there |
| `navNoModel` | No model connected: the message box is disabled | Opens there (the same as `ready`) |
| `navExplain` | The Navigator explains Orka with a card of terms | Opens there |
| `navTask` | A proposed task card; Edit, then Create task runs it to Succeeded; Stop task while it runs cancels it | Opens there |
| `navTaskFails` | As `navTask`, but the task fails | Opens there, then Create task |
| `navAgent` | A proposed agent card; Create agent writes the file | Opens there |

The Navigator answers in every scenario with the AI connected, with scripted replies chosen by the question: a link or checksum gets a task card, "agent" an agent card, "new to Orka" an explanation, and a repo, monitor or charter question an explanation with no card, as Bridge can't set those up yet. It runs on This computer only: choose `team-dev` in the Orka switcher in `clusterUnreachable` for the can't-reach card, or in `clusterReady` for the card that offers This computer.

In every scenario a local test run can be stopped while it runs, and a finished run is recorded for Overview with its time, as native local results are. A run keeps the files it started from, so editing the agent afterwards shows its result as not run since your edit.

## Run with a local VM

Local VM development needs a debug build on Windows x64 or Apple silicon Mac.
Set `BRIDGE_DEV_IMAGE_DIR` to a folder containing all four matching files:

- `boot-assets.json`
- `os.kernel`
- `os.initrd`
- `os.qcow2` on Windows, or `os.raw` on Mac

The files must match the sizes and SHA-256 hashes in
[`dev_image.rs`](app/src-tauri/src/dev_image.rs). Do not mix files from different
builds or convert the disk format. Ask a maintainer for a matching development
image. [The guest image build guide](images/guest/README.md) describes image
preparation, but does not reproduce these pinned files. Bridge installs private
copies; it does not modify this source folder. A previously installed compatible image can reopen without it.

### Windows x64

Enable Windows Hypervisor Platform. Bridge uses WHPX, not WSL or a host QEMU
installation. Extract `bridge-windows-runtime-portable.zip` to `native\qemu`
beside the executable. Its `provenance\runtime-manifest.json` must match the pin
in [`local_vm_native.rs`](app/src-tauri/src/local_vm_native.rs). Keep the tree
and notices intact. See [the Windows runtime producer](runtime/windows/README.md)
for artifact download instructions.

From `app/`, in PowerShell:

```powershell
$env:BRIDGE_DEV_IMAGE_DIR = 'C:\path\to\image'
npm run tauri -- build --debug --no-bundle -- --locked
```

Still in `app/`, place the QEMU tree at `src-tauri\target\debug\native\qemu`,
then launch the app:

```powershell
.\src-tauri\target\debug\orka-bridge.exe
```

The `bridge-windows-x64` CI artifact includes an embedded-frontend debug build
at `development\orka-bridge.exe`. Place QEMU at `development\native\qemu`, set
`BRIDGE_DEV_IMAGE_DIR` in PowerShell, and run `development\orka-bridge.exe`
from that shell. The executable at the artifact root is a release build and
cannot use the local development image.

### Apple silicon Mac

From `app/`:

```sh
BRIDGE_DEV_IMAGE_DIR=/path/to/image npm run tauri dev
```

Mac builds compile and ad-hoc sign the bundled VM helper automatically.
See [the helper build instructions](app/src-tauri/native/macos/README.md).
Intel Macs can build the app but cannot run its local VM. Both pinned images
support model Connect, the workspace catalog and the example agent.

In Clusters, dismiss the welcome with Get started (it returns at each launch until a workspace or cluster is set up), then press Set up
on This computer's place and Continue after its computer checks. On Windows and
Apple-silicon Mac, the running VM unlocks Use GitHub sign-in, which needs GitHub CLI already installed and signed
in to github.com. Bridge does not install it or open browser sign-in. Starting
a VM alone does not verify model inference. Local images are debug-only;
release builds ignore `BRIDGE_DEV_IMAGE_DIR`. Supported release builds select
reviewed hash-pinned images from the image-only prerelease and download them
anonymously. Before that prerelease is published, startup reports a missing-image
download error; it never falls back to GitHub credentials. A release candidate
still needs the [preview walks](docs/preview.md) before app publication.

## Reopen and remove the saved sign-in

After **Quit Bridge**, reopen Bridge and use **Use GitHub sign-in**
once the workspace is running. Bridge reuses its native-store copy without
reading GitHub CLI again. A new VM boot first disconnects the prior owned model
route. This requires an image built with the retained-connection cleanup handler;
older development images cannot reconnect across boots. Build and pin the new
guest together with its matching app and a new source-bound state compatibility
ID. New guest replies are not supported by older app builds.

While the workspace is running, choose **Remove sign-in** in the model section
before removing Bridge. This stops the owned model route, clears its guest
credential and removes only Bridge's native-store copy. You can connect again.
GitHub CLI remains signed in; removal is not token revocation. A failed or
unknown guest cleanup keeps the saved sign-in for another explicit attempt.
An interrupted first Connect with no acknowledged connection identity, or a
completed disconnect whose host update was lost before another VM boot, may
still require maintainer help. Bridge never guesses an identity or deletes
metadata to bypass cleanup.

## Checks

From `app/`, run the [contributor checks in AGENTS.md](AGENTS.md#checks).
Build frontend assets before Rust checks because Tauri embeds them. In addition,
check the debug-only engine and local-image code:

```sh
cargo +1.98.1 test --manifest-path src-tauri/Cargo.toml --locked --test engine
cargo +1.98.1 test --manifest-path src-tauri/Cargo.toml --locked --lib --test dev_image_profile
cargo +1.98.1 clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
npm audit
npm audit --omit=dev
```

Do not apply forced major-version audit fixes blindly. CI runs frontend checks
and native Windows and Mac builds. Its Mac smoke walk captures navigation in
both system themes under `native-walk/` in the `bridge-macos-arm64` artifact.
Review those captures for visible changes; they do not exercise a real VM or
model connection.

## Platform builds

From `app/`, build a Windows x64 release executable:

```sh
npm run tauri -- build --no-bundle -- --locked
```

For an Apple silicon Mac app bundle:

```sh
npm run tauri -- build --bundles app --no-sign --no-binary-patching -- --locked
```

Preserve the bundled third-party notices when redistributing builds. Mac bundles
include them in Resources; Windows artifacts place them beside the executable.
CI artifacts expire after three days. They are unsigned test builds, not
installers or releases. Windows needs WebView2; browser-downloaded Mac builds
may be blocked by Gatekeeper.

## Where things live

- `app/src/`: React UI and frontend tests.
- `app/src-tauri/`: Rust shell, native commands and tests.
- `app/src-tauri/native/macos/`: Swift VM helper.
- `runtime/`: VM protocol, guest endpoint and Windows QEMU build.
- `images/`: Guest image build, packaging and validation.
- `docs/`: Roadmap and shared-cluster design.
- `app/src/home/navigator/`: The Navigator's instructions and the digest of Bridge and Orka docs it answers from.
  Regenerate the digest with `node scripts/navigator-digest.mjs --orka <path to an orka checkout>` after either set of
  docs changes; `--check` reports a stale digest. `size.test.ts` keeps both inside one Agent create request.
- `.github/workflows/checks.yml`: App checks, artifacts and native smoke walk.

Keep renderer input separate from native credentials and VM ownership. Preserve
Microsoft project files and third-party notices, and do not edit generated
lockfiles by hand.
