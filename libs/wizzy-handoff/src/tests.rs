use super::*;

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MANIFEST: &str = concat!(
    "wizard-wizzy-handoff-v1\n",
    "artifact.abi=fixture.artifact-v1\n",
    "artifact.bytes=7\n",
    "artifact.sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
    "effects.abi=fixture.effects-v1\n",
    "effects.required=fixture.console,fixture.sleep\n",
    "platform.abi=wizard-platform-v1\n",
    "provenance.compiler-revision=1111111111111111111111111111111111111111\n",
    "provenance.lock.sha256=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
    "provenance.repository=https://example.invalid/wizzy-fixture\n",
    "provenance.runtime-revision=2222222222222222222222222222222222222222\n",
    "provenance.source-revision=3333333333333333333333333333333333333333\n",
    "runtime.abi=fixture.runtime-v1\n",
    "target.id=x86_64-unknown-wizard\n",
);

const VERSIONS: Versions<'static> = Versions {
    artifact: "fixture.artifact-v1",
    runtime: "fixture.runtime-v1",
    effects: "fixture.effects-v1",
};

const IDENTITY: ArtifactIdentity<'static> = ArtifactIdentity {
    bytes: 7,
    sha256: DIGEST,
};

#[test]
fn unknown_or_noncanonical_fields_are_refused() {
    let unknown = format!("{MANIFEST}delivery.route=bytecode\n");
    assert_eq!(
        verify(&unknown, VERSIONS, IDENTITY),
        Err(Error::UnknownField)
    );

    let reordered = MANIFEST.replace(
        "artifact.abi=fixture.artifact-v1\nartifact.bytes=7",
        "artifact.bytes=7\nartifact.abi=fixture.artifact-v1",
    );
    assert_eq!(
        verify(&reordered, VERSIONS, IDENTITY),
        Err(Error::NonCanonicalFields)
    );

    let duplicate = format!("{MANIFEST}target.id={TARGET_ID}\n");
    assert_eq!(
        verify(&duplicate, VERSIONS, IDENTITY),
        Err(Error::NonCanonicalFields)
    );
}
