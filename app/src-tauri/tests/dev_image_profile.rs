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
            "270e3a180ce55270b6e1b1ea5a7cdc09cccd2d8406aab07f4d9609112fcdc7db",
            "602f3a9f7871f590d44d277c1ffb294d8e931d47c655ace1796758b7f8520c3c",
            "4a44487bb3cfdc2be1d7697b8c71c97733d6ab2db62e6e6e8f1c6af9431ffd2e",
            "4febfbd8d144e84ce1526a3fcde2130c701a68fdffcc366b3663d0bb2c28797f",
        ]
    } else {
        [
            "3f0108d781819ee10cddf4836890c46dfb222b9fb44660b7f04ef84a1f9de724",
            "c825114be1778ba341ef0680aabd8478745b228e2fc3e71ee1bd2b256e9831ff",
            "27f833e233753a0a601f3a6792dcd73e2f2c7c8bd30e00b837315c71c5996dcb",
            "e78ca98b09c444d2d6ba2c73ae650c98ab49cf8e4d7319de5700066d324e7f41",
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
