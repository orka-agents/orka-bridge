use bridge_engine::runtime::image_cache::ImageCache;
#[test]
fn cache_open_only_initializes_a_native_local_namespace() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("cache");
    let _cache = ImageCache::open(&root).unwrap();
    assert!(root.join("images/v2").is_dir());
    assert_eq!(
        std::fs::read_dir(root.join("images/v2")).unwrap().count(),
        0
    );
}
