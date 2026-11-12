# Computer requirements screenshots

Bridge source: `8d500cad54f6f4481588d51f5b9f7169f2ba670a`.

## Renderer fixtures

The images in `fixtures/` render the real app with mocked native command responses. They are not native platform verification and do not demonstrate a VM launch. Each scenario is shown in light and dark themes:

- `ready`: virtualization available, 8 GiB installed RAM.
- `memory`: 4 GiB installed RAM, with the installed-memory explanation opened.
- `virtualization`: firmware virtualization off or unavailable.
- `unknown`: both observations unreadable; actual launch remains authoritative.
- `mac`: platform virtualization unsupported, with Mac-specific guidance opened.

No local workspace was selected during these captures. The fixtures do not substitute for native qualification.

## Native observations

`native/` contains actual branch-head Windows x64 and Apple-silicon Mac captures from App CI run [43131247122](https://github.com/orka-agents/orka-bridge/actions/runs/43131247122), at the source head above; both platform jobs passed. The native scripts first locate the Computer requirements region and its settled recheck button, then capture without selecting a workspace.

Windows reports Available and 16 GiB installed; this hosted Mac reports Couldn't check and 7 GiB installed. No values are forced to Ready. The narrow Mac window scrolls; Continue is partly below the fold. These are empty-source development-shell walks, not a VM launch or real credential/cluster qualification. The isolated Mac debug capture was followed by a successful separate release first-run walk.
