use super::*;

pub(super) fn drive_file(options: &str, path: &Path) -> [OsString; 2] {
    let escaped = path.to_string_lossy().replace(',', ",,");
    [
        OsString::from("-drive"),
        OsString::from(format!("{options},file={escaped}")),
    ]
}

pub(super) fn snapshot_drive_file(options: &str, path: &Path) -> [OsString; 2] {
    drive_file(&format!("{options},snapshot=on"), path)
}

static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

/// A name another run already holds in the private run directory is skipped, never reused.
pub(super) fn reserve_run_file(
    target: &Path,
    stem: &str,
    extension: &str,
) -> Result<(PathBuf, PathBuf, File)> {
    let directory = crate::image::qemu_run_directory(target)?;
    for _ in 0..1024 {
        let sequence = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            "{stem}-{}-{sequence}.{extension}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        OpenOptions::write(&mut options, true);
        OpenOptions::create_new(&mut options, true);
        match OpenOptions::open(&options, &path) {
            Ok(file) => return Ok((directory, path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| format!("creating {}", path.display()));
            }
        }
    }
    bail!("could not reserve a private QEMU {stem} file")
}

#[derive(Debug)]
pub(super) struct RunVars {
    path: PathBuf,
}

impl RunVars {
    pub(super) fn create(target: &Path, template: &Path) -> Result<Self> {
        Self::create_with(target, template, |_| {})
    }

    pub(super) fn create_with(
        target: &Path,
        template: &Path,
        after_reservation: impl Fn(&Path),
    ) -> Result<Self> {
        let (directory, path, mut destination) = reserve_run_file(target, "efi-vars", "fd")?;
        after_reservation(&path);
        let copied = (|| -> Result<()> {
            let mut source =
                crate::mutate::workspace::open_real_file(template, "QEMU variable firmware")?;
            std::io::copy(&mut source, &mut destination)
                .with_context(|| format!("copying {} to {}", template.display(), path.display()))?;
            destination
                .sync_all()
                .with_context(|| format!("flushing {}", path.display()))?;
            drop(destination);
            crate::mutate::workspace::require_direct_file(
                &directory,
                &path,
                "private QEMU variable store",
            )?;
            ensure!(
                crate::mutate::workspace::canonical_existing_real_directory(
                    &directory,
                    "QEMU private run directory",
                )? == directory,
                "QEMU private run directory changed while copying firmware"
            );
            let source = crate::sha256::digest_reader(crate::mutate::workspace::open_real_file(
                template,
                "QEMU variable firmware",
            )?)
            .with_context(|| format!("hashing private firmware {}", template.display()))?;
            let copied = crate::sha256::digest_reader(crate::mutate::workspace::open_real_file(
                &path,
                "private QEMU variable store",
            )?)
            .with_context(|| format!("hashing private firmware copy {}", path.display()))?;
            ensure!(copied == source, "private firmware copy changed bytes");
            Ok(())
        })();
        crate::mutate::workspace::finish_staged_operation(
            &directory,
            &path,
            "private QEMU variable store",
            copied,
        )?;
        Ok(Self { path })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn directory(&self) -> &Path {
        self.path
            .parent()
            .expect("a reserved QEMU variable-store path has a parent")
    }
}

impl Drop for RunVars {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The coverage table states the real catch-all topology beside QEMU's own DMAR; the handoff
/// table states the entry-time DMA contract once CPU state authenticates this platform.
/// Both files must stay owned until the machine terminates.
#[derive(Debug)]
pub(super) struct RunDmaCoverage {
    path: Option<PathBuf>,
    handoff: PathBuf,
}

impl RunDmaCoverage {
    pub(super) fn create(target: &Path, remapping: Remapping) -> Result<Self> {
        let path = (remapping == Remapping::Present)
            .then(|| {
                write_run_acpi_table(
                    target,
                    "dma-coverage",
                    &qemu_dma_coverage_table(),
                    "DMA coverage attestation",
                )
            })
            .transpose()?;
        let handoff = match write_run_acpi_table(
            target,
            "dma-handoff",
            &crate::platform_acpi::dma_handoff_table(),
            "DMA handoff authority",
        ) {
            Ok(handoff) => handoff,
            Err(error) => {
                let cleanup = path.as_ref().map_or(Ok(()), std::fs::remove_file);
                return match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(anyhow!(
                        "{error:#}; removing private DMA coverage table also failed: {cleanup}"
                    )),
                };
            }
        };
        Ok(Self { path, handoff })
    }

    pub(super) fn arguments(&self) -> Vec<OsString> {
        let mut arguments = Vec::with_capacity(4);
        if let Some(path) = &self.path {
            arguments.extend([OsString::from("-acpitable"), acpi_file_argument(path)]);
        }
        arguments.extend([
            OsString::from("-acpitable"),
            acpi_file_argument(&self.handoff),
        ]);
        arguments
    }
}

fn write_run_acpi_table(target: &Path, stem: &str, bytes: &[u8], label: &str) -> Result<PathBuf> {
    let (directory, path, mut destination) = reserve_run_file(target, stem, "bin")?;
    let written = (|| -> Result<()> {
        destination
            .write_all(bytes)
            .with_context(|| format!("writing {}", path.display()))?;
        destination
            .sync_all()
            .with_context(|| format!("flushing {}", path.display()))?;
        drop(destination);
        crate::mutate::workspace::require_direct_file(
            &directory,
            &path,
            &format!("private QEMU {label}"),
        )?;
        ensure!(
            crate::mutate::workspace::canonical_existing_real_directory(
                &directory,
                "QEMU private run directory",
            )? == directory,
            "QEMU private run directory changed while writing {label}"
        );
        Ok(())
    })();
    crate::mutate::workspace::finish_staged_operation(
        &directory,
        &path,
        &format!("private QEMU {label}"),
        written,
    )?;
    Ok(path)
}

pub(super) fn acpi_file_argument(path: &Path) -> OsString {
    let file = path
        .file_name()
        .expect("a reserved QEMU ACPI table path has a file name")
        .to_string_lossy()
        .replace(',', ",,");
    OsString::from(format!("file={file}"))
}

impl Drop for RunDmaCoverage {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_file(&self.handoff);
    }
}

pub(super) fn qemu_dma_coverage_table() -> [u8; QEMU_DMA_COVERAGE_BYTES] {
    let mut table = [0u8; QEMU_DMA_COVERAGE_BYTES];
    table[0..4].copy_from_slice(b"WZDM");
    table[4..8].copy_from_slice(&(QEMU_DMA_COVERAGE_BYTES as u32).to_le_bytes());
    table[8] = 1;
    table[10..16].copy_from_slice(b"WIZARD");
    table[16..24].copy_from_slice(b"BEDROCK ");
    table[24..28].copy_from_slice(&1u32.to_le_bytes());
    table[28..32].copy_from_slice(b"WZRD");
    table[32..36].copy_from_slice(&1u32.to_le_bytes());
    table[36..38].copy_from_slice(&0u16.to_le_bytes()); // Segment zero.
    table[38..40].copy_from_slice(&0u16.to_le_bytes()); // Reserved.
    table[40..48].copy_from_slice(&0xfed9_0000u64.to_le_bytes());
    table[9] = 0u8.wrapping_sub(table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
    table
}
