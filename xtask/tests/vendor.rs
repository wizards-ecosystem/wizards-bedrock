//! Tampering with a vendored binary must fail `cargo test`, not just
//! `cargo xtask verify-vendor`.

use std::path::PathBuf;

#[test]
fn vendored_binaries_match_their_recorded_digests() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits inside the workspace")
        .to_path_buf();
    let listing = root.join("boot").join("limine").join("CHECKSUMS");

    let verified = xtask::vendor::verify_listing(&listing).expect("vendored binaries verify");
    assert_eq!(
        verified, 1,
        "expected exactly the Limine executable to be pinned"
    );
}
