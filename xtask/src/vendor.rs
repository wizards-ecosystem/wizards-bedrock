use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use crate::paths::Paths;
use crate::sha256;

// Unlisted files are refused, since a digest catches a changed file but never an added one.
const PAPERWORK: &[&str] = &["CHECKSUMS", "LICENSE", "README.md"];

#[derive(Clone, PartialEq, Eq, Debug)]
struct Entry {
    digest: String,
    name: String,
}

pub(crate) fn verify(paths: &Paths) -> Result<()> {
    for listing in [paths.limine_checksums(), paths.tla_checksums()] {
        let count = verify_listing(&listing)?;
        println!(
            "[xtask] {count} vendored file(s) match {}",
            listing.display()
        );
    }
    Ok(())
}

pub(crate) fn verify_listing(listing: &Path) -> Result<usize> {
    let directory = listing.parent().context("listing has no directory")?;
    let entries = entries(listing)?;
    for entry in &entries {
        let path = directory.join(&entry.name);
        let (_, digest) = File::open(&path)
            .and_then(sha256::digest_reader)
            .with_context(|| format!("hashing {}", path.display()))?;
        require_digest(&path, entry, &digest)?;
    }
    let present =
        std::fs::read_dir(directory).with_context(|| format!("listing {}", directory.display()))?;
    let mut extra = Vec::new();
    for file in present {
        let name = file?.file_name().to_string_lossy().into_owned();
        let listed = entries.iter().any(|entry| entry.name == name);
        if !listed && !PAPERWORK.contains(&name.as_str()) {
            extra.push(name);
        }
    }
    if !extra.is_empty() {
        extra.sort_unstable();
        bail!(
            "{} has files no digest accounts for: {}",
            directory.display(),
            extra.join(", ")
        );
    }
    Ok(entries.len())
}

pub(crate) fn read_verified(listing: &Path, payload: &Path) -> Result<Vec<u8>> {
    let name = payload
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("{} has no UTF-8 file name", payload.display()))?;
    let entries = entries(listing)?;
    let entry = entries
        .iter()
        .find(|entry| entry.name == name)
        .with_context(|| format!("{} lists no digest for {name}", listing.display()))?;
    let bytes = std::fs::read(payload).with_context(|| format!("reading {}", payload.display()))?;
    require_digest(payload, entry, &sha256::digest(&bytes))?;
    Ok(bytes)
}

fn require_digest(path: &Path, entry: &Entry, digest: &[u8; 32]) -> Result<()> {
    let actual = sha256::hex(digest);
    ensure!(
        actual == entry.digest,
        "{} does not match its recorded digest\n  recorded: {}\n  actual:   {actual}",
        path.display(),
        entry.digest
    );
    Ok(())
}

// Both `sha256sum` forms, `digest  name` and `digest *name`; blank and `#` lines are skipped.
fn entries(listing: &Path) -> Result<Vec<Entry>> {
    let text = std::fs::read_to_string(listing)
        .with_context(|| format!("reading {}", listing.display()))?;
    let mut entries = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (digest, name) = line
            .split_once(char::is_whitespace)
            .with_context(|| format!("checksum line has no file name: {line}"))?;
        let name = name.trim_start().trim_start_matches('*');
        ensure!(
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "not a sha256 digest: {digest}"
        );
        ensure!(
            !name.is_empty() && !name.contains(['/', '\\']),
            "checksum file name is not a name in its directory: {line}"
        );
        entries.push(Entry {
            digest: digest.to_ascii_lowercase(),
            name: name.to_string(),
        });
    }
    Ok(entries)
}
