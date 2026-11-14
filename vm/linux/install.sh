#!/usr/bin/env bash
# Installs the Bridge Linux VM: Orka on kind, ready to use, in a QEMU/KVM virtual machine.
#
#   curl -fsSL <url>/install.sh | bash
#
# Installs the bridge-vm command in ~/.local/bin (BRIDGE_VM_BIN), downloads and verifies
# the VM image, boots it and waits until Orka is ready. BRIDGE_VM_URL overrides the
# address the image is downloaded from.
set -euo pipefail

url=${BRIDGE_VM_URL:-@BRIDGE_VM_URL@}
url=${url%/}
bin=${BRIDGE_VM_BIN:-$HOME/.local/bin}
platform=linux-amd64

die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

case $url in
  '' | @*) die "set BRIDGE_VM_URL to the address that serves the image" ;;
esac
[ "$(uname -s)" = Linux ] || die "this installer is for Linux"
[ "$(uname -m)" = x86_64 ] || die "this installer is for x86_64 Linux"
for tool in curl sha256sum sed; do
  command -v "$tool" >/dev/null || die "$tool is required"
done

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
get() { curl -fsSL --retry 3 --retry-connrefused -o "$1" "$2"; }

get "$tmp/manifest.json" "$url/manifest-$platform.json" ||
  die "cannot download $url/manifest-$platform.json"
field() { sed -n "s/^ *\"$1\": \"\([^\"]*\)\",\{0,1\}\$/\1/p" "$tmp/manifest.json" | head -n1; }
cli=$(field cliFile)
cli_sha=$(field cliSha256)
[[ $cli =~ ^[A-Za-z0-9._-]+$ ]] || die "invalid command name in the manifest"
[[ $cli_sha =~ ^[0-9a-f]{64}$ ]] || die "invalid command checksum in the manifest"

get "$tmp/bridge-vm" "$url/$cli" || die "cannot download $url/$cli"
printf '%s  %s\n' "$cli_sha" "$tmp/bridge-vm" | sha256sum -c --quiet - 2>/dev/null ||
  die "checksum mismatch for $url/$cli"
mkdir -p "$bin"
install -m 0755 "$tmp/bridge-vm" "$bin/bridge-vm"
printf 'Installed %s\n' "$bin/bridge-vm"
case :$PATH: in
  *:"$bin":*) ;;
  *) printf 'Add %s to PATH to run bridge-vm directly.\n' "$bin" ;;
esac

# exec replaces this shell, so the EXIT trap would not run.
rm -rf "$tmp"
trap - EXIT
BRIDGE_VM_BIN=$bin exec "$bin/bridge-vm" install "$url"
