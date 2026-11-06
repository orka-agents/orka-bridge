#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/bridge-swift-test.XXXXXX")
trap 'rm -rf "$work"' EXIT
xcrun swiftc --version
native="$root/app/src-tauri/native/macos"
sources=("$native/BridgeVM.swift" "$native/VirtualizationDriver.swift"
         "$native/InheritedDescriptors.swift" "$native/SupervisorSocket.swift" "$native/TerminationObserver.swift"
         "$native/HelperConfiguration.swift")
flags=(-swift-version 5 -target arm64-apple-macos13.0 -warnings-as-errors -parse-as-library)
# Compile the production core independently of test code as well as in the runner.
xcrun swiftc "${flags[@]}" -whole-module-optimization -emit-object "${sources[@]}" -o "$work/BridgeVM.o"
xcrun swiftc "${flags[@]}" "${sources[@]}" "$native/tests/BridgeVMTests.swift" -o "$work/tests"
xcrun swiftc "${flags[@]}" "${sources[@]}" "$native/tests/SupervisorIOTests.swift" -o "$work/io-tests"
xcrun swiftc "${flags[@]}" "${sources[@]}" "$native/tests/HelperConfigurationTests.swift" -o "$work/configuration-tests"
for runner in tests io-tests configuration-tests; do
    codesign --force --sign - --entitlements "$native/virtualization.entitlements" "$work/$runner"
done
"$work/tests"
"$work/configuration-tests"
# Both new suites must run: an I/O assertion failure must not hide peer failures.
status=0
if ! "$work/io-tests"; then status=1; fi
if ! BRIDGE_TEST_SWIFT_PEER="$work/io-tests" CARGO_TERM_COLOR=never cargo +1.98.1 test \
    --manifest-path "$root/runtime/Cargo.toml" --locked -p bridge-runtime-protocol \
    --test apple_backend runtime::apple::tests::swift_supervisor_peer -- --exact --ignored \
    >"$work/rust-peer.log" 2>&1; then
    status=1
fi
cat "$work/rust-peer.log"
# An absent selector or a platform-gated test must never count as a native pass.
if ! grep -Eq '^test result: ok\. 1 passed; 0 failed;' "$work/rust-peer.log"; then status=1; fi
exit "$status"
