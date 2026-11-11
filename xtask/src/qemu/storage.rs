use std::io::{Read, Seek, SeekFrom, Write};

use super::*;

/// The kernel reads the block size from `Identify`; the content below is written in blocks
/// and must agree with QEMU's 512-byte format.
pub const NVME_BLOCK_BYTES: usize = 512;

pub const NVME_SIGNED_BLOCKS: u64 = 8;

/// The first eight bytes of block `lba`, distinct in every block so a driver that skipped the
/// read or ignored the logical block address would fail.
pub const fn nvme_signature(lba: u64) -> u64 {
    0x5749_5A44_0000_0000 | lba
}

/// The FAT32 volume starts a megabyte in, where a partition table would put it, so the reader
/// must take a base sector.
pub const NVME_VOLUME_SECTOR: u64 = 2048;

/// Distinct blocks for the block-layer checks fill the first megabyte and the FAT32 corpus
/// follows on the same namespace, as on a real disk.
pub(super) const NVME_BYTES: u64 =
    NVME_VOLUME_SECTOR * NVME_BLOCK_BYTES as u64 + crate::image::corpus::VOLUME_BYTES as u64;

/// The namespace the QEMU and VMware runners share, machine state apart from the boot image.
pub fn nvme_image(root: &Path) -> Result<std::path::PathBuf> {
    let (parent, backing) = storage_paths(root)?;
    publish_namespace(&parent, &backing, write_nvme_namespace)?;
    Ok(backing)
}

pub(super) fn storage_paths(root: &Path) -> Result<(PathBuf, PathBuf)> {
    let backing = nvme_path(root);
    let parent = backing
        .parent()
        .context("NVMe image has no parent")?
        .to_path_buf();
    crate::mutate::workspace::prepare_real_directory_tree(&parent, "QEMU storage directory")?;
    Ok((parent, backing))
}

const STORAGE_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const STORAGE_STAGING_NAME: &str = "nvme.img.incomplete";
const STORAGE_ZERO_CHUNK_BYTES: usize = 64 * 1024;

pub(super) fn acquire_storage_lock(
    parent: &Path,
    path: &Path,
    canonical_parent: &Path,
) -> Result<File> {
    acquire_storage_lock_with(parent, path, canonical_parent, || {})
}

pub(super) fn acquire_storage_lock_with(
    parent: &Path,
    path: &Path,
    canonical_parent: &Path,
    after_contention: impl FnOnce(),
) -> Result<File> {
    let file = open_storage_lock(parent, path, canonical_parent)?;
    let deadline = Instant::now() + STORAGE_LOCK_TIMEOUT;
    let mut after_contention = Some(after_contention);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                if let Some(after_contention) = after_contention.take() {
                    after_contention();
                }
                if Instant::now() >= deadline {
                    bail!("another runner did not release {}", path.display());
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("locking storage build {}", path.display()));
            }
        }
    }
}

fn open_storage_lock(parent: &Path, path: &Path, canonical_parent: &Path) -> Result<File> {
    open_storage_lock_with(parent, path, canonical_parent, |_| {})
}

pub(super) fn open_storage_lock_with(
    parent: &Path,
    path: &Path,
    canonical_parent: &Path,
    after_open: impl FnOnce(&Path),
) -> Result<File> {
    require_storage_lock_path(parent, path, false)?;
    let mut options = OpenOptions::new();
    OpenOptions::create(&mut options, true);
    OpenOptions::truncate(&mut options, false);
    OpenOptions::read(&mut options, true);
    OpenOptions::write(&mut options, true);
    let file = OpenOptions::open(&options, path)
        .with_context(|| format!("opening storage build lock {}", path.display()))?;
    after_open(path);
    require_storage_lock_path(parent, path, true)?;
    ensure!(
        crate::mutate::workspace::canonical_existing_real_directory(
            parent,
            "QEMU storage directory",
        )? == canonical_parent,
        "QEMU storage directory changed while opening its lock"
    );
    Ok(file)
}

fn require_storage_lock_path(parent: &Path, path: &Path, must_exist: bool) -> Result<()> {
    ensure!(
        path == parent.join("nvme.lock"),
        "storage build lock is not the exact allowlisted path"
    );
    let present = crate::mutate::workspace::require_direct_file_if_present(
        parent,
        path,
        "storage build lock",
    )?;
    ensure!(
        !must_exist || present,
        "storage build lock disappeared after it was opened: {}",
        path.display()
    );
    Ok(())
}

