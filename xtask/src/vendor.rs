//! Verifying vendored binaries against recorded digests.
//!
//! `boot/limine/BOOTX64.EFI` is a prebuilt binary from another project. Pinning
//! its digest is what makes "update the bootloader" a reviewable change rather
//! than an opaque one.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Component, Path};

use anyhow::{Context, Result, bail, ensure};

use crate::paths::Paths;
use crate::sha256;

pub(crate) const MAX_LISTING_BYTES: u64 = 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 256;
pub(crate) const MAX_PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIRECTORY_PAYLOAD_BYTES: u64 = 256 * 1024 * 1024;

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
    let mut entries = Vec::new();
    let mut names = BTreeSet::new();
    for line in listing.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
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
        let mut components = Path::new(name).components();
        ensure!(
            matches!(
                core::iter::Iterator::next(&mut components),
                Some(Component::Normal(_))
            ) && core::iter::Iterator::next(&mut components).is_none()
                && !name.contains(['/', '\\'])
                && !name.chars().any(char::is_control),
            "checksum file name is not one direct regular-file name: {name}"
        );
        ensure!(
            names.insert(name.to_string()),
            "checksum listing repeats file name `{name}`"
        );
        ensure!(
            entries.len() < MAX_DIRECTORY_ENTRIES,
            "checksum listing has more than {MAX_DIRECTORY_ENTRIES} entries"
        );
        entries.push(Entry {
            digest: digest.to_ascii_lowercase(),
            name: name.to_string(),
        });
    }
    Ok(entries)
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
    let dir = listing.parent().unwrap_or(Path::new("."));
    crate::mutation_workspace::canonical_existing_real_directory(dir, "vendor directory")?;
    crate::mutation_workspace::require_direct_file(dir, listing, "vendor checksum listing")?;
    let text = read_listing(listing)?;
    let entries = parse(&text).with_context(|| format!("parsing {}", listing.display()))?;

    let mut measured = Vec::with_capacity(entries.len());
    let mut total = 0u64;
    for entry in &entries {
        let file = dir.join(&entry.name);
        crate::mutation_workspace::require_direct_file(dir, &file, "vendored payload")?;
        let length = crate::mutation_workspace::open_real_file(&file, "vendored payload")?
            .metadata()
            .with_context(|| format!("measuring {}", file.display()))?
            .len();
        ensure!(
            length <= MAX_PAYLOAD_BYTES,
            "{} is {length} bytes, exceeding the {MAX_PAYLOAD_BYTES}-byte vendored-payload limit",
            file.display()
        );
        total = total
            .checked_add(length)
            .context("vendored payload length overflowed")?;
        ensure!(
            total <= MAX_DIRECTORY_PAYLOAD_BYTES,
            "{} contains {total} payload bytes, exceeding its {MAX_DIRECTORY_PAYLOAD_BYTES}-byte limit",
            dir.display()
        );
        measured.push((entry, file, length));
    }

    for (entry, file, length) in measured {
        crate::mutation_workspace::require_direct_file(dir, &file, "vendored payload")?;
        let input = crate::mutation_workspace::open_real_file(&file, "vendored payload")?;
        let (bytes, digest) = sha256::digest_reader(input.take(length + 1))
            .with_context(|| format!("hashing {}", file.display()))?;
        crate::mutation_workspace::require_direct_file(dir, &file, "vendored payload")?;
        ensure!(
            bytes == length
                && crate::mutation_workspace::open_real_file(&file, "vendored payload")?
                    .metadata()
                    .with_context(|| format!("remeasuring {}", file.display()))?
                    .len()
                    == length,
            "{} changed length while it was hashed",
            file.display()
        );
        let actual = sha256::hex(&digest);
        if actual != entry.digest {
            bail!(
                "{} does not match its recorded digest\n  recorded: {}\n  actual:   {}",
                file.display(),
                entry.digest,
                actual
            );
        }
    }

    let mut present = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
        let entry = entry.with_context(|| format!("reading an entry in {}", dir.display()))?;
        ensure!(
            present.len() < MAX_DIRECTORY_ENTRIES,
            "{} has more than {MAX_DIRECTORY_ENTRIES} entries",
            dir.display()
        );
        present.push(
            entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("{} has a non-UTF-8 entry", dir.display()))?,
        );
    }

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

