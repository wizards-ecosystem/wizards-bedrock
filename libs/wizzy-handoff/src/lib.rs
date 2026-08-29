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

use core::fmt;

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
    /// The record contains a carriage return rather than canonical LF endings.
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
        f.write_str(match self {
            Self::NonCanonicalLineEnding => "handoff record does not use canonical LF endings",
            Self::UnsupportedFormat => "handoff record format is unsupported",
            Self::MissingField => "handoff record is missing a required field",
            Self::InvalidLine => "handoff record contains an invalid line",
            Self::UnknownField => "handoff record contains an unknown field",
            Self::NonCanonicalFields => "handoff fields are duplicated or out of order",
            Self::InvalidValue => "handoff record contains an invalid value",
            Self::VersionMismatch => "handoff target or ABI version is unsupported",
            Self::ArtifactMismatch => "artifact bytes do not match the handoff record",
        })
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
    if manifest.contains('\r') {
        return Err(Error::NonCanonicalLineEnding);
    }
    let mut lines = manifest.lines();
    if lines.next() != Some(FORMAT) {
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
    let extra = lines.next();
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
    let line = lines.next().ok_or(Error::MissingField)?;
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
        && record.repository.starts_with("https://")
        && record
            .repository
            .bytes()
            .all(|byte| byte.is_ascii_graphic())
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
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return None;
    }
    value.parse::<u64>().ok().filter(|value| *value != 0)
}

fn token(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
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
}
