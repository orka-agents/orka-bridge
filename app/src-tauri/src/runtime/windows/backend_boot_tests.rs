#![cfg(windows)]
use super::native::pin_boot;
use crate::runtime::{
    image_install::materialize_image, image_types::ImageTarget, journal::Journal,
    types::BackendKind,
};
use std::{
    fs::{self, OpenOptions},
    os::windows::fs::OpenOptionsExt,
};
mod fixture_runtime {
    pub use crate::runtime::{image_cache, image_types, manifest};
}
#[path = "../../../tests/support/runtime_image_fixture.rs"]
pub(crate) mod fixture;
#[test]
fn installed_boot_pins_deny_writes_and_reject_changed_bytes() {
    let home = std::env::var_os("USERPROFILE").unwrap();
    let temp = tempfile::tempdir_in(home).unwrap();
    let root = temp.path().join("install");
    let journal = Journal::create(&root, BackendKind::Qemu, b"pc-q35-11.0".to_vec()).unwrap();
    let verified = fixture::verified(temp.path(), ImageTarget::WindowsAmd64);
    let target = journal
        .image_target(&verified.image().identity().state_compatibility_id)
        .unwrap();
    let (_sender, closed) = tokio::sync::watch::channel(false);
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let image = rt
        .block_on(materialize_image(&verified, &target, closed, |_| {}))
        .unwrap();
    let held = pin_boot(&image, &root).unwrap();
    let kernel = root.join(image.kernel_path());
    let error = OpenOptions::new().write(true).open(&kernel).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(32));
    let error = OpenOptions::new()
        .access_mode(0x00010000)
        .share_mode(7)
        .open(&kernel)
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(32));
    drop(held);
    fs::write(&kernel, b"changed boot").unwrap();
    assert!(pin_boot(&image, &root).is_err());
}
