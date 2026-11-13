use std::fs;

fn contains(binary: &[u8], needle: &[u8]) -> bool {
    binary.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn actual_app_binary_contains_the_local_image_selector_only_in_debug() {
    let binary = fs::read(env!("CARGO_BIN_EXE_orka-bridge")).unwrap();
    // Cargo supplies the actual app target. Check its executable format, not
    // an unrelated message whose wording can change without changing this gate.
    #[cfg(windows)]
    assert!(binary.starts_with(b"MZ"));
    #[cfg(target_os = "macos")]
    assert!(binary.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]));
    #[cfg(target_os = "linux")]
    assert!(binary.starts_with(b"\x7fELF"));
    assert_eq!(
        contains(&binary, b"BRIDGE_DEV_IMAGE_DIR"),
        cfg!(debug_assertions),
        "The local image selector must exist only in the debug app"
    );
    #[cfg(all(
        not(debug_assertions),
        any(
            all(windows, target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "aarch64")
        )
    ))]
    for hash in if cfg!(windows) {
        [
            "0233d40dbb5cc3c89c6ecc2f2802b975c588cca3de17907ade4853d6a59c02e4",
            "9aa023fd1b0187d5fc6b3099a51f63d4390f8dacbf3884b6d4f42a945252b7f4",
            "4a44487bb3cfdc2be1d7697b8c71c97733d6ab2db62e6e6e8f1c6af9431ffd2e",
            "2454d45db754d144cdd63b01ea0b973a0aefcb94e4376aa66cc261978a667885",
        ]
    } else {
        [
            "a186512dcf6cdf6c1dc556e8e57f7012385584288d2e8f0d7622efd5a2f80e13",
            "4f75a6f374923f9a6569997c65bb7f5ac65ff1f0ac1c65d5af2c8fd4e803d0fd",
            "27f833e233753a0a601f3a6792dcd73e2f2c7c8bd30e00b837315c71c5996dcb",
            "3bd75d66535a48ae837afcb938fbd831e0e65ef9080cf19de5952929cb5d23c8",
        ]
    } {
        let raw: Vec<u8> = (0..64)
            .step_by(2)
            .map(|index| u8::from_str_radix(&hash[index..index + 2], 16).unwrap())
            .collect();
        assert!(
            contains(&binary, hash.as_bytes()) || contains(&binary, &raw),
            "The supported release must carry its reviewed image/runtime pins"
        );
    }
}
