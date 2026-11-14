#!/bin/bash
# Tests install.sh and bridge-vm against a local HTTP server, with a stand-in for QEMU that
# reports the guest's status lines. Needs python3, jq, curl and a usable /dev/kvm (only for
# bridge-vm's preflight; nothing boots).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
server_pid=
cleanup() {
  [ -z "$server_pid" ] || kill "$server_pid" 2>/dev/null || true
  for pidfile in "$tmp"/home*/instance/qemu.pid; do
    if [ -f "$pidfile" ]; then kill -KILL "$(cat "$pidfile")" 2>/dev/null || true; fi
  done
  rm -rf "$tmp"
}
trap cleanup EXIT

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
pass() { printf 'ok - %s\n' "$*"; }
free_port() { python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'; }

# A stand-in for qemu-system-x86_64: it serves /readyz on the forwarded Orka port, writes the
# pid file and reports the guest states that bridge-vm-ready would send.
mkdir -p "$tmp/fakebin"
cat >"$tmp/fakebin/qemu-system-x86_64" <<'EOF'
#!/bin/bash
set -euo pipefail
while [ $# -gt 0 ]; do
  case $1 in
    -pidfile) pidfile=$2; shift ;;
    -chardev) status=${2##*path=}; shift ;;
    -nic) orka_port=$(printf '%s' "$2" | sed -n 's/.*127.0.0.1:\([0-9]*\)-:8080.*/\1/p'); shift ;;
  esac
  shift
done
web=$(dirname "$pidfile")/orka-bridge-vm-web
mkdir -p "$web"
echo ok >"$web/readyz"
if [ "${FAKE_GUEST:-}" = stubborn ]; then
  # A QEMU that ignores SIGTERM.
  (trap '' TERM; exec python3 -I -m http.server "$orka_port" --bind 127.0.0.1 --directory "$web") >/dev/null 2>&1 &
else
  python3 -I -m http.server "$orka_port" --bind 127.0.0.1 --directory "$web" >/dev/null 2>&1 &
fi
echo $! >"$pidfile"
for _ in $(seq 50); do curl -fsS -o /dev/null "http://127.0.0.1:$orka_port/readyz" 2>/dev/null && break; sleep 0.1; done
{
  echo '{"state":"booted","uptime":1.0}'
  echo '{"state":"kubernetes-ready","uptime":2.0}'
} >"$status"
kubeconfig=$(printf 'clusters:\n- cluster:\n    server: https://127.0.0.1:6443\n  name: kind-kaimahi-p1\ncurrent-context: kind-kaimahi-p1\n' | base64 -w0)
ready="{\"state\":\"ready\",\"orka\":\"v0.2.0\",\"kmx\":\"v0.9.0-rc.6\",\"uptime\":4.0,\"kubeconfig\":\"$kubeconfig\"}"
case ${FAKE_GUEST:-ready} in
  failed) echo '{"state":"failed","reason":"the Orka controller did not become ready","uptime":3.0}' >>"$status" ;;
  # The ready line arrives in two writes, with the host polling in between.
  partial) printf '%s' "${ready:0:40}" >>"$status"; (sleep 3; printf '%s\n' "${ready:40}" >>"$status") & ;;
  *) echo "$ready" >>"$status" ;;
esac
EOF
cat >"$tmp/fakebin/qemu-img" <<'EOF'
#!/bin/bash
[ "$1" = create ] || exit 1
for last; do :; done
printf '%s\n' "$@" >"$last.args"
touch "$last"
EOF
chmod +x "$tmp/fakebin/"*
export PATH="$tmp/fakebin:$PATH"

# A release directory as build.sh lays it out.
port=$(free_port)
url=http://127.0.0.1:$port
publish() { # directory image-sha256-override
  local dir=$1 sha
  mkdir -p "$dir"
  head -c 300000 /dev/urandom >"$dir/orka-bridge-vm-orka-v0.2.0-linux-amd64.qcow2"
  install -m 0755 "$here/bridge-vm" "$dir/bridge-vm"
  sed "s#@BRIDGE_VM_URL@#$url/$(basename "$dir")#" "$here/install.sh" >"$dir/install.sh"
  sha=${2:-$(sha256sum "$dir/orka-bridge-vm-orka-v0.2.0-linux-amd64.qcow2" | cut -d' ' -f1)}
  jq -n --arg sha "$sha" --arg cli "$(sha256sum "$dir/bridge-vm" | cut -d' ' -f1)" \
    '{schemaVersion: 1, platform: "linux-amd64", orkaVersion: "v0.2.0", kmxVersion: "v0.9.0-rc.6",
      imageFile: "orka-bridge-vm-orka-v0.2.0-linux-amd64.qcow2", imageFormat: "qcow2",
      imageSha256: $sha, imageSize: 300000, cliFile: "bridge-vm", cliSha256: $cli}' \
    >"$dir/manifest-linux-amd64.json"
}
publish "$tmp/www/good"
publish "$tmp/www/bad-image" "$(printf 0 | sha256sum | cut -d' ' -f1)"
publish "$tmp/www/bad-cli"
echo 'echo tampered' >>"$tmp/www/bad-cli/bridge-vm"
publish "$tmp/www/failed"
publish "$tmp/www/partial"
publish "$tmp/www/resume"
publish "$tmp/www/no-url"
sed 's#@BRIDGE_VM_URL@##' "$here/install.sh" >"$tmp/www/no-url/install.sh"
python3 -I -m http.server "$port" --bind 127.0.0.1 --directory "$tmp/www" >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 50); do curl -fsS -o /dev/null "$url/good/install.sh" 2>/dev/null && break; sleep 0.1; done

