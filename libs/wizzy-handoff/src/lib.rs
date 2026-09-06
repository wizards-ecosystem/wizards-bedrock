//! The route-neutral identity envelope between Wizzy and The Wizard's OS.
//!
//! The envelope says which target and ABI versions opaque artifact bytes need,
//! where those bytes came from, which effects they request, and which digest
//! identifies them. It does not say whether the bytes are bytecode, ELF or
//! Wasm.
//!
//! # Examples
//!
//! ```
//! use wizzy_handoff::{ArtifactIdentity, Versions, verify};
//!
//! let manifest = concat!(
//!     "wizard-wizzy-handoff-v1\n",
//!     "artifact.abi=example.artifact-v1\n",
//!     "artifact.bytes=3\n",
//!     "artifact.sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
//!     "effects.abi=example.effects-v1\n",
//!     "effects.required=example.echo\n",
//!     "platform.abi=wizard-platform-v1\n",
//!     "provenance.compiler-revision=1111111111111111111111111111111111111111\n",
//!     "provenance.lock.sha256=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
//!     "provenance.repository=https://example.invalid/wizzy\n",
//!     "provenance.runtime-revision=2222222222222222222222222222222222222222\n",
//!     "provenance.source-revision=3333333333333333333333333333333333333333\n",
//!     "runtime.abi=example.runtime-v1\n",
//!     "target.id=x86_64-unknown-wizard\n",
//! );
//! let versions = Versions {
//!     artifact: "example.artifact-v1",
//!     runtime: "example.runtime-v1",
//!     effects: "example.effects-v1",
//! };
//! let identity = ArtifactIdentity {
//!     bytes: 3,
//!     sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
//! };
//! assert!(verify(manifest, versions, identity).is_ok());
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::{
    fmt,
    net::{Ipv4Addr, Ipv6Addr},
};

/// Version header accepted by this parser.
pub const FORMAT: &str = "wizard-wizzy-handoff-v1";

/// Target identifier owned by The Wizard's OS.
pub const TARGET_ID: &str = "x86_64-unknown-wizard";

/// Aggregate version of the native entry, ring and time-page contract.
pub const PLATFORM_ABI: &str = "wizard-platform-v1";

const FIELDS: &[&str] = &[
    "artifact.abi",
    "artifact.bytes",
    "artifact.sha256",
    "effects.abi",
    "effects.required",
    "platform.abi",
    "provenance.compiler-revision",
    "provenance.lock.sha256",
    "provenance.repository",
    "provenance.runtime-revision",
    "provenance.source-revision",
    "runtime.abi",
    "target.id",
];

/// Compiler-owned versions an integration explicitly supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Versions<'a> {
    /// Serialized artifact ABI version.
    pub artifact: &'a str,
    /// Separable target-runtime ABI version.
    pub runtime: &'a str,
    /// Encoding version for the effect requirement set.
    pub effects: &'a str,
}

/// Identity calculated from the artifact bytes being consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactIdentity<'a> {
    /// Complete artifact length in bytes.
    pub bytes: u64,
    /// Lowercase SHA-256 of the complete artifact.
    pub sha256: &'a str,
}

/// Why a handoff record was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The record does not use canonical LF endings, including the terminal LF.
    NonCanonicalLineEnding,
    /// The envelope version is absent or unsupported.
    UnsupportedFormat,
    /// A required field is absent.
    MissingField,
    /// A line has no key/value separator.
    InvalidLine,
    /// A field is not part of this envelope version.
    UnknownField,
    /// Known fields are duplicated or out of canonical order.
    NonCanonicalFields,
    /// A field value is malformed or non-canonical.
    InvalidValue,
    /// A target or ABI version differs from the explicitly supported one.
    VersionMismatch,
    /// The consumed bytes differ in length or digest from the record.
    ArtifactMismatch,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        core::fmt::Write::write_str(
            f,
            match self {
                Self::NonCanonicalLineEnding => "handoff record does not use canonical LF endings",
                Self::UnsupportedFormat => "handoff record format is unsupported",
                Self::MissingField => "handoff record is missing a required field",
                Self::InvalidLine => "handoff record contains an invalid line",
                Self::UnknownField => "handoff record contains an unknown field",
                Self::NonCanonicalFields => "handoff fields are duplicated or out of order",
                Self::InvalidValue => "handoff record contains an invalid value",
                Self::VersionMismatch => "handoff target or ABI version is unsupported",
                Self::ArtifactMismatch => "artifact bytes do not match the handoff record",
            },
        )
    }
}

