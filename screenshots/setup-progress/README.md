# Setup progress

Captured from source `0102a673584ebaca541c8688af235dbdb5c078a7` in the native Windows and Mac builds from [App run 43270037954](https://github.com/orka-agents/orka-bridge/actions/runs/43270037954).

Windows images use the compiled frontend in the app's WebView2 with synthetic IPC. Light and dark use renderer media emulation. Compact captures use an actual native-window resize: client 540×900, WebView 540×900 CSS pixels, device-pixel ratio 1. “History” shows the same compact page scrolled down, not another layout.

These are UI fixtures, not proof of a real VM, download, credential connection or inference. Times and byte counts are synthetic. Unsupported-build fixtures deliberately report no model capability and offer no early sign-in. The current Mac image remains model-locked; the separate Mac model fixture below supplies synthetic ready/available replies solely to exercise the UI.

## Installation and startup

| State | Light | Dark | Compact light | Compact dark | Compact history light | Compact history dark |
|---|---|---|---|---|---|---|
| Installing | [View](windows/light-wide-setup-transfer.png) | [View](windows/dark-wide-setup-transfer.png) | [View](windows/light-compact-setup-transfer.png) | [View](windows/dark-compact-setup-transfer.png) | [View](windows/light-compact-setup-transfer-end.png) | [View](windows/dark-compact-setup-transfer-end.png) |
| Model choice queued | [View](windows/light-wide-setup-transfer-queued.png) | [View](windows/dark-wide-setup-transfer-queued.png) | [View](windows/light-compact-setup-transfer-queued.png) | [View](windows/dark-compact-setup-transfer-queued.png) | [View](windows/light-compact-setup-transfer-queued-end.png) | [View](windows/dark-compact-setup-transfer-queued-end.png) |
| Starting | [View](windows/light-wide-setup-startup.png) | [View](windows/dark-wide-setup-startup.png) | [View](windows/light-compact-setup-startup.png) | [View](windows/dark-compact-setup-startup.png) | [View](windows/light-compact-setup-startup-end.png) | [View](windows/dark-compact-setup-startup-end.png) |
| Startup beyond estimate | [View](windows/light-wide-setup-startup-slow.png) | [View](windows/dark-wide-setup-startup-slow.png) | [View](windows/light-compact-setup-startup-slow.png) | [View](windows/dark-compact-setup-startup-slow.png) | [View](windows/light-compact-setup-startup-slow-end.png) | [View](windows/dark-compact-setup-startup-slow-end.png) |
| Ready for model choice | [View](windows/light-wide-setup-ready.png) | [View](windows/dark-wide-setup-ready.png) | [View](windows/light-compact-setup-ready.png) | [View](windows/dark-compact-setup-ready.png) | [View](windows/light-compact-setup-ready-end.png) | [View](windows/dark-compact-setup-ready-end.png) |
| Observation unavailable | [View](windows/light-wide-setup-unavailable.png) | [View](windows/dark-wide-setup-unavailable.png) | [View](windows/light-compact-setup-unavailable.png) | [View](windows/dark-compact-setup-unavailable.png) | [View](windows/light-compact-setup-unavailable-end.png) | [View](windows/dark-compact-setup-unavailable-end.png) |
| No native model support | [View](windows/light-wide-setup-transfer-unsupported.png) | [View](windows/dark-wide-setup-transfer-unsupported.png) | [View](windows/light-compact-setup-transfer-unsupported.png) | [View](windows/dark-compact-setup-transfer-unsupported.png) | [View](windows/light-compact-setup-transfer-unsupported-end.png) | [View](windows/dark-compact-setup-transfer-unsupported-end.png) |

## Model and shell states on Mac

The model fixture embeds the same frontend in a separate native app with synthetic IPC, no VM and no credentials. It demonstrates rendering and control transitions, not model support in the current Mac guest.

| State | Light | Dark |
|---|---|---|
| Before connection | [View](macos-model/light/before-connect.png) | [View](macos-model/dark/before-connect.png) |
| Connected | [View](macos-model/light/connected.png) | [View](macos-model/dark/connected.png) |
| Removal failed | [View](macos-model/light/remove-failed.png) | [View](macos-model/dark/remove-failed.png) |
| Sign-in removed | [View](macos-model/light/removed.png) | [View](macos-model/dark/removed.png) |
| Reconnected | [View](macos-model/light/reconnected.png) | [View](macos-model/dark/reconnected.png) |
| Native Clusters screen, retained choice | [View](macos-shell/light/clusters-pointer.png) | [View](macos-shell/dark/clusters-pointer.png) |

Initial native captures differ because the walk retains its local choice between themes: [light first launch, no place selected](macos-shell/light/initial.png); [dark retained local selection](macos-shell/dark/initial.png).