run() { # name directory [env...] -- runs the published one-liner
  local name=$1 dir=$2
  shift 2
  env BRIDGE_VM_HOME="$tmp/home-$name" BRIDGE_VM_BIN="$tmp/bin-$name" \
    BRIDGE_VM_KUBE_PORT="$(free_port)" BRIDGE_VM_ORKA_PORT="$(free_port)" "$@" \
    bash -c "curl -fsSL $url/$dir/install.sh | bash"
}

out=$(run good good 2>&1) || fail "install: $out"
home=$tmp/home-good
grep -q 'Orka v0.2.0 is ready' <<<"$out" || fail "no ready message: $out"
[ -x "$tmp/bin-good/bridge-vm" ] || fail "bridge-vm was not installed"
[ "$(stat -c %a "$home/kubeconfig")" = 600 ] || fail "kubeconfig is not private"
grep -q 'server: https://127.0.0.1:[0-9]*$' "$home/kubeconfig" || fail "kubeconfig server"
! grep -q '6443\|kind-kaimahi-p1' "$home/kubeconfig" || fail "kubeconfig not rewritten"
grep -q 'current-context: orka-bridge-vm' "$home/kubeconfig" || fail "kubeconfig context"
[ ! -w "$home/images/orka-bridge-vm-orka-v0.2.0-linux-amd64.qcow2" ] || fail "image is writable"
pass "the one-liner downloads, verifies, boots and writes a private kubeconfig"

vm() { BRIDGE_VM_HOME=$home "$tmp/bin-good/bridge-vm" "$@"; }
out=$(run good good 2>&1) || fail "reinstall: $out"
grep -q 'already downloaded and verified' <<<"$out" || fail "image downloaded twice: $out"
grep -q 'already running' <<<"$out" || fail "second VM started: $out"
pass "running the one-liner again reuses the image and the running VM"

image=$home/images/orka-bridge-vm-orka-v0.2.0-linux-amd64.qcow2
before=$(sha256sum "$image")
publish "$tmp/www/good"
if out=$(run good good 2>&1); then fail "replaced the image under an existing VM"; fi
grep -q 'a VM from another image exists' <<<"$out" || fail "no refusal: $out"
[ "$(sha256sum "$image")" = "$before" ] || fail "the image under the VM changed"
pass "a rebuilt image with the same name never replaces the one under an existing VM"

vm status | grep -q '^Running: ready' || fail "status while running: $(vm status)"
# The guest reports "ready" once per boot; status asks the Orka API itself.
mv "$home/instance/orka-bridge-vm-web/readyz" "$home/instance/orka-bridge-vm-web/readyz.off"
vm status | grep -q '^Running: booted, but the Orka API is not answering' ||
  fail "status with the Orka API down: $(vm status)"
mv "$home/instance/orka-bridge-vm-web/readyz.off" "$home/instance/orka-bridge-vm-web/readyz"
pass "status checks that the Orka API still answers"

vm stop | grep -q stopped || fail "stop"
vm status | grep -q '^Stopped' || fail "status after stop: $(vm status)"
out=$(BRIDGE_VM_KUBE_PORT=$(free_port) BRIDGE_VM_ORKA_PORT=$(free_port) vm start 2>&1) || fail "start: $out"
grep -q 'is ready' <<<"$out" || fail "start: $out"
vm stop >/dev/null
pass "stop and start"

out=$(run stubborn good FAKE_GUEST=stubborn 2>&1) || fail "install for the stop test: $out"
svm() { BRIDGE_VM_HOME=$tmp/home-stubborn BRIDGE_VM_KILL_WAIT=1 "$tmp/bin-stubborn/bridge-vm" "$@"; }
qpid=$(cat "$tmp/home-stubborn/instance/qemu.pid")
if out=$(svm stop 2>&1); then fail "stop succeeded while QEMU kept running: $out"; fi
grep -q 'is still running' <<<"$out" || fail "no error for a QEMU that kept running: $out"
[ -f "$tmp/home-stubborn/instance/qemu.pid" ] || fail "stop dropped the pid of a running QEMU"
if svm remove >/dev/null 2>&1; then fail "removed the VM while QEMU kept running"; fi
[ -f "$tmp/home-stubborn/instance/disk.qcow2" ] || fail "remove deleted a disk QEMU still uses"
kill -KILL "$qpid"
out=$(svm stop 2>&1) || fail "stop after QEMU exited: $out"
pass "stop waits for QEMU to exit and keeps its pid while it runs"

