# Bridge logo files

The mark is an orca fin in a ring above three waves, in ink and secondary ink
from the app's palette. The SVG files are the source; the PNG files are exports.

| File | Use |
| --- | --- |
| `bridge-mark.svg` | README header and other large uses on light backgrounds |
| `bridge-mark-dark.svg` | The same on dark backgrounds |
| `bridge-mark-small.svg` | 32 px and smaller on light backgrounds: heavier ring, two waves |
| `bridge-mark-small-dark.svg` | The same on dark backgrounds |
| `bridge-icon.svg` | App icon source: the mark on a white disc |
| `bridge-icon-small.svg` | App icon at 32 px and smaller |
| `bridge-icon-16.png`, `bridge-icon-32.png` | Exports of `bridge-icon-small.svg` |
| `bridge-icon-128.png`, `bridge-icon-512.png` | Exports of `bridge-icon.svg` |
| `bridge-logo.png` | Earlier mascot artwork, not used by the README or the app |

The app's own mark, `app/src/assets/orka-mark.svg`, is a copy of
`bridge-mark-small.svg`; dark mode inverts it with `--mark-filter`.

To regenerate the native icons from `app/`, run
`npm run tauri -- icon ../assets/bridge-icon.svg -o <temp dir>` and copy
`icon.png`, `icon.ico` and `icon.icns` into `src-tauri/icons/`. The PNG exports
use the same command with `-p 16,32` on `bridge-icon-small.svg` and
`-p 128,512` on `bridge-icon.svg`.
