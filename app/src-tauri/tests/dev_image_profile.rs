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
    #[cfg(not(debug_assertions))]
    for hash in [
        "b4cbacc7d7212c9e7e3a4ed4cac306bb466b165efcab6f05252d4b00860f5e5f",
        "33d6946647e7d6ab3e83e42a25caa497aea215ad1cac91d011473a4ad0a008ae",
        "03a3d4900bafb9970ea0eba6c7b0be19b57ba4fb2f0e3cabbd2f4a2ccd1571ba",
        "a2787ff53247377e0d16b94dbc7ee43033c00aea8379de0cfe68ce58f843426e",
        "27f833e233753a0a601f3a6792dcd73e2f2c7c8bd30e00b837315c71c5996dcb",
        "4a44487bb3cfdc2be1d7697b8c71c97733d6ab2db62e6e6e8f1c6af9431ffd2e",
        "694704f1580e9986fc5426bb9b3d1221be94c3f6ef81eff22df8b4eff658dcec",
        "6f843edde13d7a26b14e54e10838fe5a70aedb2a3c26bbfb998fd45ae90d743a",
        "a1dead19c0d1dcef558008e556cd9bd4a8c5082daf4ecea6f531d03173748385",
    ] {
        let raw: Vec<u8> = (0..64)
            .step_by(2)
            .map(|index| u8::from_str_radix(&hash[index..index + 2], 16).unwrap())
            .collect();
        assert!(
            !contains(&binary, hash.as_bytes()),
            "Dev hex pin in release"
        );
        assert!(!contains(&binary, &raw), "Dev raw pin in release");
    }
}
