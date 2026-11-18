#!/usr/bin/env bash
# Test scripts/macos-dark-mode.sh with a fake osascript, then against the real
# System Events while it is quitting (macOS only).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
helper="$root/scripts/macos-dark-mode.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/bridge-dark-mode-test.XXXXXX")
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin"
cat >"$work/bin/osascript" <<'EOF'
#!/usr/bin/env bash
calls=$(( $(cat "$FAKE_CALLS") + 1 )); echo "$calls" >"$FAKE_CALLS"
reply() { case "$2" in *"get dark mode") echo false ;; esac; }
case "$FAKE_MODE" in
  flaky) if (( calls < 3 )); then echo "71:80: execution error: System Events got an error: Connection is invalid. (-609)" >&2; exit 1; fi; reply "$@" ;;
  always) echo "execution error: Connection is invalid. (-609)" >&2; exit 1 ;;
  stopping) if (( calls < 2 )); then echo "execution error: System Events got an error: Application isn't running. (-600)" >&2; exit 1; fi; reply "$@" ;;
  noisy) echo "osascript warning: unrelated" >&2; reply "$@" ;;
  stdout609) echo "(-609)"; exit 1 ;;
  denied) echo "execution error: Not authorized to send Apple events to System Events. (-1743)" >&2; exit 1 ;;
esac
EOF
chmod 0755 "$work/bin/osascript"

failures=0
# expect MODE STATUS OUTPUT CALLS ARGS...
expect() {
  local mode=$1 status=$2 output=$3 calls=$4 actual_status=0 actual
  shift 4
  echo 0 >"$work/calls"
  actual=$(PATH="$work/bin:$PATH" FAKE_MODE=$mode FAKE_CALLS="$work/calls" bash "$helper" "$@" 2>"$work/stderr") || actual_status=$?
  if [[ $actual_status != "$status" || $actual != "$output" || $(cat "$work/calls") != "$calls" ]]; then
    echo "FAIL $mode $*: status $actual_status, output [$actual], calls $(cat "$work/calls"); expected $status, [$output], $calls" >&2
    sed 's/^/  stderr: /' "$work/stderr" >&2
    failures=$((failures + 1))
  else
    echo "PASS $mode $*"
  fi
}
expect flaky 0 false 3 get
expect flaky 0 "" 3 set true
expect always 1 "" 5 get
expect always 1 "" 5 set false
expect noisy 0 false 1 get
expect stdout609 1 "" 1 get
expect stopping 0 false 2 get
expect denied 1 "" 1 get
expect denied 1 "" 1 set false
expect flaky 2 "" 0 set maybe
expect flaky 2 "" 0 get extra
expect flaky 2 "" 0
(( failures == 0 )) || exit 1

if [[ $(uname) == Darwin && ${GITHUB_ACTIONS:-} == true ]]; then
  # Quit System Events and send the next request at once, as an idle quit would race it.
  value=$(bash "$helper" get)
  [[ $value == true || $value == false ]]
  for _ in 1 2 3 4 5; do
    osascript -e 'tell application "System Events" to quit' >/dev/null 2>&1 || true
    test "$(bash "$helper" get)" = "$value"
  done
  bash "$helper" set "$value"
  echo "PASS native: helper read and set dark mode while System Events quit"
fi
