#!/usr/bin/env bash
# Read or set the macOS dark-mode appearance through System Events.
# Usage: macos-dark-mode.sh get | macos-dark-mode.sh set true|false
#
# System Events quits when idle. An Apple event sent while it is quitting fails
# with "Connection is invalid. (-609)"; the next request relaunches it. Retry
# only that error, and fail on anything else.
set -uo pipefail

case "${1:-}" in
  get) [[ $# -eq 1 ]] || { echo "usage: $0 get" >&2; exit 2; }
       script='tell application "System Events" to tell appearance preferences to get dark mode' ;;
  set) [[ $# -eq 2 && ( $2 == true || $2 == false ) ]] || { echo "usage: $0 set true|false" >&2; exit 2; }
       script="tell application \"System Events\" to tell appearance preferences to set dark mode to $2" ;;
  *) echo "usage: $0 get | set true|false" >&2; exit 2 ;;
esac

for attempt in 1 2 3 4 5; do
  # osascript writes the result to stdout and an error to stderr only on failure.
  if output="$(osascript -e "$script" 2>&1)"; then
    [[ -n $output ]] && printf '%s\n' "$output"
    exit 0
  fi
  printf '%s\n' "$output" >&2
  if [[ $output != *'(-609)'* || $attempt == 5 ]]; then exit 1; fi
  echo "System Events connection closed; retrying ($attempt of 5)" >&2
  sleep 1
done
