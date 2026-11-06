#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/bridge-swift-test.XXXXXX")
trap 'rm -rf "$work"' EXIT
xcrun swiftc --version
sources=("$root/app/src-tauri/native/macos/BridgeVM.swift" "$root/app/src-tauri/native/macos/VirtualizationDriver.swift")
flags=(-swift-version 5 -target arm64-apple-macos13.0 -warnings-as-errors -parse-as-library)
# Compile the production core independently of test code as well as in the runner.
xcrun swiftc "${flags[@]}" -whole-module-optimization -emit-object "${sources[@]}" -o "$work/BridgeVM.o"
xcrun swiftc "${flags[@]}" "${sources[@]}" "$root/app/src-tauri/native/macos/tests/BridgeVMTests.swift" -o "$work/tests"
"$work/tests"
