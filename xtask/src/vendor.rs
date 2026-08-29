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

/// Files a vendored directory carries as its own paperwork rather than as
/// payload, and which therefore need no digest.
///
/// Everything else in the directory must be listed. A digest catches a file
/// being *changed*; only this catches one being *added*, which is the easier
/// of the two to slip past a reviewer.
const PAPERWORK: &[&str] = &["CHECKSUMS", "LICENSE", "README.md"];

/// Names in a vendored directory that neither an entry nor the directory's own
/// paperwork accounts for, sorted so the report is stable.
pub fn unaccounted<'a, I>(present: I, entries: &[Entry]) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut extra: Vec<String> = present
        .into_iter()
        .filter(|name| !PAPERWORK.contains(name))
        .filter(|name| !entries.iter().any(|entry| entry.name == *name))
        .map(str::to_string)
        .collect();
    extra.sort_unstable();
    extra
}

/// Check every file beside `listing`: each entry against its recorded digest,
/// and the directory against the listing.
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

    let present = std::fs::read_dir(dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<Vec<String>>>()
        .with_context(|| format!("listing {}", dir.display()))?;

    let extra = unaccounted(present.iter().map(String::as_str), &entries);
    if !extra.is_empty() {
        bail!(
            "{} has files no digest accounts for: {}\n  add them to {} or remove them",
            dir.display(),
            extra.join(", "),
            listing.display()
        );
    }
    Ok(entries.len())
}

/// Verify everything vendored in the repository.
///
/// Every listing, not the first one: a directory added and not added *here* is
/// a directory nothing checks, which is the drift `paths.rs`'s
/// `default-members` test exists to prevent one layer up (D-105).
pub fn verify(paths: &Paths) -> Result<()> {
    for listing in [
        paths.limine_checksums(),
        paths.tla_checksums(),
        paths.fat32_corpus_checksums(),
    ] {
        let count = verify_listing(&listing)?;
        println!(
            "[xtask] {count} vendored file(s) match {}",
            listing.display()
        );
    }
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

    fn listed(names: &[&str]) -> Vec<Entry> {
        names
            .iter()
            .map(|name| Entry {
                digest: DIGEST.to_string(),
                name: (*name).to_string(),
            })
            .collect()
    }

    /// The gap a digest cannot close: a file that was *added* matches no
    /// recorded hash because it has none.
    #[test]
    fn a_file_no_entry_lists_is_unaccounted_for() {
        let entries = listed(&["BOOTX64.EFI"]);

        assert_eq!(
            unaccounted(["BOOTX64.EFI", "BOOTIA32.EFI"], &entries),
            ["BOOTIA32.EFI"]
        );
    }

    #[test]
    fn a_directory_holding_exactly_what_is_listed_is_accounted_for() {
        let entries = listed(&["BOOTX64.EFI"]);

        assert!(unaccounted(["BOOTX64.EFI"], &entries).is_empty());
        assert!(
            unaccounted([], &entries).is_empty(),
            "absence is the digest's job"
        );
    }

    /// The directory's own paperwork carries no payload, so it needs no digest
    /// — including `CHECKSUMS` itself, which could not hash itself anyway.
    #[test]
    fn paperwork_needs_no_digest() {
        let entries = listed(&["BOOTX64.EFI"]);
        let present = ["BOOTX64.EFI", "CHECKSUMS", "LICENSE", "README.md"];

        assert!(unaccounted(present, &entries).is_empty());
    }

    /// Several strays are all reported at once, in a stable order, so fixing
    /// them is one pass rather than one commit per file.
    #[test]
    fn every_stray_is_reported_in_a_stable_order() {
        let entries = listed(&["BOOTX64.EFI"]);

        assert_eq!(
            unaccounted(["zebra.efi", "BOOTX64.EFI", "alpha.efi"], &entries),
            ["alpha.efi", "zebra.efi"]
        );
    }

    /// Every vendored directory is one `verify` checks.
    ///
    /// `verify` walks a hand-written array, and its own doc says why that is
    /// the wrong shape: *"a directory added and not added here is a directory
    /// nothing checks."* The `unaccounted` machinery closes "a file was added"
    /// *within* a listed directory and leaves the outer case open (D-124).
    ///
    /// A vendored directory is one holding a `CHECKSUMS`, which is the same
    /// definition `verify` uses — so this finds what it should have listed.
    #[test]
    fn every_vendored_directory_is_verified() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");

        let listed = [
            root.join("boot").join("limine").join("CHECKSUMS"),
            root.join("tools").join("tla").join("CHECKSUMS"),
            root.join("tools").join("fat32-corpus").join("CHECKSUMS"),
        ];

        let mut found = Vec::new();
        let mut pending = vec![root.join("boot"), root.join("tools")];
        while let Some(next) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&next) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.file_name().is_some_and(|name| name == "CHECKSUMS") {
                    found.push(path);
                }
            }
        }

        for checksums in &found {
            assert!(
                listed.iter().any(|known| known == checksums),
                "{} is not one `verify` checks",
                checksums.display()
            );
        }
        assert_eq!(found.len(), listed.len(), "a listing has no directory");
    }
}