pub(super) fn publish_namespace(
    parent: &Path,
    backing: &Path,
    write_expected: impl FnOnce(&mut File) -> Result<()>,
) -> Result<()> {
    ensure!(
        backing == parent.join("nvme.img"),
        "NVMe image is not the exact allowlisted storage path"
    );
    let canonical_parent = crate::mutate::workspace::canonical_existing_real_directory(
        parent,
        "QEMU storage directory",
    )?;
    let lock_path = parent.join("nvme.lock");
    let _lock = acquire_storage_lock(parent, &lock_path, &canonical_parent)?;
    crate::mutate::workspace::require_direct_file_if_present(
        parent,
        backing,
        "existing NVMe image",
    )?;

    let staging = parent.join(STORAGE_STAGING_NAME);
    if crate::mutate::workspace::require_direct_file_if_present(
        parent,
        &staging,
        "NVMe staging image",
    )? {
        std::fs::remove_file(&staging)
            .with_context(|| format!("removing stale NVMe stage {}", staging.display()))?;
    }
    let result = (|| {
        let mut options = OpenOptions::new();
        OpenOptions::create_new(&mut options, true);
        OpenOptions::read(&mut options, true);
        OpenOptions::write(&mut options, true);
        let mut file = OpenOptions::open(&options, &staging)
            .with_context(|| format!("creating NVMe staging image {}", staging.display()))?;
        write_expected(&mut file)?;
        file.sync_all()
            .with_context(|| format!("syncing NVMe staging image {}", staging.display()))?;
        drop(file);

        crate::mutate::workspace::require_direct_file(parent, &staging, "NVMe staging image")?;
        ensure!(
            crate::mutate::workspace::canonical_existing_real_directory(
                parent,
                "QEMU storage directory",
            )? == canonical_parent,
            "QEMU storage directory changed during image construction"
        );
        let expected = crate::sha256::digest_reader(crate::mutate::workspace::open_real_file(
            &staging,
            "NVMe staging image",
        )?)
        .with_context(|| format!("hashing NVMe staging image {}", staging.display()))?;

        crate::mutate::workspace::publish_staged_file(
            parent,
            &staging,
            backing,
            "NVMe image",
            |published| {
                let digest = crate::sha256::digest_reader(
                    crate::mutate::workspace::open_real_file(published, "published NVMe image")?,
                )
                .with_context(|| format!("hashing published NVMe image {}", published.display()))?;
                ensure!(
                    digest == expected,
                    "published NVMe image differs from its staged bytes"
                );
                Ok(())
            },
        )?;
        Ok(())
    })();
    crate::mutate::workspace::finish_staged_operation(
        parent,
        &staging,
        "NVMe staging image",
        result,
    )
}

pub(super) fn write_nvme_namespace(file: &mut File) -> Result<()> {
    let zeros = [0u8; STORAGE_ZERO_CHUNK_BYTES];
    let mut remaining = NVME_BYTES;
    while remaining != 0 {
        let take = remaining.min(zeros.len() as u64) as usize;
        file.write_all(&zeros[..take])
            .context("streaming zeroed NVMe image bytes")?;
        remaining -= take as u64;
    }
    for lba in 0..NVME_SIGNED_BLOCKS {
        Seek::seek(file, SeekFrom::Start(lba * NVME_BLOCK_BYTES as u64))
            .context("seeking to an NVMe signature block")?;
        file.write_all(&nvme_signature(lba).to_le_bytes())
            .context("writing an NVMe signature block")?;
    }
    let volume_start = NVME_VOLUME_SECTOR * NVME_BLOCK_BYTES as u64;
    {
        let mut volume = FileWindow::new(
            file,
            volume_start,
            crate::image::corpus::VOLUME_BYTES as u64,
        )?;
        crate::image::corpus::populate(&mut volume).context("building the corpus volume")?;
    }
    ensure!(
        Seek::seek(file, SeekFrom::End(0)).context("checking the NVMe image length")? == NVME_BYTES,
        "streamed NVMe image has the wrong length"
    );
    Ok(())
}

#[derive(Debug)]
struct FileWindow<'file> {
    file: &'file mut File,
    start: u64,
    len: u64,
    position: u64,
}

impl<'file> FileWindow<'file> {
    fn new(file: &'file mut File, start: u64, len: u64) -> Result<Self> {
        ensure!(
            start.checked_add(len) == Some(NVME_BYTES),
            "NVMe volume window does not cover the exact image tail"
        );
        Ok(Self {
            file,
            start,
            len,
            position: 0,
        })
    }

    fn position_file(&mut self) -> std::io::Result<()> {
        Seek::seek(&mut *self.file, SeekFrom::Start(self.start + self.position)).map(drop)
    }
}

impl Read for FileWindow<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.position == self.len {
            return Ok(0);
        }
        self.position_file()?;
        let take = buffer.len().min((self.len - self.position) as usize);
        let read = Read::read(&mut *self.file, &mut buffer[..take])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Write for FileWindow<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self.position == self.len && !buffer.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "write escaped the NVMe volume window",
            ));
        }
        self.position_file()?;
        let take = buffer.len().min((self.len - self.position) as usize);
        let written = Write::write(&mut *self.file, &buffer[..take])?;
        self.position += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Write::flush(&mut *self.file)
    }
}

impl Seek for FileWindow<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let (base, offset) = match position {
            SeekFrom::Start(offset) => (0i128, i128::from(offset)),
            SeekFrom::Current(offset) => (i128::from(self.position), i128::from(offset)),
            SeekFrom::End(offset) => (i128::from(self.len), i128::from(offset)),
        };
        let Some(position) = base.checked_add(offset) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "NVMe volume seek overflowed",
            ));
        };
        if position < 0 || position > i128::from(self.len) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "NVMe volume seek escaped its window",
            ));
        }
        self.position = position as u64;
        Ok(self.position)
    }
}

pub(crate) fn nvme_path(root: &Path) -> PathBuf {
    root.join("target")
        .join("qemu storage,private")
        .join("nvme.img")
}

/// The first controller has no namespace, so the kernel must retire it before using its
/// sibling.
pub(super) fn storage(root: &Path) -> Result<Vec<OsString>> {
    let backing = nvme_image(root)?;
    let [drive_flag, drive_spec] = snapshot_drive_file("if=none,id=wizardnvm,format=raw", &backing);
    Ok(vec![
        OsString::from("-device"),
        OsString::from("nvme,serial=wizard-empty"),
        drive_flag,
        drive_spec,
        OsString::from("-device"),
        OsString::from("nvme,serial=wizard0,drive=wizardnvm"),
    ])
}
