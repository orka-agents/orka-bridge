#!/usr/bin/env bash
# Read or set the macOS dark-mode appearance through System Events.
# Usage: macos-dark-mode.sh get | macos-dark-mode.sh set true|false
#
# System Events quits when idle. An Apple event sent while it is quitting fails
# with "Connection is invalid. (-609)" or "Application isn't running. (-600)";
# the next request relaunches it. Retry only those errors, and fail on anything else.
set -uo pipefail

case "${1:-}" in
  get) [[ $# -eq 1 ]] || { echo "usage: $0 get" >&2; exit 2; }
       script='tell application "System Events" to tell appearance preferences to get dark mode' ;;
  set) [[ $# -eq 2 && ( $2 == true || $2 == false ) ]] || { echo "usage: $0 set true|false" >&2; exit 2; }
       script="tell application \"System Events\" to tell appearance preferences to set dark mode to $2" ;;
  *) echo "usage: $0 get | set true|false" >&2; exit 2 ;;
esac

errors="$(mktemp)"
trap 'rm -f "$errors"' EXIT
for attempt in 1 2 3 4 5; do
  # Keep stdout exactly as osascript printed it; callers compare it.
  if output="$(osascript -e "$script" 2>"$errors")"; then
    cat "$errors" >&2
    [[ -n $output ]] && printf '%s\n' "$output"
    exit 0
  fi
  cat "$errors" >&2
  if ! grep -Fq -e '(-609)' -e '(-600)' "$errors" || [[ $attempt == 5 ]]; then exit 1; fi
  echo "System Events is quitting; retrying ($attempt of 5)" >&2
  sleep 1
done
exit 1