out=$(cd "$tmp" && env BRIDGE_VM_HOME=home-relative BRIDGE_VM_BIN="$tmp/bin-relative" \
  BRIDGE_VM_KUBE_PORT="$(free_port)" BRIDGE_VM_ORKA_PORT="$(free_port)" \
  bash -c "curl -fsSL $url/good/install.sh | bash" 2>&1) || fail "relative BRIDGE_VM_HOME: $out"
backing=$(grep -A1 '^-b$' "$tmp/home-relative/instance/disk.qcow2.args" | tail -n1)
case $backing in
  "$tmp"/home-relative/images/*) ;;
  *) fail "backing file for a relative BRIDGE_VM_HOME: $backing" ;;
esac
BRIDGE_VM_HOME=$tmp/home-relative "$tmp/bin-relative/bridge-vm" stop >/dev/null
pass "a relative BRIDGE_VM_HOME still gives the VM an absolute backing file"

if out=$(run bad-image bad-image 2>&1); then fail "installed an image with the wrong checksum"; fi
grep -q 'checksum mismatch' <<<"$out" || fail "no checksum error: $out"
[ -z "$(ls -A "$tmp/home-bad-image/images")" ] || fail "kept a bad image"
[ ! -e "$tmp/home-bad-image/instance/disk.qcow2" ] || fail "created a VM from a bad image"
pass "an image that does not match the manifest is refused and removed"

if out=$(run bad-cli bad-cli 2>&1); then fail "installed a tampered bridge-vm"; fi
grep -q 'checksum mismatch' <<<"$out" || fail "no checksum error: $out"
[ ! -e "$tmp/bin-bad-cli/bridge-vm" ] || fail "installed a tampered bridge-vm"
pass "a bridge-vm that does not match the manifest is refused"

if out=$(run failed failed FAKE_GUEST=failed 2>&1); then fail "reported ready for a failed guest"; fi
grep -q 'the Orka controller did not become ready' <<<"$out" || fail "no guest reason: $out"
BRIDGE_VM_HOME=$tmp/home-failed "$tmp/bin-failed/bridge-vm" stop >/dev/null
pass "a guest failure is reported with its reason"

out=$(run partial partial FAKE_GUEST=partial 2>&1) || fail "partial ready line: $out"
grep -q 'current-context: orka-bridge-vm' "$tmp/home-partial/kubeconfig" || fail "truncated kubeconfig"
BRIDGE_VM_HOME=$tmp/home-partial "$tmp/bin-partial/bridge-vm" stop >/dev/null
pass "a ready line that arrives in pieces is read only when complete"

# Python's http.server ignores Range requests, so a stale partial download must restart.
mkdir -p "$tmp/home-resume/images"
touch "$tmp/home-resume/.bridge-vm"
head -c 1000 "$tmp/www/resume/orka-bridge-vm-orka-v0.2.0-linux-amd64.qcow2" \
  >"$tmp/home-resume/images/orka-bridge-vm-orka-v0.2.0-linux-amd64.qcow2.part"
out=$(run resume resume 2>&1) || fail "resume without range support: $out"
BRIDGE_VM_HOME=$tmp/home-resume "$tmp/bin-resume/bridge-vm" stop >/dev/null
pass "a partial download restarts when the server cannot resume it"

if out=$(run no-url no-url 2>&1); then fail "installed without an address"; fi
grep -q 'set BRIDGE_VM_URL' <<<"$out" || fail "no address error: $out"
pass "install.sh without a built-in address asks for BRIDGE_VM_URL"

mkdir -p "$tmp/home-foreign"
echo keep >"$tmp/home-foreign/notes.txt"
if out=$(run foreign good 2>&1); then fail "installed into a directory bridge-vm does not own"; fi
grep -q 'not created by bridge-vm' <<<"$out" || fail "no ownership error: $out"
if BRIDGE_VM_HOME=$tmp/home-foreign "$tmp/bin-foreign/bridge-vm" remove >/dev/null 2>&1; then
  fail "removed a directory bridge-vm does not own"
fi
[ -f "$tmp/home-foreign/notes.txt" ] || fail "deleted files in a foreign BRIDGE_VM_HOME"
pass "install and remove leave a BRIDGE_VM_HOME that bridge-vm did not create alone"

vm remove >/dev/null
[ ! -e "$home" ] || fail "remove left $home"
[ ! -e "$tmp/bin-good/bridge-vm" ] || fail "remove left the bridge-vm command"
pass "remove deletes the VM state and the command"
