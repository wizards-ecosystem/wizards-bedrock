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
#[derive(Clone, PartialEq, Eq, Debug)]
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
        if digest.len() != 64
            || !digest
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            bail!("not a sha256 digest: {digest}");
        }
        let name = name.trim_start().trim_start_matches('*');
        if name.is_empty() {
            bail!("checksum line has no file name: {line}");
        }
        let mut components = Path::new(name).components();
        ensure!(
            matches!(components.next(), Some(Component::Normal(_)))
                && components.next().is_none()
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
pub fn unaccounted<'name, I>(present: I, entries: &[Entry]) -> Vec<String>
where
    I: IntoIterator<Item = &'name str>,
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
    crate::mutate::workspace::canonical_existing_real_directory(dir, "vendor directory")?;
    crate::mutate::workspace::require_direct_file(dir, listing, "vendor checksum listing")?;
    let text = read_listing(listing)?;
    let entries = parse(&text).with_context(|| format!("parsing {}", listing.display()))?;

    let mut measured = Vec::with_capacity(entries.len());
    let mut total = 0u64;
    for entry in &entries {
        let file = dir.join(&entry.name);
        crate::mutate::workspace::require_direct_file(dir, &file, "vendored payload")?;
        let length = crate::mutate::workspace::open_real_file(&file, "vendored payload")?
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
        crate::mutate::workspace::require_direct_file(dir, &file, "vendored payload")?;
        let input = crate::mutate::workspace::open_real_file(&file, "vendored payload")?;
        let (bytes, digest) = sha256::digest_reader(input.take(length + 1))
            .with_context(|| format!("hashing {}", file.display()))?;
        crate::mutate::workspace::require_direct_file(dir, &file, "vendored payload")?;
        ensure!(
            bytes == length
                && crate::mutate::workspace::open_real_file(&file, "vendored payload")?
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
    crate::mutate::workspace::canonical_existing_real_directory(parent, "vendor directory")?;
    crate::mutate::workspace::require_direct_file(parent, path, "vendor checksum listing")?;
    let mut file = crate::mutate::workspace::open_real_file(path, "vendor checksum listing")?;
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
    crate::mutate::workspace::require_direct_file(parent, path, "vendor checksum listing")?;
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
mod tests;
