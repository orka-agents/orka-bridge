# Computer requirements screenshots

Bridge source: `8d1e18cf465184cb9f873e0a1d1f080bb95cd22f`.

## Renderer fixtures

The images in `fixtures/` render the real app with mocked native command responses. They are not native platform verification and do not demonstrate a VM launch. Each scenario is shown in light and dark themes:

- `ready`: virtualization available, 8 GiB installed RAM.
- `memory`: 4 GiB installed RAM, with the installed-memory explanation opened.
- `virtualization`: firmware virtualization off or unavailable.
- `unknown`: both observations unreadable; actual launch remains authoritative.
- `mac`: platform virtualization unsupported, with Mac-specific guidance opened.

No local workspace was selected during these captures. Native runner screenshots, when available, are stored separately under `native/` with their source revision.
