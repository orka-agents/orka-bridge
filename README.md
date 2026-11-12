# Workspace and AI recovery screenshots

Source: `52ce25137c40ab6d78332d7c88283d3f749ab73f`.

Captured from the built Windows app's embedded WebView2 in [passing CI](https://github.com/orka-agents/orka-bridge/actions/runs/43145020428). These are **injected renderer states**, not native IPC, VM, credential or download-path verification. The exact compiled frontend runs in an isolated debugger world. Light/dark media emulation does not change system preferences.

The download category is reserved: current native startup prepares development image copies and cannot reach this download screen. Native backend tests and the separate, non-injected Windows/Mac lifecycle walks passed at the same source commit; those checks are not replaced by these fixtures.

| State | Light | Dark |
| --- | --- | --- |
| Startup permission denied | [Light](light-permission.png) | [Dark](dark-permission.png) |
| Local workspace preparation failed | [Light](light-prepare.png) | [Dark](dark-prepare.png) |
| Unknown startup failure | [Light](light-unknown.png) | [Dark](dark-unknown.png) |
| Reserved download failure | [Light](light-download-reserved.png) | [Dark](dark-download-reserved.png) |
| GitHub CLI unavailable | [Light](light-cli.png) | [Dark](dark-cli.png) |
| Sign-in check failed | [Light](light-sign-in.png) | [Dark](dark-sign-in.png) |
| Native credential-store operation failed | [Light](light-model-store.png) | [Dark](dark-model-store.png) |
