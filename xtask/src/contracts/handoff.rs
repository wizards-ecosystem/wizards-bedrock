//! Repository-level qualification of the committed Wizzy handoff fixture.

use wizzy_handoff::{ArtifactIdentity, Versions};

use crate::sha256;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root")
        .to_path_buf()
}

#[test]
fn the_committed_handoff_fixture_is_self_contained() {
    let root = root();
    let manifest = std::fs::read_to_string(root.join("handoff/conformance-v1.wzh"))
        .expect("reading the handoff manifest");
    let artifact = std::fs::read(root.join("handoff/conformance-v1.artifact"))
        .expect("reading the handoff artifact");
    let digest = sha256::hex(&sha256::digest(&artifact));
    let versions = Versions {
        artifact: "fixture.artifact-v1",
        runtime: "fixture.runtime-v1",
        effects: "fixture.effects-v1",
    };
    let identity = ArtifactIdentity {
        bytes: artifact.len() as u64,
        sha256: &digest,
    };

    assert_eq!(wizzy_handoff::verify(&manifest, versions, identity), Ok(()));
}

#[test]
fn platform_v1_is_entry_ring_and_time_v1() {
    let platform_header =
        std::fs::read_to_string(root().join("fixtures/abi-pressure/abi/wizard_abi.h"))
            .expect("reading the platform ABI header");
    for declaration in [
        "#define WIZ_ENTRY_VERSION             1u",
        "#define WIZ_RING_VERSION 1u",
        "#define WIZ_TIME_VERSION 1u",
    ] {
        assert!(
            platform_header.lines().any(|line| line == declaration),
            "{declaration} is not the platform-v1 contract"
        );
    }
}
