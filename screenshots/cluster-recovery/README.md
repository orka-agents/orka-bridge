# Cluster and model recovery

The six Mac reload captures (failed, selection cleared, corrected contexts; light/dark) show PR #140 at `8e7fad3c83c810cc4440b279b0095fcfb031fa57`, after merging #135. [App run](https://github.com/orka-agents/orka-bridge/actions/runs/43242533862). They show the shared styling at a 540×900 native content size.

The other 26 images retain the previously reviewed captures at `13fe24c40a5f0e3f6633bf84d46381166e3efbda`, before #135's styling. They are not current-head appearance evidence.

Mac: the reload captures above and the [earlier App run](https://github.com/orka-agents/orka-bridge/actions/runs/43234995435) exercise native IPC against a loopback-only fake Kubernetes/Orka server and a synthetic kubeconfig. Model captures use the separate compiled-frontend fixture with synthetic IPC.

Windows: [App run](https://github.com/orka-agents/orka-bridge/actions/runs/43234980242). Renderer fixtures run the compiled frontend inside native WebView2 with synthetic IPC and clipboard failure. They are UI evidence, not native credential or cluster proof.

No real credentials, cluster, VM or inference were used.

## Windows

| State | Light | Dark |
|---|---|---|
| Empty kubeconfig | [Light](windows/light-cluster-empty.png) | [Dark](windows/dark-cluster-empty.png) |
| Configuration failure | [Light](windows/light-cluster-configuration.png) | [Dark](windows/dark-cluster-configuration.png) |
| Permission failure | [Light](windows/light-cluster-permission.png) | [Dark](windows/dark-cluster-permission.png) |
| Unknown failure | [Light](windows/light-cluster-unknown.png) | [Dark](windows/dark-cluster-unknown.png) |
| Reload pending | [Light](windows/light-cluster-reload-pending.png) | [Dark](windows/dark-cluster-reload-pending.png) |
| Reloaded, selection cleared | [Light](windows/light-cluster-reloaded.png) | [Dark](windows/dark-cluster-reloaded.png) |
| Model clipboard fallback | [Light](windows/light-model-copy.png) | [Dark](windows/dark-model-copy.png) |

## Mac cluster backend

| State | Light | Dark |
|---|---|---|
| Denied | [Light](macos/light/cluster-denied.png) | [Dark](macos/dark/cluster-denied.png) |
| Reload failed | [Light](macos/light/cluster-reload-failed.png) | [Dark](macos/dark/cluster-reload-failed.png) |
| Selection cleared | [Light](macos/light/cluster-reload-cleared.png) | [Dark](macos/dark/cluster-reload-cleared.png) |
| Corrected contexts | [Light](macos/light/cluster-reload-options.png) | [Dark](macos/dark/cluster-reload-options.png) |

## Mac model renderer

| State | Light | Dark |
|---|---|---|
| Before connecting | [Light](model/light/before-connect.png) | [Dark](model/dark/before-connect.png) |
| Connected | [Light](model/light/connected.png) | [Dark](model/dark/connected.png) |
| Removal failed | [Light](model/light/remove-failed.png) | [Dark](model/dark/remove-failed.png) |
| Removed | [Light](model/light/removed.png) | [Dark](model/dark/removed.png) |
| Reconnected | [Light](model/light/reconnected.png) | [Dark](model/dark/reconnected.png) |
