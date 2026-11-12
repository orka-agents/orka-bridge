# Cluster recovery native screenshots

Source: [`c5d0e6bfddeefd212f89281fdb826efc78714dd2`](https://github.com/orka-agents/orka-bridge/commit/c5d0e6bfddeefd212f89281fdb826efc78714dd2), [PR #140](https://github.com/orka-agents/orka-bridge/pull/140).

Captured by the passing [App workflow](https://github.com/orka-agents/orka-bridge/actions/runs/43215667417). No real clusters, credentials, model or VM were used.

## Windows WebView2 renderer fixtures

These run the final embedded app entry in the native WebView2 with isolated-world synthetic IPC. They demonstrate presentation, not native IPC or real connection behavior. Light/dark media emulation affects only the test WebView, not host preferences.

| State | Light | Dark |
|---|---|---|
| Empty kubeconfig | ![](windows/light-cluster-empty.png) | ![](windows/dark-cluster-empty.png) |
| Configuration error | ![](windows/light-cluster-configuration.png) | ![](windows/dark-cluster-configuration.png) |
| Provider permission denied | ![](windows/light-cluster-permission.png) | ![](windows/dark-cluster-permission.png) |
| Unknown identity check | ![](windows/light-cluster-unknown.png) | ![](windows/dark-cluster-unknown.png) |
| Reload pending | ![](windows/light-cluster-reload-pending.png) | ![](windows/dark-cluster-reload-pending.png) |
| Selection cleared after reload | ![](windows/light-cluster-reloaded.png) | ![](windows/dark-cluster-reloaded.png) |
| Clipboard fallback | ![](windows/light-model-copy.png) | ![](windows/dark-model-copy.png) |

## macOS native backend with loopback fixtures

These use the native app and backend with a loopback-only fake Kubernetes/Orka server. The walk edits its synthetic kubeconfig, reloads it and restores it. Screenshots follow the native system theme.

| State | Light | Dark |
|---|---|---|
| Orka identity permission denied | ![](macos/light/cluster-denied.png) | ![](macos/dark/cluster-denied.png) |
| Failed reload | ![](macos/light/cluster-reload-failed.png) | ![](macos/dark/cluster-reload-failed.png) |
| Selection cleared | ![](macos/light/cluster-reload-cleared.png) | ![](macos/dark/cluster-reload-cleared.png) |
| Corrected context reread | ![](macos/light/cluster-reload-options.png) | ![](macos/dark/cluster-reload-options.png) |
