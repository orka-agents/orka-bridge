# Model connection with shared foundation styles

Native macOS light/dark captures from PR #135 at `7d0a3a2aa73fa3b42540c78abf62efccd37cb154`, after merging #138. [App run](https://github.com/orka-agents/orka-bridge/actions/runs/43227539552).

These show the compiled frontend in the separate synthetic native model fixture. They exercise connect, failed removal, successful removal and reconnect. They do not use a VM, OS credential store, GitHub CLI, or real inference. The pending-removal indicator is covered by the App regression test; these captures show settled states.

| State | Light | Dark |
|---|---|---|
| Before connecting | [Light](light/before-connect.png) | [Dark](dark/before-connect.png) |
| Connected | [Light](light/connected.png) | [Dark](dark/connected.png) |
| Removal failed | [Light](light/remove-failed.png) | [Dark](dark/remove-failed.png) |
| Removed | [Light](light/removed.png) | [Dark](dark/removed.png) |
| Reconnected | [Light](light/reconnected.png) | [Dark](dark/reconnected.png) |

The other foundation screenshots retain their original reviewed captures from before the main merge.
