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
        "2c589a7ea777cd353e5a7664f04a6ab4e332d4e174732bf43f69f08f3fd6f403",
        "c18e950cfe3fde37e0dd31bbdc629aa06e369a3b800374273e185133ccb209dc",
        "4647c7068e7f3daeae282b0fc36921142522683482885eb679334752d71172dc",
        "069e00084af957b1068f7cf38869ba5b87f867a6986bc8dcab922f66cab04c9b",
        "27f833e233753a0a601f3a6792dcd73e2f2c7c8bd30e00b837315c71c5996dcb",
        "4a44487bb3cfdc2be1d7697b8c71c97733d6ab2db62e6e6e8f1c6af9431ffd2e",
        "1e0129f4ad2349e39e1bef65f6d7590f74294b0f1557257c6b2de41b15a112f6",
        "9e79b76a9a4174c0638e199dc6e477971f06091526b62e118cc9058fd95f3588",
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
