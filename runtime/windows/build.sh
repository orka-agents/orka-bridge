#!/usr/bin/env bash
set -euo pipefail
[[ $# == 2 && ${MSYSTEM:-} == UCRT64 ]] || { echo 'usage: build.sh INPUTS NEW_OUTPUT (MSYS2 UCRT64)' >&2; exit 2; }
recipe=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
inputs=$(cygpath -au "$1")
output=$(cygpath -au "$2")
[[ -d "$inputs" && ! -e "$output" ]] || { echo 'inputs must exist and output must be new' >&2; exit 1; }
export PATH=/ucrt64/bin:/usr/bin
export LC_ALL=C TZ=UTC PIP_NO_INDEX=1 PIP_DISABLE_PIP_VERSION_CHECK=1 GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_COUNT=3 GIT_CONFIG_KEY_0=core.autocrlf GIT_CONFIG_VALUE_0=false
export GIT_CONFIG_KEY_1=core.symlinks GIT_CONFIG_VALUE_1=false GIT_CONFIG_KEY_2=core.longpaths GIT_CONFIG_VALUE_2=true
py=/usr/bin/python3
native_py=/ucrt64/bin/python.exe
sources="$recipe/sources.lock.json"
packages="$recipe/packages.lock.json"
work=$(mktemp -d "$(cygpath -u "$RUNNER_TEMP")/bridge-build.XXXXXXXX")
checkouts="$work/checkouts"
mkdir "$checkouts"
value() { "$py" -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$1" "$2"; }
SOURCE_DATE_EPOCH=$(value "$sources" sourceDateEpoch)
export SOURCE_DATE_EPOCH
"$py" "$recipe/recipe.py" check-inputs --sources "$sources" --packages "$packages" --inputs "$inputs"
pacman -Q | "$py" -c 'import json,sys; print(json.dumps([dict(zip(("name","version"),x.split())) for x in sys.stdin]))' > "$work/installed.json"
"$py" "$recipe/recipe.py" check-installed --packages "$packages" --inventory "$work/installed.json"
"$py" "$recipe/recipe.py" list-git --sources "$sources" --packages "$packages" > "$work/git-inputs.tsv"
while IFS=$'\t' read -r path repository commit base; do
    destination="$checkouts/$path"
    mkdir -p "$destination"
    git -C "$destination" init --quiet
    # Enforce blob bytes even where an upstream attributes file requests a filter.
    printf '* -text -filter -ident -working-tree-encoding\n' > "$destination/.git/info/attributes"
    if [[ "$base" == - ]]; then
        git -C "$destination" fetch --quiet --depth=1 "$repository" "$commit"
    else
        git -C "$destination" fetch --quiet --depth=256 "$repository" "$commit" "$base"
        git -C "$destination" merge-base --is-ancestor "$base" "$commit"
    fi
    git -C "$destination" checkout --quiet --detach "$commit"
    [[ $(git -C "$destination" rev-parse HEAD) == "$commit" ]]
done < "$work/git-inputs.tsv"
qemu="$checkouts/qemu"
"$py" - "$sources" <<'PY' > "$work/patches.txt"
import json,sys
for row in json.load(open(sys.argv[1]))['patches']:
    print(row['path'])
PY
(
    cd "$qemu"
    while IFS= read -r patch; do
        git apply --check "$inputs/$patch"
        git apply --index "$inputs/$patch"
    done < "$work/patches.txt"
)
[[ $(git -C "$qemu" write-tree) == "$(value "$packages" patchedQemuTree)" ]]
# These fixtures execute compiler-built tests, never a guest or QEMU machine.
"$native_py" "$inputs/checks/test-socket-close.py" "$qemu/util/oslib-win32.c"
"$native_py" "$inputs/checks/test-fd-setsize.py" "$qemu"
"$native_py" "$inputs/checks/test-whpx-cpu-model.py" "$qemu"
"$py" - "$sources" "$qemu" "$packages" <<'PY'
import hashlib,json,sys
from pathlib import Path
sources=json.load(open(sys.argv[1])); root=Path(sys.argv[2]); packages=json.load(open(sys.argv[3]))
for row in sources['wraps']:
    assert hashlib.sha256((root/row['path']).read_bytes()).hexdigest()==row['sha256'],row['path']
for row in packages['pythonArchives']:
    assert hashlib.sha256((root/'python/wheels'/Path(row['path']).name).read_bytes()).hexdigest()==row['sha256'],row['path']
PY
build="$work/build"
install="$work/install"
mkdir "$build"
(
    cd "$build"
    "$qemu/configure" --prefix=/ucrt64 --target-list=x86_64-softmmu --python="$native_py" \
        --without-default-features --disable-download --enable-tools --enable-whpx --enable-slirp \
        --enable-zstd --enable-pixman --disable-tcg --disable-plugins --disable-docs --disable-rust \
        --audio-drv-list= --disable-fdt --disable-sdl --disable-sdl-image --disable-gtk --disable-vnc \
        --disable-curses --disable-opengl --disable-virglrenderer --disable-rutabaga-gfx --disable-libusb \
        --disable-usb-redir --disable-virtfs --disable-guest-agent --disable-guest-agent-msi \
        --disable-alsa --disable-coreaudio --disable-dsound --disable-jack --disable-oss --disable-pa \
        --disable-pipewire --disable-sndio --disable-spice --disable-dbus-display --disable-werror
)
meson="$build/pyvenv/bin/meson"
[[ ! -f "$meson.exe" ]] || meson="$meson.exe"
"$meson" compile -C "$build" -j "$(nproc)"
"$meson" install -C "$build" --destdir "$(cygpath -m "$install")"
"$py" "$recipe/recipe.py" stage --sources "$sources" --packages "$packages" \
    --checkouts "$checkouts" --inputs "$inputs" --build "$build" --install "$install" \
    --runtime "$work/runtime" --source "$work/source" --evidence "$work/build-report.json" --meson "$meson"
"$py" "$recipe/recipe.py" package --sources "$sources" --packages "$packages" \
    --runtime "$work/runtime" --source "$work/source" --output "$output" --evidence "$work/build-report.json"
"$py" "$recipe/verify.py" --output "$output" --sources "$sources" --packages "$packages"
