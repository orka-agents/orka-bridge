use std::fs;

fn contains(binary: &[u8], needle: &[u8]) -> bool {
    binary.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn actual_app_binary_contains_the_local_image_selector_only_in_debug() {
    let binary = fs::read(env!("CARGO_BIN_EXE_orka-bridge")).unwrap();
    // Positive control: inspecting the wrong/empty file must not pass exclusion.
    assert!(contains(&binary, b"error while running Bridge"));
    assert_eq!(
        contains(&binary, b"BRIDGE_DEV_IMAGE_DIR"),
        cfg!(debug_assertions),
        "The local image selector must exist only in the debug app"
    );
    #[cfg(not(debug_assertions))]
    for hash in [
        "77518696b514004b78fd3390dd6103565b54c2f5b0bb01fad6d035e19ef97a22",
        "4c8007757f1814eab249c84fbdbc51cbd8736aa16938b006a063079adf2c583b",
        "b99c4e062151f1a8ad3fe68fdcc6d81ca4c550bbc5e3d4e64dfc7c405c81a075",
        "56f75f268bc1739ae80edd12a5a5e5c11241286d71608b145a0087e1c8a2eed0",
        "27f833e233753a0a601f3a6792dcd73e2f2c7c8bd30e00b837315c71c5996dcb",
        "4a44487bb3cfdc2be1d7697b8c71c97733d6ab2db62e6e6e8f1c6af9431ffd2e",
        "0b5b9f353e5acfa91a6e0ed2f0f28ee6d22ce48664d2a7b7b98d45f0439aa369",
        "dc96ba25f698e17d041d87046ddd33cde9a0c5757f4bf6c6356e44776fd8f2d2",
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