impl core::error::Error for Error {}

struct Record<'a> {
    artifact_abi: &'a str,
    artifact_bytes: u64,
    artifact_sha256: &'a str,
    effects_abi: &'a str,
    effects_required: &'a str,
    platform_abi: &'a str,
    compiler_revision: &'a str,
    lock_sha256: &'a str,
    repository: &'a str,
    runtime_revision: &'a str,
    source_revision: &'a str,
    runtime_abi: &'a str,
    target_id: &'a str,
}

/// Verify one canonical record against supported versions and consumed bytes.
///
/// Effect requirements are authenticated metadata only. Successful
/// verification does not grant the artifact any authority.
///
/// # Errors
///
/// Returns [`Error`] for any unknown, missing, malformed or mismatched part of
/// the record, including an artifact length or digest mismatch.
pub fn verify(
    manifest: &str,
    versions: Versions<'_>,
    artifact: ArtifactIdentity<'_>,
) -> Result<(), Error> {
    let record = parse(manifest)?;
    if !record_values_are_canonical(&record) {
        return Err(Error::InvalidValue);
    }
    if !versions_match(&record, versions) {
        return Err(Error::VersionMismatch);
    }
    if !identity_matches(&record, artifact) {
        return Err(Error::ArtifactMismatch);
    }
    Ok(())
}

fn parse(manifest: &str) -> Result<Record<'_>, Error> {
    if manifest.contains('\r') || !manifest.ends_with('\n') {
        return Err(Error::NonCanonicalLineEnding);
    }
    let mut lines = manifest.lines();
    if core::iter::Iterator::next(&mut lines) != Some(FORMAT) {
        return Err(Error::UnsupportedFormat);
    }

    let artifact_abi = field(&mut lines, FIELDS[0])?;
    let artifact_bytes =
        canonical_decimal(field(&mut lines, FIELDS[1])?).ok_or(Error::InvalidValue)?;
    let artifact_sha256 = field(&mut lines, FIELDS[2])?;
    let effects_abi = field(&mut lines, FIELDS[3])?;
    let effects_required = field(&mut lines, FIELDS[4])?;
    let platform_abi = field(&mut lines, FIELDS[5])?;
    let compiler_revision = field(&mut lines, FIELDS[6])?;
    let lock_sha256 = field(&mut lines, FIELDS[7])?;
    let repository = field(&mut lines, FIELDS[8])?;
    let runtime_revision = field(&mut lines, FIELDS[9])?;
    let source_revision = field(&mut lines, FIELDS[10])?;
    let runtime_abi = field(&mut lines, FIELDS[11])?;
    let target_id = field(&mut lines, FIELDS[12])?;
    let extra = core::iter::Iterator::next(&mut lines);
    if !no_extra_fields(extra) {
        let known = extra
            .and_then(|line| line.split_once('='))
            .is_some_and(|(key, _)| FIELDS.contains(&key));
        return Err(if known {
            Error::NonCanonicalFields
        } else {
            Error::UnknownField
        });
    }

    Ok(Record {
        artifact_abi,
        artifact_bytes,
        artifact_sha256,
        effects_abi,
        effects_required,
        platform_abi,
        compiler_revision,
        lock_sha256,
        repository,
        runtime_revision,
        source_revision,
        runtime_abi,
        target_id,
    })
}

