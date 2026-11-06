use bridge_engine::runtime::{acquire::ImageHttpClient, image_types::*};
fn pin() -> ImagePin {
    ImagePin {
        release_id: "test-only".into(),
        manifest_url: "https://images.example.test/manifest.json?redacted=secret".into(),
        manifest_sha256: "a".repeat(64),
        accepted_schema_sha256: "b".repeat(64),
        target: ImageTarget::WindowsAmd64,
        asset_base: "https://assets.example.test/release/".into(),
        redirect_origins: vec!["https://cdn.example.test".into()],
        limits: TransferLimits {
            compressed_bytes: 4096,
            disk_bytes: 4096,
            kernel_bytes: 4096,
            initrd_bytes: 4096,
        },
    }
}
#[test]
fn native_pinned_https_policy_is_constructible_without_network() {
    assert!(ImageHttpClient::new(&pin()).is_ok());
}
#[test]
fn production_rejects_plaintext_credentials_loopback_and_nonorigin_redirect_rules() {
    for value in [
        "http://images.example.test/a",
        "https://user:password@images.example.test/a",
        "https://localhost/a",
        "https://127.0.0.1/a",
        "https://[::1]/a",
        "file:///manifest.json",
    ] {
        let mut p = pin();
        p.manifest_url = value.into();
        assert!(ImageHttpClient::new(&p).is_err(), "{value}");
    }
    let mut p = pin();
    p.redirect_origins = vec!["https://cdn.example.test/path".into()];
    assert!(ImageHttpClient::new(&p).is_err());
    let mut p = pin();
    p.asset_base = "https://assets.example.test/not-a-directory".into();
    assert!(ImageHttpClient::new(&p).is_err());
}
