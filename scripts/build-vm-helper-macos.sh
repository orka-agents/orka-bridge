#!/bin/bash
set -euo pipefail
umask 077
root=$(cd "$(dirname "$0")/.." && pwd)
if [[ $(uname -s) != Darwin || $(uname -m) != arm64 ]]; then
    echo 'Apple helper requires an Apple silicon Mac with the Xcode Swift compiler' >&2
    exit 1
fi
native="$root/app/src-tauri/native/macos"
destination="$root/app/src-tauri/gen/apple-helper"
mkdir -p "$destination"
work=$(mktemp -d "$destination/.build.XXXXXX")
trap 'rm -rf "$work"' EXIT
sources=("$native/BridgeVM.swift" "$native/VirtualizationDriver.swift"
         "$native/InheritedDescriptors.swift" "$native/SupervisorSocket.swift"
         "$native/TerminationObserver.swift" "$native/HelperConfiguration.swift"
         "$native/HelperBootstrap.swift" "$native/BridgeVMMain.swift")
xcrun swiftc -swift-version 5 -target arm64-apple-macos13.0 \
    -warnings-as-errors -parse-as-library "${sources[@]}" -o "$work/BridgeVM"
codesign --force --sign - --entitlements "$native/virtualization.entitlements" "$work/BridgeVM"
codesign --verify --strict "$work/BridgeVM"
# Publish only a complete signed executable; the previous build survives failure.
mv -f "$work/BridgeVM" "$destination/BridgeVM"
