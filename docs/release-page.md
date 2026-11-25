Bridge runs an AI agent on your own computer and checks its answer with a test. It sets up a small Linux VM with [Orka](https://github.com/orka-agents/orka) inside, so there's nothing else to install. This is a preview for Apple-silicon Macs (macOS 13 or later) and Windows 10 or 11 on x64.

**Download one file:** `bridge-macos-arm64.tar.gz` for a Mac, or `bridge-windows-x64.zip` for Windows. You don't need `bridge-windows-runtime-source.zip`: it's the source for the VM engine bundled in the Windows app, published for its licence. Bridge downloads its VM image itself.

**Before you start**

- GitHub CLI, signed in to an account with Copilot (`gh auth login --hostname github.com`).
- 16 GB of memory; the VM uses 6 GB.
- About 75 GB free disk on a Mac, or 60 GB on Windows.

**First open.** The app isn't signed by a known developer, so your computer asks once:

- Mac: macOS says it can't verify Bridge. Open System Settings > Privacy & Security and choose Open Anyway. If your account isn't an administrator, macOS asks for an administrator's name and password: ask whoever manages your Mac, or your IT team.
- Windows: SmartScreen says it protected your PC. Choose More info, then Run anyway.

If macOS says Bridge is damaged, or offers no Open Anyway, stop and [open an issue](https://github.com/orka-agents/orka-bridge/issues/new/choose). Don't turn off Gatekeeper.

Setup takes about 15 minutes, most of it a 2 GB download. Follow the [setup guide](https://github.com/orka-agents/orka-bridge/blob/main/docs/preview.md).

**New in this release**

- A Mac download opens: earlier Mac releases showed "damaged" in a browser download.
- Clearer setup steps and failed-check messages.
- Overview lists your agents in one place, and shows the workspace's health.
- Bridge uses the Orka website's look.