/// Verify one already-read vendored payload against its sole checksum entry.
///
/// The caller can write `bytes` after this returns without reopening `payload`,
/// so authentication and consumption name the same snapshot.
pub(crate) fn verify_payload_bytes(listing: &Path, payload: &Path, bytes: &[u8]) -> Result<()> {
    let directory = listing
        .parent()
        .with_context(|| format!("{} has no vendor directory", listing.display()))?;
    ensure!(
        payload.parent() == Some(directory),
        "vendored payload {} is not beside {}",
        payload.display(),
        listing.display()
    );
    let name = payload
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| {
            format!(
                "vendored payload has no UTF-8 file name: {}",
                payload.display()
            )
        })?;
    let text = read_listing(listing)?;
    let entries = parse(&text).with_context(|| format!("parsing {}", listing.display()))?;
    ensure!(
        entries.len() == 1,
        "{} must contain exactly one payload digest, found {}",
        listing.display(),
        entries.len()
    );
    let entry = &entries[0];
    ensure!(
        entry.name == name,
        "{} pins `{}` instead of the consumed payload `{name}`",
        listing.display(),
        entry.name
    );
    ensure!(
        bytes.len() as u64 <= MAX_PAYLOAD_BYTES,
        "{} is {} bytes, exceeding the {MAX_PAYLOAD_BYTES}-byte vendored-payload limit",
        payload.display(),
        bytes.len()
    );
    let actual = sha256::hex(&sha256::digest(bytes));
    ensure!(
        actual == entry.digest,
        "{} does not match its recorded digest\n  recorded: {}\n  actual:   {}",
        payload.display(),
        entry.digest,
        actual
    );
    Ok(())
}

pub(crate) fn read_listing(path: &Path) -> Result<String> {
    read_listing_with(path, || Ok(()))
}

