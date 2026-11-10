//! Link-refusing file checks shared by the evidence, qualification and mutation stores.

use std::path::Path;

use anyhow::Result;

pub(crate) fn require_real_directory(path: &Path, label: &str) -> Result<()> {
    crate::mutate::workspace::canonical_existing_real_directory(path, label)?;
    Ok(())
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