fn field<'a>(lines: &mut core::str::Lines<'a>, expected: &str) -> Result<&'a str, Error> {
    let line = core::iter::Iterator::next(lines).ok_or(Error::MissingField)?;
    let (key, value) = line.split_once('=').ok_or(Error::InvalidLine)?;
    if key != expected {
        return if FIELDS.contains(&key) {
            Err(Error::NonCanonicalFields)
        } else {
            Err(Error::UnknownField)
        };
    }
    Ok(value)
}

fn no_extra_fields(extra: Option<&str>) -> bool {
    extra.is_none()
}

fn record_values_are_canonical(record: &Record<'_>) -> bool {
    token(record.artifact_abi)
        && token(record.runtime_abi)
        && token(record.effects_abi)
        && sha256(record.artifact_sha256)
        && effects(record.effects_required)
        && repository(record.repository)
        && git_revision(record.source_revision)
        && git_revision(record.compiler_revision)
        && git_revision(record.runtime_revision)
        && sha256(record.lock_sha256)
}

fn versions_match(record: &Record<'_>, versions: Versions<'_>) -> bool {
    record.target_id == TARGET_ID
        && record.platform_abi == PLATFORM_ABI
        && record.artifact_abi == versions.artifact
        && record.runtime_abi == versions.runtime
        && record.effects_abi == versions.effects
}

fn identity_matches(record: &Record<'_>, artifact: ArtifactIdentity<'_>) -> bool {
    record.artifact_bytes == artifact.bytes && record.artifact_sha256 == artifact.sha256
}

fn canonical_decimal(value: &str) -> Option<u64> {
    if value.is_empty()
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    <u64 as core::str::FromStr>::from_str(value).ok()
}

fn token(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = core::iter::Iterator::next(&mut bytes) else {
        return false;
    };
    first.is_ascii_lowercase()
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

fn effects(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let mut previous = None;
    for effect in value.split(',') {
        if !token(effect) || previous.is_some_and(|before| before >= effect) {
            return false;
        }
        previous = Some(effect);
    }
    true
}

fn repository(value: &str) -> bool {
    if !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return false;
    }
    let Some(location) = value.strip_prefix("https://") else {
        return false;
    };
    let authority_end = location
        .bytes()
        .position(|byte| matches!(byte, b'/' | b'?' | b'#'))
        .unwrap_or(location.len());
    repository_authority(&location[..authority_end])
}

fn repository_authority(authority: &str) -> bool {
    if let Some(literal) = authority.strip_prefix('[') {
        let Some(close) = literal.find(']') else {
            return false;
        };
        let host = &literal[..close];
        let suffix = &literal[close + 1..];
        return <Ipv6Addr as core::str::FromStr>::from_str(host).is_ok()
            && (suffix.is_empty() || suffix.strip_prefix(':').is_some_and(repository_port));
    }

    if authority.contains(['[', ']']) {
        return false;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    !host.contains(':') && repository_host(host) && port.is_none_or(repository_port)
}

fn repository_host(host: &str) -> bool {
    if host.len() > 253 {
        return false;
    }
    if host
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return <Ipv4Addr as core::str::FromStr>::from_str(host).is_ok();
    }
    host.split('.').all(|label| {
        label.len() <= 63
            && label
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && label
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

fn repository_port(port: &str) -> bool {
    !port.is_empty()
        && !port.starts_with('0')
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && <u16 as core::str::FromStr>::from_str(port).is_ok_and(|port| port != 0)
}

fn git_revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
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
            let malformed =
                MANIFEST.replacen("https://example.invalid/wizzy-fixture", repository, 1);
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
            let manifest =
                MANIFEST.replacen("https://example.invalid/wizzy-fixture", repository, 1);
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
            let manifest =
                MANIFEST.replacen("https://example.invalid/wizzy-fixture", &repository, 1);
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
}