fn read_listing_with<F>(path: &Path, after_measurement: F) -> Result<String>
where
    F: FnOnce() -> Result<()>,
{
    let parent = path.parent().unwrap_or(Path::new("."));
    crate::mutation_workspace::canonical_existing_real_directory(parent, "vendor directory")?;
    crate::mutation_workspace::require_direct_file(parent, path, "vendor checksum listing")?;
    let mut file = crate::mutation_workspace::open_real_file(path, "vendor checksum listing")?;
    let initial = file
        .metadata()
        .with_context(|| format!("measuring {}", path.display()))?
        .len();
    ensure!(
        initial <= MAX_LISTING_BYTES,
        "{} is {initial} bytes, exceeding the {MAX_LISTING_BYTES}-byte checksum-listing limit",
        path.display()
    );
    after_measurement()?;
    let capacity =
        usize::try_from(initial).context("checksum listing length does not fit usize")?;
    let mut bytes = Vec::with_capacity(capacity);
    (&mut file)
        .take(MAX_LISTING_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    ensure!(
        bytes.len() as u64 <= MAX_LISTING_BYTES,
        "{} grew beyond the {MAX_LISTING_BYTES}-byte checksum-listing limit",
        path.display()
    );
    ensure!(
        file.metadata()
            .with_context(|| format!("remeasuring {}", path.display()))?
            .len()
            == bytes.len() as u64,
        "{} changed length while it was read",
        path.display()
    );
    crate::mutation_workspace::require_direct_file(parent, path, "vendor checksum listing")?;
    String::from_utf8(bytes).with_context(|| format!("{} is not UTF-8", path.display()))
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
    use std::fs::File;

    use super::*;

    const DIGEST: &str = "91ef81c1f95c4ac8bf77c4393a0b2fcd2628550827d7abf89a43194400e17a85";

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating vendor listing symlink");
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_file(target, link).expect("creating vendor listing symlink");
    }

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

    #[test]
    fn checksum_names_are_unique_direct_children() {
        assert!(parse(&format!("{DIGEST}  ../outside")).is_err());
        assert!(parse(&format!("{DIGEST}  nested/payload")).is_err());
        assert!(parse(&format!("{DIGEST}  nested\\payload")).is_err());
        assert!(parse(&format!("{DIGEST}  payload\n{DIGEST}  payload\n")).is_err());
    }

    #[test]
    fn checksum_entry_count_is_bounded_independently_of_text_size() {
        let listing: String = (0..=MAX_DIRECTORY_ENTRIES)
            .map(|index| format!("{DIGEST}  payload-{index}\n"))
            .collect();
        let error = parse(&listing).unwrap_err().to_string();
        assert!(error.contains("more than 256 entries"), "{error}");
    }

    #[test]
    fn consumed_payload_authentication_requires_one_exact_adjacent_entry() {
        let root =
            std::env::temp_dir().join(format!("wizard-vendor-consumption-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let payload = root.join("BOOTX64.EFI");
        let listing = root.join("CHECKSUMS");
        let bytes = b"reviewed bootloader";
        std::fs::write(&payload, bytes).unwrap();
        let digest = sha256::hex(&sha256::digest(bytes));

        std::fs::write(&listing, format!("{digest} *BOOTX64.EFI\n")).unwrap();
        verify_payload_bytes(&listing, &payload, bytes).unwrap();

        std::fs::write(
            &listing,
            format!("{digest} *BOOTX64.EFI\n{digest} *BOOTIA32.EFI\n"),
        )
        .unwrap();
        let error = verify_payload_bytes(&listing, &payload, bytes)
            .unwrap_err()
            .to_string();
        assert!(error.contains("exactly one payload digest"), "{error}");

        std::fs::write(&listing, format!("{digest} *BOOTIA32.EFI\n")).unwrap();
        let error = verify_payload_bytes(&listing, &payload, bytes)
            .unwrap_err()
            .to_string();
        assert!(error.contains("instead of the consumed payload"), "{error}");

        let outside = root.parent().unwrap().join(format!(
            "wizard-vendor-consumption-outside-{}",
            std::process::id()
        ));
        let error = verify_payload_bytes(&listing, &outside, bytes)
            .unwrap_err()
            .to_string();
        assert!(error.contains("is not beside"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn consumed_payload_authentication_refuses_a_redirected_listing() {
        let root = std::env::temp_dir().join(format!(
            "wizard-vendor-consumption-link-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        let vendor = root.join("vendor");
        let outside = root.join("outside");
        std::fs::create_dir_all(&vendor).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let bytes = b"substituted bootloader";
        let payload = vendor.join("BOOTX64.EFI");
        std::fs::write(&payload, bytes).unwrap();
        let external_listing = outside.join("CHECKSUMS");
        let digest = sha256::hex(&sha256::digest(bytes));
        let external_bytes = format!("{digest} *BOOTX64.EFI\n");
        std::fs::write(&external_listing, &external_bytes).unwrap();
        let listing = vendor.join("CHECKSUMS");
        create_file_symlink(&external_listing, &listing);

        let result = verify_payload_bytes(&listing, &payload, bytes);
        let retained = std::fs::read(&external_listing).unwrap();
        std::fs::remove_dir_all(&root).unwrap();

        let error = result.expect_err("a redirected checksum listing must be refused");
        assert!(
            format!("{error:#}").contains("not a real file"),
            "unexpected refusal: {error:#}"
        );
        assert_eq!(retained, external_bytes.as_bytes());
    }

    #[test]
    fn oversized_checksum_listings_are_refused_before_allocation() {
        let root = std::env::temp_dir().join(format!(
            "wizard-vendor-listing-bound-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let listing = root.join("CHECKSUMS");
        File::create(&listing)
            .unwrap()
            .set_len(MAX_LISTING_BYTES + 1)
            .unwrap();

        let error = verify_listing(&listing).unwrap_err().to_string();
        assert!(
            error.contains("exceeding the 1048576-byte checksum-listing limit"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checksum_listing_growth_is_refused_after_measurement() {
        let root = std::env::temp_dir().join(format!(
            "wizard-vendor-listing-growth-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let listing = root.join("CHECKSUMS");
        std::fs::write(&listing, b"x").unwrap();
        let growing = listing.clone();

        let error = read_listing_with(&listing, move || {
            let mut options = std::fs::OpenOptions::new();
            std::fs::OpenOptions::write(&mut options, true);
            std::fs::OpenOptions::open(&options, growing)?.set_len(MAX_LISTING_BYTES + 1)?;
            Ok(())
        })
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("grew beyond the 1048576-byte checksum-listing limit"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_vendored_payloads_are_refused_before_hashing() {
        let root = std::env::temp_dir().join(format!(
            "wizard-vendor-payload-bound-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let listing = root.join("CHECKSUMS");
        std::fs::write(&listing, format!("{DIGEST}  payload\n")).unwrap();
        File::create(root.join("payload"))
            .unwrap()
            .set_len(MAX_PAYLOAD_BYTES + 1)
            .unwrap();

        let error = verify_listing(&listing).unwrap_err().to_string();
        assert!(
            error.contains("exceeding the 67108864-byte vendored-payload limit"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vendored_directories_have_an_independent_total_payload_bound() {
        let root =
            std::env::temp_dir().join(format!("wizard-vendor-total-bound-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let mut listing = String::new();
        for index in 0..5 {
            let name = format!("payload-{index}");
            listing.push_str(&format!("{DIGEST}  {name}\n"));
            File::create(root.join(name))
                .unwrap()
                .set_len(MAX_PAYLOAD_BYTES)
                .unwrap();
        }
        let checksum_path = root.join("CHECKSUMS");
        std::fs::write(&checksum_path, listing).unwrap();

        let error = verify_listing(&checksum_path).unwrap_err().to_string();
        assert!(
            error.contains("exceeding its 268435456-byte limit"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
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
            for entry in crate::paths::directory_entries(&next).unwrap_or_else(|reason| {
                panic!(
                    "complete directory listing for {}: {reason}",
                    next.display()
                )
            }) {
                let path = entry.path();
                if entry.is_dir() {
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
