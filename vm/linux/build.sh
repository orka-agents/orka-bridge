#!/bin/bash
# Builds the Bridge Linux VM image: a Debian VM with Docker, kind, a kind cluster running
# Orka (installed from its release chart, images pinned by digest and pre-pulled) and kmx.
#
# Usage: vm/linux/build.sh [OUT_DIR]
# Needs qemu-system-x86_64, qemu-img, cloud-localds, curl, jq and a usable /dev/kvm.
# WORK_DIR holds the base image cache and build disk; without it a temporary directory is
# used and removed afterwards.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
out=${1:-$repo/vm/out}
if [ -n "${WORK_DIR:-}" ]; then
  work=$WORK_DIR
else
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
fi
build_cpus=${BUILD_CPUS:-8}
build_memory=${BUILD_MEMORY:-8192}
disk_size=${DISK_SIZE:-40G}
# Optional: also pre-pull the coding-agent runtime images (several GB).
prepull_acp=${PREPULL_ACP:-0}
# The address install.sh downloads from unless BRIDGE_VM_URL is set when it runs, for
# example a local server. Empty means install.sh always needs BRIDGE_VM_URL.
base_url=${BASE_URL:-}

# shellcheck source=vm/versions.env
. "$repo/vm/versions.env"

log() { printf '==> %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

for tool in qemu-system-x86_64 qemu-img cloud-localds curl jq sha256sum sha512sum; do
  command -v "$tool" >/dev/null || die "$tool is required"
done
{ [ -r /dev/kvm ] && [ -w /dev/kvm ]; } || die "/dev/kvm is not usable by $(id -un)"
mkdir -p "$out" "$work"

fetch() { # url destination
  curl -fsSL --retry 5 --retry-all-errors -o "$2.part" "$1"
  mv "$2.part" "$2"
}

base_name=$(basename "$DEBIAN_IMAGE_URL" .qcow2)
base=$work/$base_name.qcow2
if ! printf '%s  %s\n' "$DEBIAN_IMAGE_SHA512" "$base" | sha512sum -c --quiet - 2>/dev/null; then
  log "downloading $base_name"
  fetch "$DEBIAN_IMAGE_URL" "$base"
  printf '%s  %s\n' "$DEBIAN_IMAGE_SHA512" "$base" | sha512sum -c --quiet - ||
    die "checksum mismatch for $DEBIAN_IMAGE_URL"
fi

log "reading Orka $ORKA_VERSION release metadata"
candidate=$work/orka-$ORKA_VERSION-candidate.json
fetch "https://github.com/orka-agents/orka/releases/download/$ORKA_VERSION/candidate.json" "$candidate"
printf '%s  %s\n' "$ORKA_CANDIDATE_SHA256" "$candidate" | sha256sum -c --quiet - ||
  die "checksum mismatch for Orka $ORKA_VERSION candidate.json"
[ "$(jq -r .version "$candidate")" = "$ORKA_VERSION" ] || die "candidate.json is not for $ORKA_VERSION"
image() { jq -er --arg k "$1" '.images[$k]' "$candidate"; }
digest() { image "$1" | sed -n 's/^[^@]*@\(sha256:[0-9a-f]\{64\}\)$/\1/p'; }
prepull="$(image ai-worker) $(image general-worker)"
if [ "$prepull_acp" = 1 ]; then
  prepull="$prepull $(image acp-codex-runtime) $(image acp-claude-runtime)"
  prepull="$prepull $(image acp-copilot-runtime) $(image acp-opencode-runtime)"
fi

seed=$work/seed
rm -rf "$seed"
mkdir -p "$seed"
cat >"$seed/build.env" <<EOF
KIND_VERSION=$KIND_VERSION
KIND_LINUX_AMD64_SHA256=$KIND_LINUX_AMD64_SHA256
KIND_NODE_IMAGE=$KIND_NODE_IMAGE
KUBECTL_VERSION=$KUBECTL_VERSION
KUBECTL_LINUX_AMD64_SHA256=$KUBECTL_LINUX_AMD64_SHA256
HELM_VERSION=$HELM_VERSION
HELM_LINUX_AMD64_SHA256=$HELM_LINUX_AMD64_SHA256
KMX_VERSION=$KMX_VERSION
KMX_LINUX_AMD64_SHA256=$KMX_LINUX_AMD64_SHA256
ORKA_VERSION=$ORKA_VERSION
ORKA_CHART_FILE=$(jq -er .chart.file "$candidate")
ORKA_CHART_SHA256=$(jq -er .chart.sha256 "$candidate")
ORKA_CONTROLLER_DIGEST=$(digest controller)
ORKA_AI_WORKER_DIGEST=$(digest ai-worker)
ORKA_GENERAL_WORKER_DIGEST=$(digest general-worker)
ORKA_PUBLISHER_DIGEST=$(digest workspace-publisher)
ORKA_CODEX_IMAGE=$(image acp-codex-runtime)
ORKA_CLAUDE_IMAGE=$(image acp-claude-runtime)
ORKA_COPILOT_IMAGE=$(image acp-copilot-runtime)
ORKA_OPENCODE_IMAGE=$(image acp-opencode-runtime)
ORKA_PREPULL_IMAGES="$prepull"
EOF
grep -q '=$' "$seed/build.env" && die "missing value in build inputs: $(grep '=$' "$seed/build.env")"
cat >"$seed/image.env" <<EOF
ORKA_VERSION=$ORKA_VERSION
KMX_VERSION=$KMX_VERSION
EOF

b64() { base64 -w0 "$1"; }
cat >"$seed/user-data" <<EOF
#cloud-config
write_files:
- {path: /opt/bridge/build.env, permissions: '0600', encoding: b64, content: $(b64 "$seed/build.env")}
- {path: /opt/bridge/image.env, permissions: '0644', encoding: b64, content: $(b64 "$seed/image.env")}
- {path: /opt/bridge/provision.sh, permissions: '0755', encoding: b64, content: $(b64 "$here/guest/provision.sh")}
- {path: /opt/bridge/bridge-vm-ready, permissions: '0755', encoding: b64, content: $(b64 "$here/guest/bridge-vm-ready")}
- {path: /opt/bridge/bridge-vm-ready.service, permissions: '0644', encoding: b64, content: $(b64 "$here/guest/bridge-vm-ready.service")}
runcmd:
- [sh, -c, 'exec >/dev/virtio-ports/ai.orka.bridge.build 2>&1; /opt/bridge/provision.sh; echo "BRIDGE-PROVISION-EXIT \$?"; sync; poweroff']
EOF
printf 'instance-id: orka-bridge-build\nlocal-hostname: orka-bridge\n' >"$seed/meta-data"
cloud-localds "$work/seed.iso" "$seed/user-data" "$seed/meta-data"

disk=$work/build.qcow2
# The seed carries the provisioning inputs (versions, digests, guest scripts); the base
# image and the disk size shape the disk itself.
inputs=$({
  cat "$seed/user-data"
  printf 'base %s\ndisk_size %s\n' "$DEBIAN_IMAGE_SHA512" "$disk_size"
} | sha256sum | cut -d' ' -f1)
# REUSE_BUILD=1 repackages a disk that already provisioned successfully from the same inputs.
if [ "${REUSE_BUILD:-0}" = 1 ] && [ -f "$disk" ] &&
  [ "$(cat "$work/provisioned-inputs" 2>/dev/null)" = "$inputs" ] &&
  grep -q '^BRIDGE-PROVISION-EXIT 0' "$work/provision.log" 2>/dev/null; then
  log "reusing provisioned $disk"
else
rm -f "$disk" "$work/console.log" "$work/provision.log" "$work/provisioned-inputs"
qemu-img convert -O qcow2 "$base" "$disk"
qemu-img resize -q "$disk" "$disk_size"

log "provisioning (log: $work/provision.log)"
start=$SECONDS
timeout 60m qemu-system-x86_64 \
  -name orka-bridge-build -machine q35,accel=kvm -cpu host \
  -smp "$build_cpus" -m "$build_memory" \
  -drive "file=$disk,if=virtio,discard=unmap,detect-zeroes=unmap" \
  -drive "file=$work/seed.iso,if=virtio,media=cdrom,readonly=on" \
  -nic user,model=virtio-net-pci -device virtio-rng-pci \
  -device virtio-serial-pci -chardev "file,id=build,path=$work/provision.log" \
  -device virtserialport,chardev=build,name=ai.orka.bridge.build \
  -display none -serial "file:$work/console.log" -no-reboot ||
  die "the build VM did not finish; see $work/console.log"
grep -q '^BRIDGE-PROVISION-EXIT 0' "$work/provision.log" ||
  die "provisioning failed; see $work/provision.log"
printf '%s\n' "$inputs" >"$work/provisioned-inputs"
log "provisioned in $((SECONDS - start))s"
fi

name=orka-bridge-vm-orka-$ORKA_VERSION-linux-amd64.qcow2
manifest="manifest-linux-amd64.json"
log "compressing $name"
rm -f "$out/$name"
qemu-img convert -c -O qcow2 -o compression_type=zstd "$disk" "$out/$name"
install -m 0755 "$here/bridge-vm" "$out/bridge-vm"
sed "s#@BRIDGE_VM_URL@#$base_url#" "$here/install.sh" >"$out/install.sh"

size=$(stat -c %s "$out/$name")
virtual=$(qemu-img info --output=json "$out/$name" | jq '.["virtual-size"]')
data=$(qemu-img measure --output=json -O qcow2 "$disk" | jq .required)
revision=$(git -C "$repo" rev-parse HEAD 2>/dev/null || echo unknown)
jq -n \
  --arg orka "$ORKA_VERSION" --arg kmx "$KMX_VERSION" --arg kind "$KIND_VERSION" \
  --arg kubernetes "$(printf '%s' "$KIND_NODE_IMAGE" | sed -n 's/^[^:]*:\(v[^@]*\)@.*/\1/p')" \
  --arg base "$base_name" --arg file "$name" --arg sha "$(sha256sum "$out/$name" | cut -d' ' -f1)" \
  --argjson size "$size" --argjson virtual "$virtual" --argjson data "$data" \
  --arg cli_sha "$(sha256sum "$out/bridge-vm" | cut -d' ' -f1)" \
  --arg created "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg revision "$revision" \
  --arg notes "kmx $KMX_VERSION adopts only an Orka $ORKA_VERSION install; a newer Orka needs a kmx release that targets it." \
  '{schemaVersion: 1, platform: "linux-amd64",
    orkaVersion: $orka, kmxVersion: $kmx, kindVersion: $kind, kubernetesVersion: $kubernetes,
    baseImage: $base, imageFile: $file, imageFormat: "qcow2", imageCompression: "zstd",
    imageSha256: $sha, imageSize: $size, imageDataSize: $data, imageVirtualSize: $virtual,
    cliFile: "bridge-vm", cliSha256: $cli_sha, created: $created, sourceRevision: $revision,
    notes: $notes}' \
  >"$out/$manifest"
(cd "$out" && sha256sum "$name" bridge-vm install.sh "$manifest" >SHA256SUMS)
log "image $out/$name: $size bytes compressed, $data bytes uncompressed"
cat "$out/$manifest"
