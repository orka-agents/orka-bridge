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
        "a240b644913e1a66b2f745706b7feb739f768f5803a0672ceb9a37eb6f4c1e27",
        "91393279cd7588a7e667e8870678e65babc332dae3269328b17d6ffccb5fb480",
        "9eb75d4a9de5f2e922157dc597601d7f10d3b71f3c52fd6ffd3f18d59c6a799b",
        "a0207fbf8d07001e25b07691daa5d17a3f7c2f7b7bd0b1c7d5b6250ef306e488",
        "27f833e233753a0a601f3a6792dcd73e2f2c7c8bd30e00b837315c71c5996dcb",
        "4a44487bb3cfdc2be1d7697b8c71c97733d6ab2db62e6e6e8f1c6af9431ffd2e",
        "606891f6e94ce5c4263bbc5360d7fdb6bb874b2109478e4e9e67322fd47d7f20",
        "51a5e3f113b2c445f2bd9e889f9e9819e1188937e2035c878f48a4d30adb9d61",
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
