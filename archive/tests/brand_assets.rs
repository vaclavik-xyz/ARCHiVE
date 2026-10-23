//! `assets/icon.png` exists only so tools such as T3 Code discover the project
//! icon; `docs/brand/favicon.png` is the master (see `docs/brand/README.md`).

use std::path::Path;

fn read(rel: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn discoverable_project_icon_matches_brand_master() {
    // `assert!` rather than `assert_eq!`: a mismatch must not dump ~1 MB of bytes.
    assert!(
        read("assets/icon.png") == read("docs/brand/favicon.png"),
        "assets/icon.png drifted; re-sync with: cp docs/brand/favicon.png assets/icon.png"
    );
}
