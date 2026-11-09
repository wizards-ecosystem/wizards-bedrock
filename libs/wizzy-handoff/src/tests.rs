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
fn a_canonical_record_is_accepted() {
    assert_eq!(verify(MANIFEST, VERSIONS, IDENTITY), Ok(()));
}

#[test]
fn a_canonical_record_has_exactly_one_terminal_lf() {
    let missing = MANIFEST
        .strip_suffix('\n')
        .expect("the fixture has a terminal LF");
    let extra = format!("{MANIFEST}\n");

    assert_eq!(
        verify(missing, VERSIONS, IDENTITY),
        Err(Error::NonCanonicalLineEnding)
    );
    assert!(verify(&extra, VERSIONS, IDENTITY).is_err());
}

#[test]
fn an_unknown_envelope_version_is_refused() {
    let unknown = MANIFEST.replacen(FORMAT, "wizard-wizzy-handoff-v2", 1);
    assert_eq!(
        verify(&unknown, VERSIONS, IDENTITY),
        Err(Error::UnsupportedFormat)
    );
}

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

#[test]
fn every_version_must_match_exactly() {
    for (from, to) in [
        (TARGET_ID, "aarch64-unknown-wizard"),
        (PLATFORM_ABI, "wizard-platform-v2"),
        (VERSIONS.artifact, "fixture.artifact-v2"),
        (VERSIONS.runtime, "fixture.runtime-v2"),
        (VERSIONS.effects, "fixture.effects-v2"),
    ] {
        let mismatched = MANIFEST.replacen(from, to, 1);
        assert_eq!(
            verify(&mismatched, VERSIONS, IDENTITY),
            Err(Error::VersionMismatch),
            "accepted {to}"
        );
    }
}

#[test]
fn every_value_encoding_is_canonical() {
    for (from, to) in [
        ("artifact.bytes=7", "artifact.bytes=07"),
        (
            "artifact.sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "artifact.sha256=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ),
        (
            "effects.required=fixture.console,fixture.sleep",
            "effects.required=fixture.sleep,fixture.console",
        ),
        (
            "provenance.repository=https://example.invalid/wizzy-fixture",
            "provenance.repository=../wizards-ink",
        ),
        (
            "provenance.source-revision=3333333333333333333333333333333333333333",
            "provenance.source-revision=local-main",
        ),
    ] {
        let malformed = MANIFEST.replacen(from, to, 1);
        assert_eq!(
            verify(&malformed, VERSIONS, IDENTITY),
            Err(Error::InvalidValue),
            "accepted {to}"
        );
    }
}

#[test]
fn repository_provenance_requires_a_host_bearing_https_authority() {
    for repository in [
        "https://",
        "https:///wizzy",
        "https://?source=wizzy",
        "https://#wizzy",
        "https://@/wizzy",
        "https://user@example.invalid/wizzy",
        "https://:443/wizzy",
        "https://example.invalid:/wizzy",
        "https://example.invalid:0/wizzy",
        "https://example.invalid:0443/wizzy",
        "https://example.invalid:65536/wizzy",
        "https://example.invalid:not-a-port/wizzy",
        "https://[::1/wizzy",
        "https://::1]/wizzy",
        "https://[]/wizzy",
        "https://[example.invalid]/wizzy",
        "https://[::1]extra/wizzy",
        "https://example..invalid/wizzy",
        "https://-example.invalid/wizzy",
        "https://example_.invalid/wizzy",
        "https://exam_ple.invalid/wizzy",
        "https://999.999.999.999/wizzy",
    ] {
        let malformed = MANIFEST.replacen("https://example.invalid/wizzy-fixture", repository, 1);
        assert_eq!(
            verify(&malformed, VERSIONS, IDENTITY),
            Err(Error::InvalidValue),
            "accepted {repository}"
        );
    }
}

#[test]
fn repository_provenance_accepts_each_supported_host_form() {
    for repository in [
        "https://example.invalid/wizzy",
        "https://example.invalid:443/wizzy",
        "https://127.0.0.1/wizzy",
        "https://[2001:db8::1]/wizzy",
        "https://[2001:db8::1]:443/wizzy",
    ] {
        let manifest = MANIFEST.replacen("https://example.invalid/wizzy-fixture", repository, 1);
        assert_eq!(
            verify(&manifest, VERSIONS, IDENTITY),
            Ok(()),
            "refused {repository}"
        );
    }
}

#[test]
fn repository_host_length_boundary_is_exact() {
    let label = "a".repeat(63);
    let accepted_host = format!("{label}.{label}.{label}.{}", "a".repeat(61));
    let refused_host = format!("{label}.{label}.{label}.{}", "a".repeat(62));
    assert_eq!(accepted_host.len(), 253);
    assert_eq!(refused_host.len(), 254);

    for (host, expected) in [
        (accepted_host, Ok(())),
        (refused_host, Err(Error::InvalidValue)),
    ] {
        let repository = format!("https://{host}/wizzy");
        let manifest = MANIFEST.replacen("https://example.invalid/wizzy-fixture", &repository, 1);
        assert_eq!(
            verify(&manifest, VERSIONS, IDENTITY),
            expected,
            "host length {}",
            host.len()
        );
    }
}

#[test]
fn artifact_length_and_digest_identify_the_same_bytes() {
    let wrong_length = ArtifactIdentity {
        bytes: IDENTITY.bytes + 1,
        ..IDENTITY
    };
    let wrong_digest = ArtifactIdentity {
        sha256: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        ..IDENTITY
    };
    assert_eq!(
        verify(MANIFEST, VERSIONS, wrong_length),
        Err(Error::ArtifactMismatch)
    );
    assert_eq!(
        verify(MANIFEST, VERSIONS, wrong_digest),
        Err(Error::ArtifactMismatch)
    );
}

#[test]
fn errors_name_the_refused_record_property() {
    let cases = [
        (
            Error::NonCanonicalLineEnding,
            "handoff record does not use canonical LF endings",
        ),
        (
            Error::UnsupportedFormat,
            "handoff record format is unsupported",
        ),
        (
            Error::MissingField,
            "handoff record is missing a required field",
        ),
        (
            Error::InvalidLine,
            "handoff record contains an invalid line",
        ),
        (
            Error::UnknownField,
            "handoff record contains an unknown field",
        ),
        (
            Error::NonCanonicalFields,
            "handoff fields are duplicated or out of order",
        ),
        (
            Error::InvalidValue,
            "handoff record contains an invalid value",
        ),
        (
            Error::VersionMismatch,
            "handoff target or ABI version is unsupported",
        ),
        (
            Error::ArtifactMismatch,
            "artifact bytes do not match the handoff record",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn decimals_tokens_and_revisions_enforce_each_canonical_boundary() {
    assert_eq!(canonical_decimal("17"), Some(17));
    for invalid in ["", "0", "00", "07", "-1", "+1", "1_0"] {
        assert_eq!(canonical_decimal(invalid), None, "accepted {invalid:?}");
    }

    for valid in ["a", "artifact.v1", "effect-name_2"] {
        assert!(token(valid), "refused {valid:?}");
    }
    for invalid in ["", "A", ".artifact", "artifact-", "artifact!v1"] {
        assert!(!token(invalid), "accepted {invalid:?}");
    }

    assert!(git_revision("0123456789abcdef0123456789abcdef01234567"));
    assert!(!git_revision("gggggggggggggggggggggggggggggggggggggggg"));
    assert!(!git_revision("0123456789abcdef"));
}
