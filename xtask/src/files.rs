//! Link-refusing file checks and bounded reads shared by the image, evidence and VMware tooling.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, ensure};

pub(crate) fn require_real_directory(path: &Path, label: &str) -> Result<()> {
    crate::workspace::canonical_existing_real_directory(path, label)?;
    Ok(())
}

pub(crate) fn read_bounded_file(path: &Path, maximum: u64, label: &str) -> Result<Vec<u8>> {
    let parent = path
        .parent()
        .with_context(|| format!("{label} has no parent directory"))?;
    crate::workspace::canonical_existing_real_directory(parent, label)?;
    crate::workspace::require_direct_file(parent, path, label)?;
    let mut file = crate::workspace::open_real_file(path, label)?;
    let initial = file
        .metadata()
        .with_context(|| format!("measuring {label} {}", path.display()))?
        .len();
    ensure!(
        initial <= maximum,
        "{label} is {initial} bytes, exceeding the {maximum}-byte allocation limit"
    );
    let capacity = usize::try_from(initial).context("bounded file length does not fit usize")?;
    let mut bytes = Vec::with_capacity(capacity);
    (&mut file)
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {label} {}", path.display()))?;
    ensure!(
        bytes.len() as u64 <= maximum,
        "{label} grew beyond the {maximum}-byte allocation limit while being read"
    );
    ensure!(
        file.metadata()
            .with_context(|| format!("remeasuring {label} {}", path.display()))?
            .len()
            == bytes.len() as u64,
        "{label} changed length while being read"
    );
    crate::workspace::require_direct_file(parent, path, label)?;
    Ok(bytes)
}

pub(crate) fn read_bounded_text(path: &Path, maximum: u64, label: &str) -> Result<String> {
    String::from_utf8(read_bounded_file(path, maximum, label)?)
        .with_context(|| format!("{label} {} is not UTF-8", path.display()))
}

pub(crate) fn boot_image_bytes() -> u64 {
    crate::paths::DISK_MIB * 1024 * 1024
}

pub(crate) fn namespace_bytes() -> u64 {
    crate::qemu::NVME_VOLUME_SECTOR * crate::qemu::NVME_BLOCK_BYTES as u64
        + crate::image::corpus::VOLUME_BYTES as u64
}

#[cfg(windows)]
pub(crate) fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub(crate) const fn is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}
