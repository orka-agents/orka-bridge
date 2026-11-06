#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
helper="${1:?pass the built Bridge.app path}/Contents/Resources/native/BridgeVM"
if [[ ! -x "$helper" ]]; then
    echo 'missing executable bundled Apple helper' >&2
    exit 1
fi
python3 - "$helper" <<'PY'
import os
import stat
import sys
mode = stat.S_IMODE(os.stat(sys.argv[1]).st_mode)
assert mode == 0o755, f'helper mode must be 0755, got {mode:04o}'
PY
test "$(lipo -archs "$helper")" = arm64
codesign --verify --strict "$helper"
entitlements=$(mktemp "${TMPDIR:-/tmp}/bridge-helper-entitlements.XXXXXX")
trap 'rm -f "$entitlements"' EXIT
codesign --display --entitlements :- "$helper" > "$entitlements"
python3 - "$entitlements" <<'PY'
import plistlib
import sys
with open(sys.argv[1], 'rb') as source:
    actual = plistlib.load(source)
assert actual == {'com.apple.security.virtualization': True}, 'unexpected helper entitlement grants'
PY
cmp "$root/app/src-tauri/gen/apple-helper/BridgeVM" "$helper"
echo 'Bundled Apple helper: executable, arm64, signature, entitlement and bytes verified'
