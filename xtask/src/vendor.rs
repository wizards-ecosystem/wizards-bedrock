//! Verifying vendored binaries against recorded digests.
//!
//! `boot/limine/BOOTX64.EFI` is a prebuilt binary from another project. Pinning
//! its digest is what makes "update the bootloader" a reviewable change rather
//! than an opaque one.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::paths::Paths;
use crate::sha256;

/// One line of a `sha256sum`-style checksum file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Lowercase hex digest.
    pub digest: String,
    /// File name, relative to the checksum file.
    pub name: String,
}

/// Parse a `sha256sum` listing.
///
/// Accepts both the text (`digest  name`) and binary (`digest *name`) forms,
/// ignoring blank lines and `#` comments.
pub fn parse(listing: &str) -> Result<Vec<Entry>> {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (digest, name) = line
                .split_once(char::is_whitespace)
                .with_context(|| format!("checksum line has no file name: {line}"))?;
            if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
                bail!("not a sha256 digest: {digest}");
            }
            let name = name.trim_start().trim_start_matches('*');
            if name.is_empty() {
                bail!("checksum line has no file name: {line}");
            }
            Ok(Entry {
                digest: digest.to_ascii_lowercase(),
                name: name.to_string(),
            })
        })
        .collect()
}

/// Check every file listed beside `listing` against its recorded digest.
pub fn verify_listing(listing: &Path) -> Result<usize> {
    let text = std::fs::read_to_string(listing)
        .with_context(|| format!("reading {}", listing.display()))?;
    let entries = parse(&text).with_context(|| format!("parsing {}", listing.display()))?;
    let dir = listing.parent().unwrap_or(Path::new("."));

    for entry in &entries {
        let file = dir.join(&entry.name);
        let bytes = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
        let actual = sha256::hex(&sha256::digest(&bytes));
        if actual != entry.digest {
            bail!(
                "{} does not match its recorded digest\n  recorded: {}\n  actual:   {}",
                file.display(),
                entry.digest,
                actual
            );
        }
    }
    Ok(entries.len())
}

/// Verify everything vendored in the repository.
pub fn verify(paths: &Paths) -> Result<()> {
    let listing = paths.limine_checksums();
    let count = verify_listing(&listing)?;
    println!(
        "[xtask] {count} vendored file(s) match {}",
        listing.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "91ef81c1f95c4ac8bf77c4393a0b2fcd2628550827d7abf89a43194400e17a85";

    #[test]
    fn parses_the_text_and_binary_forms() {
        let entries = parse(&format!(
            "# a comment\n\n{DIGEST}  plain.efi\n{DIGEST} *binary.efi\n"
        ))
        .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "plain.efi");
        assert_eq!(entries[1].name, "binary.efi");
        assert!(entries.iter().all(|entry| entry.digest == DIGEST));
    }

    #[test]
    fn uppercase_digests_are_normalised() {
        let entries = parse(&format!("{}  x.efi", DIGEST.to_ascii_uppercase())).unwrap();
        assert_eq!(entries[0].digest, DIGEST);
    }

    #[test]
    fn rejects_a_truncated_digest() {
        assert!(parse("deadbeef  x.efi").is_err());
    }

    #[test]
    fn rejects_a_line_with_no_file_name() {
        assert!(parse(DIGEST).is_err());
        assert!(parse(&format!("{DIGEST}   ")).is_err());
    }
}
