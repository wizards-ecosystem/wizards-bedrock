//! Locating and launching QEMU.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use panic_record::{COMPLETE_MARKER, OPEN_PREFIX};

use crate::paths::Paths;
use crate::qmp::{NOT_CANCELLED, Qmp};

/// Default install location on Windows, checked before `PATH`.
const WINDOWS_DEFAULT: &str = r"C:\Program Files\qemu";

/// The q35 machine type shared by both runners.
pub(crate) const MACHINE_TYPE: &str = "pc-q35-9.2";

/// Whether the fixed machine presents the DMA-remapping unit storage requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Remapping {
    /// The ordinary machine: an Intel unit ahead of every PCI device (D-357).
    Present,
    /// The degraded machine that proves storage fails closed without one.
    Absent,
}

/// The remapping unit owns both DMA and interrupt delivery. Its empty IR table
/// blocks every PCI interrupt while caching mode makes the unit cache
/// not-present DMA entries, so every mapping change must be invalidated.
const IOMMU_DEVICE: [&str; 2] = ["-device", "intel-iommu,intremap=on,eim=on,caching-mode=on"];

/// The fixed machine profile, with or without the remapping unit.
///
/// Pins the CPU, TCG, and ambient configuration so protection checks cannot
/// pass under a weaker or different execution model (D-18). The unit comes
/// ahead of the bridge and the storage devices, which QEMU requires of it.
fn machine_args(remapping: Remapping) -> Vec<&'static str> {
    let mut arguments = vec![
        "-no-user-config",
        "-accel",
        "tcg",
        "-machine",
        MACHINE_TYPE,
        "-cpu",
        "max,hypervisor=on,tcg-cpuid=on",
        "-m",
        "512M",
    ];
    if remapping == Remapping::Present {
        arguments.extend(IOMMU_DEVICE);
    }
    // Provide a downstream bus so a bus-zero-only scan fails as it would on
    // VMware's bridged topology (D-168).
    arguments.extend(["-device", "pci-bridge,id=behind,chassis_nr=1"]);
    arguments
}

fn validate_run_arguments(extra: &[OsString]) -> Result<()> {
    for argument in extra {
        let argument = argument.to_string_lossy();
        let option = argument
            .split_once('=')
            .map_or(argument.as_ref(), |(name, _)| name);
        if matches!(
            option,
            "-accel"
                | "--accel"
                | "-enable-kvm"
                | "--enable-kvm"
                | "-machine"
                | "--machine"
                | "-M"
                | "-cpu"
                | "--cpu"
                | "-global"
                | "--global"
                | "-readconfig"
                | "--readconfig"
        ) {
            bail!("QEMU argument {argument:?} conflicts with the fixed TCG machine profile");
        }
    }
    Ok(())
}

/// Subdirectories of the QEMU installation that may hold firmware images.
const FIRMWARE_DIRS: &[&str] = &["share", "share/qemu", "share/OVMF", "."];

/// The UEFI firmware image, mapped read-only.
const CODE_FIRMWARE: &str = "edk2-x86_64-code.fd";

/// Template for the writable UEFI variable store. The i386 naming is genuine
/// QEMU packaging, not a mistake.
const VARS_FIRMWARE: &str = "edk2-i386-vars.fd";

/// Resolved emulator and firmware inputs used by every QEMU runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Installation {
    pub(crate) emulator: PathBuf,
    pub(crate) code_firmware: PathBuf,
    pub(crate) vars_firmware: PathBuf,
}

/// Append the platform's executable suffix to `stem`.
pub fn exe_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

/// Find `exe` in `dirs`.
pub fn which_in<I: IntoIterator<Item = PathBuf>>(dirs: I, exe: &str) -> Option<PathBuf> {
    dirs.into_iter()
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
}

/// Locate the QEMU installation directory.
///
/// `QEMU` names the emulator binary outright; `QEMU_DIR` names the installation
/// it lives in. Both win over the default location and over `PATH`.
pub fn locate() -> Result<PathBuf> {
    if let Some(binary) = std::env::var_os("QEMU").map(PathBuf::from) {
        return binary
            .parent()
            .map(Path::to_path_buf)
            .with_context(|| format!("QEMU={} has no parent directory", binary.display()));
    }
    if let Some(dir) = std::env::var_os("QEMU_DIR").map(PathBuf::from) {
        return Ok(dir);
    }

    let emulator = exe_name("qemu-system-x86_64");
    let default = PathBuf::from(WINDOWS_DEFAULT);
    if default.join(&emulator).is_file() {
        return Ok(default);
    }

    let path = std::env::var_os("PATH").unwrap_or_default();
    match which_in(std::env::split_paths(&path), &emulator) {
        Some(found) => found
            .parent()
            .map(Path::to_path_buf)
            .with_context(|| format!("{} has no parent directory", found.display())),
        None => bail!("QEMU not found: set QEMU_DIR, or put {emulator} on PATH"),
    }
}

fn firmware(qemu_dir: &Path, name: &str) -> Result<PathBuf> {
    FIRMWARE_DIRS
        .iter()
        .map(|dir| qemu_dir.join(dir).join(name))
        .find(|candidate| candidate.is_file())
        .with_context(|| {
            format!(
                "UEFI firmware {name} not found under {}",
                qemu_dir.display()
            )
        })
}

/// Resolve the exact emulator and pristine firmware inputs a boot will use.
pub(crate) fn installation() -> Result<Installation> {
    let directory = locate()?;
    Ok(Installation {
        emulator: directory.join(exe_name("qemu-system-x86_64")),
        code_firmware: firmware(&directory, CODE_FIRMWARE)?,
        vars_firmware: firmware(&directory, VARS_FIRMWARE)?,
    })
}

/// Boot `image` in QEMU with non-profile `extra` arguments.
pub fn run(paths: &Paths, image: &Path, extra: &[OsString]) -> Result<()> {
    validate_run_arguments(extra)?;
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;

    println!("[xtask] launching QEMU (serial on stdio, close the window to exit)");

    let status = Command::new(&installation.emulator)
        .args(machine_args(Remapping::Present))
        .args(["-serial", "stdio"])
        .args(drive_file(
            "if=pflash,format=raw,readonly=on",
            &installation.code_firmware,
        ))
        .args(drive_file("if=pflash,format=raw", vars.path()))
        .args(snapshot_drive_file("format=raw", image))
        .args(storage(paths.root())?)
        .args(extra)
        .status()
        .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

    // A non-zero status here usually means the user closed the window, which is
    // how you quit QEMU — reporting it as a build failure would be a lie.
    if !status.success() {
        println!("[xtask] QEMU exited with {status}");
    }
    Ok(())
}

/// One paused, private QEMU machine owned by a debugger invocation.
pub(crate) struct DebugMachine {
    child: Option<Child>,
    endpoint: String,
    _qmp: Qmp,
    _vars: RunVars,
}

impl DebugMachine {
    /// Start the machine and return the dynamically allocated GDB endpoint.
    pub(crate) fn start(paths: &Paths, image: &Path) -> Result<Self> {
        let installation = installation()?;
        let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;

        let mut child = Command::new(&installation.emulator)
            .args(machine_args(Remapping::Present))
            .args(["-S", "-display", "none", "-serial", "null", "-no-reboot"])
            .args(QMP_ON_STDIO)
            .args(drive_file(
                "if=pflash,format=raw,readonly=on",
                &installation.code_firmware,
            ))
            .args(drive_file("if=pflash,format=raw", vars.path()))
            .args(snapshot_drive_file("format=raw", image))
            .args(storage(paths.root())?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

        let started = (|| -> Result<(Qmp, String)> {
            let deadline = Instant::now() + DEBUG_CONTROL_TIMEOUT;
            let mut qmp = attach_qmp(&mut child, deadline, &NOT_CANCELLED)
                .context("negotiating the paused machine's QMP session")?;
            qmp.human("gdbserver tcp:127.0.0.1:0", deadline, &NOT_CANCELLED)
                .context("starting QEMU's dynamic GDB server")?;
            let report = qmp
                .human("info chardev", deadline, &NOT_CANCELLED)
                .context("querying QEMU's dynamic GDB endpoint")?;
            Ok((qmp, parse_debug_endpoint(&report)?))
        })();

        match started {
            Ok((qmp, endpoint)) => Ok(Self {
                child: Some(child),
                endpoint,
                _qmp: qmp,
                _vars: vars,
            }),
            Err(error) => {
                let cleanup = stop_qemu(&mut child);
                match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup) => {
                        Err(anyhow!("{error:#}; QEMU cleanup also failed: {cleanup:#}"))
                    }
                }
            }
        }
    }

    /// Loopback address for GDB's `target remote` command.
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Stop and reap the machine before its private state is removed.
    pub(crate) fn stop(&mut self) -> Result<()> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        stop_qemu(child)?;
        self.child = None;
        Ok(())
    }
}

impl Drop for DebugMachine {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = stop_qemu(child);
        }
    }
}

const DEBUG_CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const DEBUG_ENDPOINT_PREFIX: &str = "tcp:127.0.0.1:";

fn parse_debug_endpoint(report: &str) -> Result<String> {
    let line = report
        .lines()
        .find(|line| line.trim_start().starts_with("gdb:"))
        .context("QEMU reported no GDB chardev")?;
    let tail = line
        .split_once(DEBUG_ENDPOINT_PREFIX)
        .map(|(_, tail)| tail)
        .context("QEMU's GDB chardev is not bound to IPv4 loopback")?;
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    let port = <u16 as core::str::FromStr>::from_str(&digits)
        .context("QEMU reported an invalid GDB port")?;
    if port == 0 {
        bail!("QEMU did not allocate a GDB port");
    }
    Ok(format!("127.0.0.1:{port}"))
}

fn drive_file(options: &str, path: &Path) -> [OsString; 2] {
    let escaped = path.to_string_lossy().replace(',', ",,");
    [
        OsString::from("-drive"),
        OsString::from(format!("{options},file={escaped}")),
    ]
}

fn snapshot_drive_file(options: &str, path: &Path) -> [OsString; 2] {
    drive_file(&format!("{options},snapshot=on"), path)
}

static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

/// Create a fresh file in the private QEMU run directory; a name another run
/// already holds is skipped, never reused.
fn reserve_run_file(
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

struct RunVars {
    path: PathBuf,
}

impl RunVars {
    fn create(target: &Path, template: &Path) -> Result<Self> {
        Self::create_with(target, template, |_| {})
    }

    fn create_with(
        target: &Path,
        template: &Path,
        after_reservation: impl Fn(&Path),
    ) -> Result<Self> {
        let (directory, path, mut destination) = reserve_run_file(target, "efi-vars", "fd")?;
        after_reservation(&path);
        let copied = (|| -> Result<()> {
            let mut source =
                crate::mutation_workspace::open_real_file(template, "QEMU variable firmware")?;
            std::io::copy(&mut source, &mut destination)
                .with_context(|| format!("copying {} to {}", template.display(), path.display()))?;
            destination
                .sync_all()
                .with_context(|| format!("flushing {}", path.display()))?;
            drop(destination);
            crate::mutation_workspace::require_direct_file(
                &directory,
                &path,
                "private QEMU variable store",
            )?;
            ensure!(
                crate::mutation_workspace::canonical_existing_real_directory(
                    &directory,
                    "QEMU private run directory",
                )? == directory,
                "QEMU private run directory changed while copying firmware"
            );
            let source = crate::sha256::digest_reader(crate::mutation_workspace::open_real_file(
                template,
                "QEMU variable firmware",
            )?)
            .with_context(|| format!("hashing private firmware {}", template.display()))?;
            let copied = crate::sha256::digest_reader(crate::mutation_workspace::open_real_file(
                &path,
                "private QEMU variable store",
            )?)
            .with_context(|| format!("hashing private firmware copy {}", path.display()))?;
            ensure!(copied == source, "private firmware copy changed bytes");
            Ok(())
        })();
        crate::mutation_workspace::finish_staged_operation(
            &directory,
            &path,
            "private QEMU variable store",
            copied,
        )?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RunVars {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Bytes of one logical block on the NVMe namespace.
///
/// QEMU's controller formats at 512 and the kernel reads what `Identify`
/// reports rather than this; the constant is here because the *content* below
/// is written a block at a time and has to agree with what the driver will
/// see.
pub const NVME_BLOCK_BYTES: usize = 512;

/// Blocks given known content.
pub const NVME_SIGNED_BLOCKS: u64 = 8;

/// The first eight bytes of block `lba`.
///
/// **Different in every block, on purpose.** A disk of zeros cannot tell a read
/// that happened from one that did not, and a disk with the same bytes
/// everywhere cannot tell a read of block three from a read of block zero — so
/// a driver that ignored the logical block address would pass. This is D-155's
/// rule applied to a corpus rather than to an argument (D-169).
pub const fn nvme_signature(lba: u64) -> u64 {
    0x5749_5A44_0000_0000 | lba
}

/// Where the FAT32 volume begins on the namespace, in sectors.
///
/// A megabyte in, which is where a partition table would put it. The reader
/// takes a base sector rather than assuming zero for exactly this reason: a
/// volume is rarely the first thing on a disk, and a reader that could not be
/// given an offset is one nothing can be put in front of (D-170).
pub const NVME_VOLUME_SECTOR: u64 = 2048;

/// Bytes of the NVMe backing file.
///
/// The first megabyte carries signed blocks the block-layer checks read, and
/// the FAT32 corpus follows it. Two things on one namespace rather than two
/// namespaces, because that is also what a real disk looks like.
const NVME_BYTES: u64 =
    NVME_VOLUME_SECTOR * NVME_BLOCK_BYTES as u64 + crate::image::corpus::VOLUME_BYTES as u64;

/// Build the streamed NVMe namespace shared by the QEMU and VMware runners.
///
/// The namespace is machine state rather than boot-image content; its distinct
/// block signatures and FAT corpus expose wrong-block and missing-read paths.
///
/// # Errors
///
/// If the file cannot be written, or the corpus cannot be built.
pub fn nvme_image(root: &Path) -> Result<std::path::PathBuf> {
    let (parent, backing) = storage_paths(root)?;
    publish_namespace(&parent, &backing, write_nvme_namespace)?;
    Ok(backing)
}

fn storage_paths(root: &Path) -> Result<(PathBuf, PathBuf)> {
    let backing = nvme_path(root);
    let parent = backing
        .parent()
        .context("NVMe image has no parent")?
        .to_path_buf();
    crate::mutation_workspace::prepare_real_directory_tree(&parent, "QEMU storage directory")?;
    Ok((parent, backing))
}

const STORAGE_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const STORAGE_STAGING_NAME: &str = "nvme.img.incomplete";
const STORAGE_ZERO_CHUNK_BYTES: usize = 64 * 1024;

fn acquire_storage_lock(parent: &Path, path: &Path, canonical_parent: &Path) -> Result<File> {
    acquire_storage_lock_with(parent, path, canonical_parent, || {})
}

fn acquire_storage_lock_with(
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

fn open_storage_lock_with(
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
        crate::mutation_workspace::canonical_existing_real_directory(
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
    let present = crate::mutation_workspace::require_direct_file_if_present(
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

fn publish_namespace(
    parent: &Path,
    backing: &Path,
    write_expected: impl FnOnce(&mut File) -> Result<()>,
) -> Result<()> {
    ensure!(
        backing == parent.join("nvme.img"),
        "NVMe image is not the exact allowlisted storage path"
    );
    let canonical_parent = crate::mutation_workspace::canonical_existing_real_directory(
        parent,
        "QEMU storage directory",
    )?;
    let lock_path = parent.join("nvme.lock");
    let _lock = acquire_storage_lock(parent, &lock_path, &canonical_parent)?;
    crate::mutation_workspace::require_direct_file_if_present(
        parent,
        backing,
        "existing NVMe image",
    )?;

    let staging = parent.join(STORAGE_STAGING_NAME);
    if crate::mutation_workspace::require_direct_file_if_present(
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

        crate::mutation_workspace::require_direct_file(parent, &staging, "NVMe staging image")?;
        ensure!(
            crate::mutation_workspace::canonical_existing_real_directory(
                parent,
                "QEMU storage directory",
            )? == canonical_parent,
            "QEMU storage directory changed during image construction"
        );
        let expected = crate::sha256::digest_reader(crate::mutation_workspace::open_real_file(
            &staging,
            "NVMe staging image",
        )?)
        .with_context(|| format!("hashing NVMe staging image {}", staging.display()))?;

        crate::mutation_workspace::publish_staged_file(
            parent,
            &staging,
            backing,
            "NVMe image",
            |published| {
                let digest = crate::sha256::digest_reader(
                    crate::mutation_workspace::open_real_file(published, "published NVMe image")?,
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
    crate::mutation_workspace::finish_staged_operation(
        parent,
        &staging,
        "NVMe staging image",
        result,
    )
}

fn write_nvme_namespace(file: &mut File) -> Result<()> {
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

struct FileWindow<'a> {
    file: &'a mut File,
    start: u64,
    len: u64,
    position: u64,
}

impl<'a> FileWindow<'a> {
    fn new(file: &'a mut File, start: u64, len: u64) -> Result<Self> {
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

/// A storage controller for QEMU to find, and the file behind it.
fn storage(root: &Path) -> Result<Vec<OsString>> {
    let backing = nvme_image(root)?;
    let [drive_flag, drive_spec] = snapshot_drive_file("if=none,id=wizardnvm,format=raw", &backing);
    Ok(vec![
        // First in PCI enumeration order and deliberately namespace-less: the
        // kernel must retire it before selecting the usable sibling (D-199).
        OsString::from("-device"),
        OsString::from("nvme,serial=wizard-empty"),
        drive_flag,
        drive_spec,
        OsString::from("-device"),
        OsString::from("nvme,serial=wizard0,drive=wizardnvm"),
    ])
}

/// QEMU exits with `(value << 1) | 1`, so the kernel's 0x10 arrives as 33 and
/// its 0x11 as 35. Kept in step with `kernel/src/arch/x86_64/debug_exit.rs`.
const SELFTEST_PASSED: i32 = 33;
const SELFTEST_FAILED: i32 = 35;

/// The serial verdict contract shared with `kernel/src/selftest.rs`.
pub(crate) const SELFTEST_VERDICT_PREFIX: &str = "selftest: verdict ";
/// What the kernel prints around a failing check's name, from
/// `kernel/src/selftest.rs`'s `error!("selftest: {name} FAILED")`.
const SELFTEST_CHECK_PREFIX: &str = "selftest: ";
const SELFTEST_CHECK_FAILED: &str = " FAILED";
const SELFTEST_VERDICT_VERSION: &str = "v1";

const QEMU_TCG_ISOLATION_MARKER: &str = "cpu: user isolation shared kernel mappings: qemu_tcg";
/// Checks the self-test kernel must report, held independently of its own
/// tally so a deleted or skipped one turns the verdict red.
/// Changes require mutation evidence (D-28, D-119).
const EXPECTED_SELFTEST_CHECKS: usize = 281;

/// Prefix of the strict performance/resource record emitted by self-test.
const BASELINE_PREFIX: &str = "selftest: baseline ";

/// Prefix and bounds of the painted NVMe candidate-stack record.
const NVME_STACK_PREFIX: &str = "selftest: nvme-stack ";
const NVME_STACK_CAPACITY: u64 = 32 * 1024;
const NVME_STACK_MIN_HEADROOM: u64 = 2 * 4096;

/// The worker has submitted an NVMe command but remains preemptible.
const STORAGE_DELAY_MARKER: &str = "selftest: awaiting storage-delay keystroke";

/// Line the self-test kernel prints immediately before its bounded wait for
/// an injected keystroke. Kept in step with `kernel/src/selftest.rs`.
const READY_MARKER: &str = "selftest: awaiting injected keystroke";

/// Line the self-test prints before its second wait, with the controller made
/// to translate. Kept in step with `kernel/src/selftest.rs`.
///
/// A second round reaches the distinct set-1 translation path (D-179).
const TRANSLATED_MARKER: &str = "selftest: awaiting translated keystroke";

/// Line the shipped kernel prints once it has finished booting and is doing
/// the one thing it does. Kept in step with `kernel/src/main.rs`.
const SETTLED_MARKER: &str = "kernel: reading the keyboard, uptime ";

/// Proof that the shipped selector retired QEMU's first storage candidate.
pub const NVME_FALLBACK_MARKER: &str = "nvme: candidate 1 retired cleanly; trying next";

const SHIPPED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[
    RequiredLogRecord::info(QEMU_TCG_ISOLATION_MARKER),
    RequiredLogRecord::info(NVME_FALLBACK_MARKER),
    RequiredLogRecord::info(
        "keyboard: PCI interrupt isolation keeps every io-apic line masked; polling",
    ),
    RequiredLogRecord::info("keyboard: polled every 10 ms"),
];

/// A shipped kernel must accept this input without unmasking legacy IRQ1.
const SHIPPED_CHALLENGE_QCODE: &str = "a";
const SHIPPED_CHALLENGE_TIMEOUT: Duration = Duration::from_secs(5);
const SHIPPED_CHALLENGE_WINDOW: Duration = Duration::from_millis(250);

/// What a kernel that has given up says. Watched for rather than inferred
/// from an exit code: a panicking kernel halts, it does not exit.
/// How a red self-test reports itself, and how a dead one does.
///
/// Named rather than written twice because `cargo xtask mutate` has to tell a
/// kernel that *noticed* a deliberate defect from one that would not build —
/// the first is the mutation being caught and the second proves nothing
/// (D-119).
pub const SELFTEST_FAILURE: &str = "self-test reported failures - see the log above";
pub const PANIC_FAILURE: &str = "the self-test kernel panicked: see the log above";

/// A complete, structurally valid self-test machine outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelftestOutcome {
    /// Every registered check passed.
    Passed,
    /// The final verdict failed and names each red check in report order.
    Failed(Vec<String>),
    /// The kernel stopped before a verdict and carries its exact serial panic
    /// transcript plus every check that had already gone red.
    Panicked {
        /// The complete single-line panic body.
        transcript: String,
        /// Named failures printed before the terminal panic.
        failed: Vec<String>,
    },
}

/// A complete degraded-boot outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DegradedOutcome {
    /// Every required refusal and fallback marker preceded a settled boot.
    Settled,
    /// The kernel stopped before settling.
    Panicked {
        /// The complete single-line panic body.
        transcript: String,
        /// The no-x2APIC refusal appeared before the panic.
        no_x2apic_diagnosed: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerdictStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelftestVerdict {
    status: VerdictStatus,
    passed: usize,
    failed: usize,
    expected: usize,
}

/// One guest-side foundation measurement from a clean self-test boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PerformanceSample {
    pub(crate) boot_ns: u64,
    pub(crate) frames_free: u64,
    pub(crate) heap_slab: u64,
    pub(crate) heap_large: u64,
    pub(crate) heap_live: u64,
    pub(crate) handles_committed: u64,
    pub(crate) handles_live: u64,
    pub(crate) trap_ticks: u64,
    pub(crate) storage_bytes: u64,
    pub(crate) storage_ticks: u64,
    pub(crate) counter_hz: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StackSample {
    used: u64,
    capacity: u64,
    headroom: u64,
    leaf: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SerialObservation {
    verdict: Option<SelftestVerdict>,
    baseline: Option<PerformanceSample>,
    nvme_stack: Option<StackSample>,
    qemu_tcg_isolation: bool,
    /// Ordered failures, so mutation grading cannot accept an unrelated red
    /// (D-134).
    failed: Vec<String>,
    panic: Option<String>,
    panic_complete: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BootSerialObservation {
    panic: Option<String>,
    panic_complete: bool,
    no_x2apic_before_panic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanicTransition {
    Unchanged,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogSeverity {
    Info,
    Warning,
    Error,
}

impl LogSeverity {
    const fn prefix(self) -> &'static str {
        match self {
            Self::Info => "[info ] ",
            Self::Warning => "[warn ] ",
            Self::Error => "[error] ",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RequiredLogRecord {
    severity: LogSeverity,
    message: &'static str,
}

impl RequiredLogRecord {
    const fn info(message: &'static str) -> Self {
        Self {
            severity: LogSeverity::Info,
            message,
        }
    }

    const fn warning(message: &'static str) -> Self {
        Self {
            severity: LogSeverity::Warning,
            message,
        }
    }
}

impl PanicTransition {
    fn publish(self, panicked: &AtomicBool) {
        if self == Self::Completed {
            panicked.store(true, Ordering::Release);
        }
    }
}

fn is_complete_panic_record(line: &str) -> bool {
    terminated_serial_text(line).and_then(|line| line.strip_prefix("[error] "))
        == Some(COMPLETE_MARKER)
}

fn terminated_serial_text(line: &str) -> Option<&str> {
    let line = line.strip_suffix('\n')?;
    let text = line.strip_suffix('\r').unwrap_or(line);
    if text.contains('\r') || text.contains('\n') {
        return None;
    }
    Some(text)
}

fn observe_panic_line(
    panic: &mut Option<String>,
    panic_complete: &mut bool,
    line: &str,
) -> Result<Option<PanicTransition>> {
    if is_complete_panic_record(line) {
        if panic.is_none() {
            bail!("the kernel completed a panic record it never started");
        }
        if *panic_complete {
            bail!("the kernel completed its panic record more than once");
        }
        *panic_complete = true;
        return Ok(Some(PanicTransition::Completed));
    }
    if *panic_complete {
        bail!("the kernel printed serial data after its terminal panic record");
    }
    if panic.is_some() && !*panic_complete {
        bail!("the kernel continued a single-line panic record");
    }
    if line.starts_with(OPEN_PREFIX) {
        let text = terminated_serial_text(line)
            .context("the kernel printed an unterminated panic opening record")?;
        *panic = Some(text.to_string());
        return Ok(Some(PanicTransition::Unchanged));
    }
    Ok(None)
}

fn completed_panic(panic: Option<String>, panic_complete: bool) -> Result<Option<String>> {
    match (panic, panic_complete) {
        (None, false) => Ok(None),
        (Some(transcript), true) => Ok(Some(transcript)),
        (Some(_), false) => bail!("QEMU stopped with an incomplete serial panic record"),
        (None, true) => bail!("QEMU observed a panic completion without its opening record"),
    }
}

fn is_exact_no_x2apic_diagnosis(line: &str) -> bool {
    is_exact_log_message(line, LogSeverity::Warning, NO_X2APIC_DIAGNOSIS)
}

fn observe_boot_line(
    observation: &mut BootSerialObservation,
    line: &str,
) -> Result<PanicTransition> {
    if let Some(transition) = observe_panic_line(
        &mut observation.panic,
        &mut observation.panic_complete,
        line,
    )? {
        return Ok(transition);
    }
    if is_exact_no_x2apic_diagnosis(line) {
        observation.no_x2apic_before_panic = true;
    }
    Ok(PanicTransition::Unchanged)
}

fn parse_failed_check(line: &str) -> Option<&str> {
    let message = log_message(line, LogSeverity::Error)?;
    let name = message
        .strip_prefix(SELFTEST_CHECK_PREFIX)?
        .strip_suffix(SELFTEST_CHECK_FAILED)?;
    (!name.is_empty()).then_some(name)
}

fn is_exact_log_message(line: &str, severity: LogSeverity, expected: &str) -> bool {
    log_message(line, severity) == Some(expected)
}

fn log_message(line: &str, severity: LogSeverity) -> Option<&str> {
    terminated_serial_text(line).and_then(|line| line.strip_prefix(severity.prefix()))
}

fn is_required_boot_marker(line: &str, record: RequiredLogRecord) -> bool {
    if record == NO_X2APIC_RECORD {
        return is_exact_no_x2apic_diagnosis(line);
    }
    is_exact_log_message(line, record.severity, record.message)
}

pub(crate) fn canonical_decimal(text: &str) -> Option<u64> {
    if text.is_empty()
        || (text.len() > 1 && text.starts_with('0'))
        || !text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    <u64 as core::str::FromStr>::from_str(text).ok()
}

fn is_settled_record(line: &str) -> bool {
    let Some(message) = log_message(line, LogSeverity::Info) else {
        return false;
    };
    message
        .strip_prefix(SETTLED_MARKER)
        .and_then(|uptime| uptime.strip_suffix(" ms"))
        .and_then(canonical_decimal)
        .is_some()
}

// Cover ordinary input and both editing paths (D-51).
const INJECTED_QCODES: &[&str] = &["a", "backspace", "ret"];

const STORAGE_DELAY_QCODES: &[&str] = &["c"];

// A distinct second key prevents a queued first-round key satisfying the
// translated-path check (D-155, D-179).
const TRANSLATED_QCODES: &[&str] = &["b"];

const INTERACTIVE_MARKERS: &[RequiredLogRecord] = &[
    RequiredLogRecord::info(STORAGE_DELAY_MARKER),
    RequiredLogRecord::info(READY_MARKER),
    RequiredLogRecord::info(TRANSLATED_MARKER),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractiveMarkerOrder {
    Strict,
    Mutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectionAction {
    Wait,
    Inject,
    Skip,
}

const fn injection_action(observed: usize, round: usize) -> InjectionAction {
    if observed < round {
        InjectionAction::Wait
    } else if observed == round {
        InjectionAction::Inject
    } else {
        InjectionAction::Skip
    }
}

fn observe_interactive_marker(
    line: &str,
    next: &mut usize,
    order: InteractiveMarkerOrder,
) -> Result<bool> {
    let Some(index) = INTERACTIVE_MARKERS
        .iter()
        .position(|record| is_required_boot_marker(line, *record))
    else {
        return Ok(false);
    };
    let ordered = match order {
        InteractiveMarkerOrder::Strict => index == *next,
        InteractiveMarkerOrder::Mutation => index >= *next,
    };
    ensure!(
        ordered,
        "self-test interactive marker {} arrived while waiting for {}",
        index + 1,
        *next + 1
    );
    *next = index + 1;
    Ok(true)
}

fn parse_selftest_verdict(line: &str) -> Option<Result<SelftestVerdict>> {
    let (severity, message) = log_message(line, LogSeverity::Info)
        .map(|message| (LogSeverity::Info, message))
        .or_else(|| {
            log_message(line, LogSeverity::Error).map(|message| (LogSeverity::Error, message))
        })?;
    let mut fields = message.strip_prefix(SELFTEST_VERDICT_PREFIX)?.split(' ');
    let version = core::iter::Iterator::next(&mut fields);
    let status = core::iter::Iterator::next(&mut fields);
    let passed = core::iter::Iterator::next(&mut fields);
    let failed = core::iter::Iterator::next(&mut fields);
    let expected = core::iter::Iterator::next(&mut fields);

    let parsed = (|| {
        if version != Some(SELFTEST_VERDICT_VERSION) {
            bail!("unsupported self-test verdict version");
        }
        let status = match status {
            Some("PASS") => VerdictStatus::Pass,
            Some("FAIL") => VerdictStatus::Fail,
            _ => bail!("invalid self-test verdict status"),
        };
        ensure!(
            matches!(
                (severity, status),
                (LogSeverity::Info, VerdictStatus::Pass)
                    | (LogSeverity::Error, VerdictStatus::Fail)
            ),
            "self-test verdict status has the wrong log severity"
        );
        if core::iter::Iterator::next(&mut fields).is_some() {
            bail!("trailing fields in self-test verdict");
        }

        Ok(SelftestVerdict {
            status,
            passed: parse_verdict_count(passed, "passed=")?,
            failed: parse_verdict_count(failed, "failed=")?,
            expected: parse_verdict_count(expected, "expected=")?,
        })
    })();
    Some(parsed)
}

fn parse_verdict_count(field: Option<&str>, name: &str) -> Result<usize> {
    let value = field
        .and_then(|field| field.strip_prefix(name))
        .context("missing self-test verdict count")?;
    usize::try_from(canonical_decimal(value).context("invalid self-test verdict count")?)
        .context("self-test verdict count does not fit usize")
}

fn parse_baseline_value(field: Option<&str>, name: &str) -> Result<u64> {
    let value = field
        .and_then(|field| field.strip_prefix(name))
        .with_context(|| format!("missing baseline field {name}"))?;
    canonical_decimal(value).with_context(|| format!("invalid baseline field {name}"))
}

fn parse_performance_sample(line: &str) -> Option<Result<PerformanceSample>> {
    let message = log_message(line, LogSeverity::Info)?;
    let mut fields = message.strip_prefix(BASELINE_PREFIX)?.split(' ');
    let parsed = (|| {
        if core::iter::Iterator::next(&mut fields) != Some("v1") {
            bail!("unsupported foundation baseline version");
        }
        let sample = PerformanceSample {
            boot_ns: parse_baseline_value(core::iter::Iterator::next(&mut fields), "boot_ns=")?,
            frames_free: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "frames_free=",
            )?,
            heap_slab: parse_baseline_value(core::iter::Iterator::next(&mut fields), "heap_slab=")?,
            heap_large: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "heap_large=",
            )?,
            heap_live: parse_baseline_value(core::iter::Iterator::next(&mut fields), "heap_live=")?,
            handles_committed: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "handles_committed=",
            )?,
            handles_live: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "handles_live=",
            )?,
            trap_ticks: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "trap_ticks=",
            )?,
            storage_bytes: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "storage_bytes=",
            )?,
            storage_ticks: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "storage_ticks=",
            )?,
            counter_hz: parse_baseline_value(
                core::iter::Iterator::next(&mut fields),
                "counter_hz=",
            )?,
        };
        if core::iter::Iterator::next(&mut fields).is_some() {
            bail!("trailing fields in foundation baseline");
        }
        Ok(sample)
    })();
    Some(parsed)
}

fn parse_stack_sample(line: &str) -> Option<Result<StackSample>> {
    let message = log_message(line, LogSeverity::Info)?;
    let mut fields = message.strip_prefix(NVME_STACK_PREFIX)?.split(' ');
    let parsed = (|| {
        if core::iter::Iterator::next(&mut fields) != Some("v2") {
            bail!("unsupported NVMe stack record version");
        }
        let sample = StackSample {
            used: parse_baseline_value(core::iter::Iterator::next(&mut fields), "used=")?,
            capacity: parse_baseline_value(core::iter::Iterator::next(&mut fields), "capacity=")?,
            headroom: parse_baseline_value(core::iter::Iterator::next(&mut fields), "headroom=")?,
            leaf: parse_baseline_value(core::iter::Iterator::next(&mut fields), "leaf=")?,
        };
        if core::iter::Iterator::next(&mut fields).is_some() {
            bail!("trailing fields in NVMe stack record");
        }
        Ok(sample)
    })();
    Some(parsed)
}

fn observe_selftest_line(
    observation: &mut SerialObservation,
    line: &str,
) -> Result<PanicTransition> {
    if observation.verdict.is_some() {
        bail!("the self-test kernel printed serial data after its final verdict");
    }
    if let Some(transition) = observe_panic_line(
        &mut observation.panic,
        &mut observation.panic_complete,
        line,
    )? {
        return Ok(transition);
    }
    if let Some(name) = parse_failed_check(line) {
        if observation.failed.iter().any(|failed| failed == name) {
            bail!("the self-test kernel named one failed check more than once");
        }
        observation.failed.push(name.to_string());
        return Ok(PanicTransition::Unchanged);
    }
    if is_exact_log_message(line, LogSeverity::Info, QEMU_TCG_ISOLATION_MARKER) {
        if observation.qemu_tcg_isolation {
            bail!("the self-test kernel printed the TCG isolation decision more than once");
        }
        observation.qemu_tcg_isolation = true;
        return Ok(PanicTransition::Unchanged);
    }
    if let Some(sample) = parse_performance_sample(line) {
        let sample = sample?;
        if observation.baseline.replace(sample).is_some() {
            bail!("the self-test kernel printed more than one foundation baseline");
        }
        return Ok(PanicTransition::Unchanged);
    }
    if let Some(sample) = parse_stack_sample(line) {
        let sample = sample?;
        if observation.nvme_stack.replace(sample).is_some() {
            bail!("the self-test kernel printed more than one NVMe stack record");
        }
        return Ok(PanicTransition::Unchanged);
    }
    let Some(verdict) = parse_selftest_verdict(line) else {
        return Ok(PanicTransition::Unchanged);
    };
    let verdict = verdict?;
    if observation.verdict.replace(verdict).is_some() {
        bail!("the self-test kernel printed more than one final verdict");
    }
    Ok(PanicTransition::Unchanged)
}

fn correlate_selftest(
    exit_code: Option<i32>,
    observation: SerialObservation,
) -> Result<SelftestOutcome> {
    if let Some(transcript) = completed_panic(observation.panic, observation.panic_complete)? {
        return Ok(SelftestOutcome::Panicked {
            transcript,
            failed: observation.failed,
        });
    }
    let verdict = observation
        .verdict
        .context("QEMU exited without a serial self-test verdict")?;
    if verdict.expected != EXPECTED_SELFTEST_CHECKS {
        bail!(
            "self-test expected {} checks, harness requires {EXPECTED_SELFTEST_CHECKS}",
            verdict.expected
        );
    }
    let checked = verdict
        .passed
        .checked_add(verdict.failed)
        .context("self-test verdict count overflowed")?;
    if checked != verdict.expected || verdict.expected == 0 {
        bail!(
            "self-test verdict counted {checked} checks, expected {}",
            verdict.expected
        );
    }

    let counts_pass = verdict.failed == 0 && verdict.passed == verdict.expected;
    if (verdict.status == VerdictStatus::Pass) != counts_pass {
        bail!("self-test serial status contradicts its counts");
    }
    if observation.failed.len() != verdict.failed {
        bail!(
            "self-test verdict counted {} failures but named {}",
            verdict.failed,
            observation.failed.len()
        );
    }
    if verdict.status == VerdictStatus::Pass && !observation.qemu_tcg_isolation {
        bail!("self-test passed without consuming the exact QEMU TCG isolation policy");
    }
    if verdict.status == VerdictStatus::Pass {
        let sample = observation
            .nvme_stack
            .context("self-test passed without an NVMe stack record")?;
        ensure!(
            sample.capacity == NVME_STACK_CAPACITY,
            "NVMe stack record named a {}-byte capacity, expected {NVME_STACK_CAPACITY}",
            sample.capacity
        );
        ensure!(
            sample.used.checked_add(sample.headroom) == Some(sample.capacity),
            "NVMe stack record does not conserve its capacity"
        );
        ensure!(
            sample.leaf <= sample.used,
            "NVMe stack record's live-leaf use {} exceeds its painted high-water mark {}",
            sample.leaf,
            sample.used
        );
        ensure!(
            sample.headroom >= NVME_STACK_MIN_HEADROOM,
            "NVMe candidates retained only {} bytes of stack headroom",
            sample.headroom
        );
    }

    match (exit_code, verdict.status) {
        (Some(SELFTEST_PASSED), VerdictStatus::Pass) => Ok(SelftestOutcome::Passed),
        (Some(SELFTEST_FAILED), VerdictStatus::Fail) => {
            Ok(SelftestOutcome::Failed(observation.failed))
        }
        (Some(SELFTEST_PASSED | SELFTEST_FAILED), _) => {
            bail!("self-test serial verdict contradicts its debug-exit status")
        }
        (Some(other), _) => bail!("QEMU exited with {other}; debug-exit gave no verdict"),
        (None, _) => bail!("QEMU was terminated by a signal"),
    }
}

// Registers are the only hang evidence when the serial log is empty.
fn report_hang(qmp: Option<&mut Qmp>) {
    let Some(qmp) = qmp else {
        println!("[xtask] no QMP session: where the kernel hung is unknown");
        return;
    };
    let deadline = Instant::now() + MONITOR_TIMEOUT;
    let Ok(state) = qmp.human("info registers", deadline, &NOT_CANCELLED) else {
        println!("[xtask] no monitor reply: where the kernel hung is unknown");
        return;
    };
    for line in state
        .lines()
        .filter(|line| line.contains("RIP=") || line.contains("RSP="))
    {
        println!("[xtask] hung at: {}", line.trim());
    }
    let Some(rip) = state
        .split_whitespace()
        .find_map(|word| word.strip_prefix("RIP="))
    else {
        return;
    };
    let Ok(code) = qmp.human(&format!("x/8i 0x{rip}"), deadline, &NOT_CANCELLED) else {
        return;
    };
    for line in code
        .lines()
        .filter(|line| line.trim_start().starts_with("0x"))
    {
        println!("[xtask] hung in:  {}", line.trim());
    }
}

/// QMP on the child's own standard streams, which only the harness holds
/// (D-346). Serial output therefore uses a separate private OS transport.
const QMP_ON_STDIO: [&str; 2] = ["-qmp", "stdio"];

/// Negotiate QMP over the piped standard streams of a freshly spawned QEMU.
fn attach_qmp(child: &mut Child, deadline: Instant, cancelled: &AtomicBool) -> Result<Qmp> {
    let input = child.stdin.take().context("taking QEMU's stdin pipe")?;
    let output = child.stdout.take().context("taking QEMU's stdout pipe")?;
    Qmp::negotiate(input, output, deadline, cancelled)
}

/// One run's private, backpressured serial transport.
///
/// Windows QEMU creates a duplex named pipe. Its default DACL and a fresh
/// 256-bit name keep the endpoint private to this run; the kernel's output
/// can fill only the operating system's bounded pipe buffer. Unix QEMU uses
/// a socket beneath the private run directory. Neither route writes guest
/// bytes to an unbounded host file (D-367).
struct SerialChannel {
    #[cfg(windows)]
    name: String,
    #[cfg(unix)]
    socket: PathBuf,
    #[cfg(unix)]
    reservation: PathBuf,
    closed: Arc<AtomicBool>,
}

impl SerialChannel {
    fn create(target: &Path) -> Result<Self> {
        #[cfg(windows)]
        {
            let _ = target;
            let mut random = [0u8; 32];
            getrandom::fill(&mut random)
                .map_err(|error| anyhow!("obtaining a private QEMU serial-pipe name: {error}"))?;
            let mut suffix = String::with_capacity(random.len() * 2);
            for byte in random {
                use core::fmt::Write as _;
                write!(suffix, "{byte:02x}").expect("writing to a String cannot fail");
            }
            Ok(Self {
                name: format!("wizard-bedrock-serial-{}-{suffix}", std::process::id()),
                closed: Arc::new(AtomicBool::new(false)),
            })
        }

        #[cfg(unix)]
        {
            let (_, reservation, guard) = reserve_run_file(target, "serial", "guard")?;
            drop(guard);
            let socket = reservation.with_extension("sock");
            ensure!(
                !socket.exists(),
                "QEMU serial socket name is already occupied"
            );
            Ok(Self {
                socket,
                reservation,
                closed: Arc::new(AtomicBool::new(false)),
            })
        }

        #[cfg(not(any(windows, unix)))]
        {
            let _ = target;
            bail!("automated QEMU serial transport is unsupported on this host")
        }
    }

    /// The bounded character device that carries the guest's first serial port.
    fn arguments(&self) -> [OsString; 4] {
        #[cfg(windows)]
        let backend = OsString::from(format!("pipe,id=serial0,path={}", self.name));
        #[cfg(unix)]
        let backend = OsString::from(format!(
            "socket,id=serial0,path={},server=on,wait=on",
            self.socket.to_string_lossy().replace(',', ",,")
        ));
        [
            OsString::from("-chardev"),
            backend,
            OsString::from("-serial"),
            OsString::from("chardev:serial0"),
        ]
    }

    /// Connect after QEMU creates its endpoint, refusing an exited or silent
    /// emulator rather than opening some ambient transport.
    fn connect(&self, child: &mut Child, deadline: Instant) -> Result<SerialReader> {
        loop {
            #[cfg(windows)]
            let opened = OpenOptions::new()
                .read(true)
                .open(format!(r"\\.\pipe\{}", self.name))
                .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>);
            #[cfg(unix)]
            let opened = std::os::unix::net::UnixStream::connect(&self.socket)
                .map(|socket| Box::new(socket) as Box<dyn Read + Send>);
            #[cfg(not(any(windows, unix)))]
            let opened: std::io::Result<Box<dyn Read + Send>> =
                Err(std::io::Error::other("unsupported serial transport"));

            match opened {
                Ok(inner) => {
                    return Ok(SerialReader {
                        inner,
                        closed: Arc::clone(&self.closed),
                    });
                }
                Err(error) => {
                    if let Some(status) = child.try_wait().context("waiting for QEMU serial")? {
                        return Err(error).with_context(|| {
                            format!("QEMU exited before its serial transport opened ({status})")
                        });
                    }
                    if Instant::now() >= deadline {
                        return Err(error).context("QEMU did not open its serial transport");
                    }
                    std::thread::sleep(SERIAL_POLL);
                }
            }
        }
    }

    /// Record that QEMU has exited, so an observed pipe close is clean EOF.
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

impl Drop for SerialChannel {
    fn drop(&mut self) {
        self.close();
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.socket);
            let _ = std::fs::remove_file(&self.reservation);
        }
    }
}

/// The host side of the bounded serial transport.
struct SerialReader {
    inner: Box<dyn Read + Send>,
    closed: Arc<AtomicBool>,
}

impl Read for SerialReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.inner.read(buffer) {
            Ok(0) => await_serial_close(&self.closed),
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
                await_serial_close(&self.closed)
            }
            outcome => outcome,
        }
    }
}

fn await_serial_close(closed: &AtomicBool) -> std::io::Result<usize> {
    let deadline = Instant::now() + SERIAL_CLOSE_GRACE;
    loop {
        if closed.load(Ordering::Acquire) {
            return Ok(0);
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "QEMU closed its serial transport before it exited",
            ));
        }
        std::thread::sleep(SERIAL_POLL);
    }
}

/// How often a serial connector looks for QEMU's freshly created endpoint.
const SERIAL_POLL: Duration = Duration::from_millis(10);
/// Enough for the parent loop to observe a QEMU exit before pipe EOF is judged
/// premature; the parent polls every 50 ms.
const SERIAL_CLOSE_GRACE: Duration = Duration::from_millis(250);

/// One guest record and one complete run are separately bounded. Once the
/// reader refuses the aggregate, OS backpressure bounds any bytes QEMU can
/// still write before the parent terminates it.
const MAX_SERIAL_LINE_BYTES: usize = 64 * 1024;
const MAX_SERIAL_BYTES: usize = 16 * 1024 * 1024;

fn read_serial_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    total: &mut usize,
) -> Result<usize> {
    read_serial_line_with_limits(reader, line, total, MAX_SERIAL_LINE_BYTES, MAX_SERIAL_BYTES)
}

fn read_serial_line_with_limits(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    total: &mut usize,
    line_limit: usize,
    total_limit: usize,
) -> Result<usize> {
    line.clear();
    loop {
        let available = reader.fill_buf().context("buffering QEMU serial output")?;
        if available.is_empty() {
            return Ok(line.len());
        }
        let take = available
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(available.len(), |newline| newline + 1);
        let line_bytes = line
            .len()
            .checked_add(take)
            .context("QEMU serial line length overflowed")?;
        ensure!(
            line_bytes <= line_limit,
            "QEMU serial line exceeded the {line_limit}-byte limit"
        );
        let aggregate = total
            .checked_add(take)
            .context("QEMU serial transcript length overflowed")?;
        ensure!(
            aggregate <= total_limit,
            "QEMU serial transcript exceeded the {total_limit}-byte limit"
        );
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        *total = aggregate;
        if line.last() == Some(&b'\n') {
            return Ok(line.len());
        }
    }
}

/// Render untrusted guest bytes without passing terminal controls through to
/// the operator's console. The parser still receives the original bytes.
fn escaped_serial_line(line: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let visible = line.strip_suffix(b"\n").unwrap_or(line);
    let mut escaped = String::with_capacity(visible.len());
    for &byte in visible {
        if (b' '..=b'~').contains(&byte) {
            escaped.push(char::from(byte));
        } else {
            escaped.push('\\');
            escaped.push('x');
            escaped.push(char::from(HEX[usize::from(byte >> 4)]));
            escaped.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    escaped
}

fn echo_serial_line(line: &[u8]) {
    println!("{}", escaped_serial_line(line));
    let _ = Write::flush(&mut std::io::stdout());
}

/// Long enough for a reply, short enough not to extend a failing run.
const MONITOR_TIMEOUT: Duration = Duration::from_millis(1500);

/// How long QEMU has to greet and accept capabilities after it starts.
const QMP_NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(10);

/// A kernel that neither passes nor fails within `timeout` is a hang, which is
/// itself a failure — so the emulator is killed rather than left running.
///
/// The boot carries a QMP control socket so the harness can press a key when
/// the kernel says it is listening: the harness listens first and QEMU dials
/// out, so there is no port to guess and no connect to race. Serial output is
/// piped and forwarded line by line — both to keep it visible and to watch
/// for the marker. QMP misbehaviour is reported and abandoned, never fatal
/// here: the missing keystroke turns the kernel's own bounded check red, so
/// the verdict stays the kernel's.
pub fn run_selftest(paths: &Paths, image: &Path, timeout: Duration) -> Result<()> {
    let (outcome, _) =
        observe_selftest_run(paths, image, timeout, true, InteractiveMarkerOrder::Strict)?;
    require_selftest(outcome)
}

/// Boot `image` headless and preserve a valid red or panic outcome for a
/// mutation caller. A missing earlier interaction may be skipped so its later
/// exact verdict remains observable; ordinary self-test boots stay strict.
pub fn observe_selftest(paths: &Paths, image: &Path, timeout: Duration) -> Result<SelftestOutcome> {
    observe_selftest_run(
        paths,
        image,
        timeout,
        true,
        InteractiveMarkerOrder::Mutation,
    )
    .map(|(outcome, _)| outcome)
}

/// Boot a self-test image and require its one versioned baseline record.
pub(crate) fn measure_performance(
    paths: &Paths,
    image: &Path,
    timeout: Duration,
) -> Result<PerformanceSample> {
    let (outcome, baseline) =
        observe_selftest_run(paths, image, timeout, false, InteractiveMarkerOrder::Strict)?;
    require_selftest(outcome)?;
    baseline.context("a clean self-test boot emitted no foundation baseline")
}

fn observe_selftest_run(
    paths: &Paths,
    image: &Path,
    timeout: Duration,
    echo_serial: bool,
    marker_order: InteractiveMarkerOrder,
) -> Result<(SelftestOutcome, Option<PerformanceSample>)> {
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;
    let serial = SerialChannel::create(paths.target_dir())?;

    println!("[xtask] booting the self-test kernel");

    let mut child = Command::new(&installation.emulator)
        .args(machine_args(Remapping::Present))
        .args(["-display", "none", "-no-reboot"])
        .args(serial.arguments())
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .args(QMP_ON_STDIO)
        .args(drive_file(
            "if=pflash,format=raw,readonly=on",
            &installation.code_firmware,
        ))
        .args(drive_file("if=pflash,format=raw", vars.path()))
        .args(snapshot_drive_file("format=raw", image))
        .args(storage(paths.root())?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

    let serial_reader = match serial.connect(&mut child, Instant::now() + QMP_NEGOTIATION_TIMEOUT) {
        Ok(reader) => reader,
        Err(error) => {
            stop_qemu(&mut child).context("stopping QEMU after serial connection failed")?;
            return Err(error);
        }
    };

    let markers_seen = Arc::new(AtomicUsize::new(0));
    let panicked = Arc::new(AtomicBool::new(false));
    let serial_failed = Arc::new(AtomicBool::new(false));
    let forwarder = forward_serial(
        serial_reader,
        Arc::clone(&markers_seen),
        Arc::clone(&panicked),
        Arc::clone(&serial_failed),
        echo_serial,
        marker_order,
    );
    let deadline = Instant::now() + timeout;
    let injection_cancelled = Arc::new(AtomicBool::new(false));
    let injector = inject_on_marker(
        &mut child,
        markers_seen,
        deadline,
        Arc::clone(&injection_cancelled),
    )?;
    let status = loop {
        if let Some(status) = child.try_wait().context("waiting for QEMU")? {
            break status;
        }
        // A panicking kernel halts rather than reporting, so the timeout
        // would otherwise be the only way it ever ends.
        if panicked.load(Ordering::Acquire) {
            stop_qemu(&mut child)?;
            serial.close();
            let observation = finish_selftest_serial(forwarder)?;
            finish_injector(injector, &injection_cancelled)?;
            let baseline = observation.baseline;
            return correlate_selftest(None, observation).map(|outcome| (outcome, baseline));
        }
        if serial_failed.load(Ordering::Acquire) {
            stop_qemu(&mut child)?;
            serial.close();
            finish_selftest_serial(forwarder)?;
            finish_injector(injector, &injection_cancelled)?;
            unreachable!("a flagged serial failure returned success");
        }
        if Instant::now() >= deadline {
            let mut qmp = finish_injector(injector, &injection_cancelled)?;
            report_hang(qmp.as_mut());
            stop_qemu(&mut child)?;
            serial.close();
            finish_selftest_serial(forwarder)?;
            bail!(
                "self-test did not finish within {} seconds",
                timeout.as_secs()
            );
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    // Drain serial before correlation so late failure lines are not lost.
    serial.close();
    let observation = finish_selftest_serial(forwarder)?;
    finish_injector(injector, &injection_cancelled)?;

    let baseline = observation.baseline;
    correlate_selftest(status.code(), observation).map(|outcome| (outcome, baseline))
}

fn require_selftest(outcome: SelftestOutcome) -> Result<()> {
    match outcome {
        SelftestOutcome::Passed => {
            println!("[xtask] self-test passed");
            Ok(())
        }
        SelftestOutcome::Failed(failed) => {
            bail!("{SELFTEST_FAILURE}: {}", failed.join("; "))
        }
        SelftestOutcome::Panicked { transcript, .. } => {
            bail!("{PANIC_FAILURE}: {transcript}")
        }
    }
}

/// Forward every serial line to the console, flagging the ready marker.
///
/// A kernel that dies mid-line still shows its last words: the unterminated
/// remainder at end-of-stream is forwarded too.
fn forward_serial(
    serial: impl Read + Send + 'static,
    markers_seen: Arc<AtomicUsize>,
    panicked: Arc<AtomicBool>,
    serial_failed: Arc<AtomicBool>,
    echo: bool,
    marker_order: InteractiveMarkerOrder,
) -> JoinHandle<Result<SerialObservation>> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(serial);
        let mut line = Vec::new();
        let mut total = 0;
        let mut observation = SerialObservation::default();
        let mut next_interactive_marker = 0;
        loop {
            line.clear();
            let read = match read_serial_line(&mut reader, &mut line, &mut total) {
                Ok(read) => read,
                Err(error) => {
                    serial_failed.store(true, Ordering::Release);
                    return Err(error).context("reading QEMU's serial output");
                }
            };
            if read == 0 {
                return Ok(observation);
            }
            let text = String::from_utf8_lossy(&line);
            if echo {
                // One print per line, so a message from another thread cannot
                // land between a dying kernel's last words and their newline.
                echo_serial_line(&line);
            }
            match observe_selftest_line(&mut observation, &text) {
                Ok(transition) => {
                    transition.publish(&panicked);
                    match observe_interactive_marker(
                        &text,
                        &mut next_interactive_marker,
                        marker_order,
                    ) {
                        Ok(true) => markers_seen.store(next_interactive_marker, Ordering::Release),
                        Ok(false) => {}
                        Err(error) => {
                            serial_failed.store(true, Ordering::Release);
                            return Err(error).context("reading interactive serial evidence");
                        }
                    }
                }
                Err(error) => {
                    serial_failed.store(true, Ordering::Release);
                    return Err(error).context("reading the kernel's self-test verdict");
                }
            }
        }
    })
}

fn finish_selftest_serial(
    forwarder: JoinHandle<Result<SerialObservation>>,
) -> Result<SerialObservation> {
    forwarder
        .join()
        .map_err(|_| anyhow!("the self-test serial reader panicked"))?
}

/// Inject each key after its marker without letting a QMP stall delay the boot
/// deadline; every exit path cancels and joins this dedicated thread, which
/// hands its negotiated session back for any later hang report.
fn inject_on_marker(
    child: &mut Child,
    markers_seen: Arc<AtomicUsize>,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
) -> Result<JoinHandle<Option<Qmp>>> {
    let input = child.stdin.take().context("taking QEMU's stdin pipe")?;
    let output = child.stdout.take().context("taking QEMU's stdout pipe")?;
    Ok(std::thread::spawn(move || {
        let mut session = None;
        let outcome = (|| -> Result<usize> {
            let negotiation = deadline.min(Instant::now() + QMP_NEGOTIATION_TIMEOUT);
            let qmp = session.insert(Qmp::negotiate(input, output, negotiation, &cancelled)?);
            let mut injected = 0;
            for (round, keys) in [
                (1, STORAGE_DELAY_QCODES),
                (2, INJECTED_QCODES),
                (3, TRANSLATED_QCODES),
            ] {
                let action = loop {
                    let observed = markers_seen.load(Ordering::Acquire);
                    let action = injection_action(observed, round);
                    if action != InjectionAction::Wait {
                        break action;
                    }
                    if cancelled.load(Ordering::Acquire) {
                        bail!("QMP injection was cancelled");
                    }
                    if Instant::now() >= deadline {
                        bail!("QMP injection reached the run deadline");
                    }
                    std::thread::sleep(POLL_INTERVAL);
                };
                if action == InjectionAction::Skip {
                    continue;
                }
                keys.iter()
                    .try_for_each(|qcode| qmp.inject_key(qcode, deadline, &cancelled))?;
                injected += keys.len();
            }
            Ok(injected)
        })();
        match outcome {
            Ok(injected) => println!("[xtask] qmp: {injected} keystrokes injected"),
            Err(reason) => {
                println!("[xtask] qmp: {reason:#}; the keystroke check will report it");
            }
        }
        session
    }))
}

/// Cancel and join the self-test injector, recovering its QMP session.
fn finish_injector(
    injector: JoinHandle<Option<Qmp>>,
    cancelled: &AtomicBool,
) -> Result<Option<Qmp>> {
    cancelled.store(true, Ordering::Release);
    injector
        .join()
        .map_err(|_| anyhow!("the QMP injector panicked"))
}

/// How often to check whether QEMU has exited. Short enough that a passing run
/// is not padded noticeably, long enough not to spin.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const QEMU_STOP_TIMEOUT: Duration = Duration::from_secs(5);

fn stop_qemu(child: &mut Child) -> Result<()> {
    stop_qemu_with(child, child_exited, terminate_child, QEMU_STOP_TIMEOUT)
}

fn child_exited(child: &mut Child) -> std::io::Result<bool> {
    Child::try_wait(child).map(|status| status.is_some())
}

fn terminate_child(child: &mut Child) -> std::io::Result<()> {
    Child::kill(child)
}

fn stop_qemu_with<C>(
    child: &mut C,
    exited: fn(&mut C) -> std::io::Result<bool>,
    terminate: fn(&mut C) -> std::io::Result<()>,
    timeout: Duration,
) -> Result<()> {
    if exited(child).context("checking whether QEMU exited")? {
        return Ok(());
    }
    if let Err(error) = terminate(child) {
        if exited(child).context("checking QEMU after termination failed")? {
            return Ok(());
        }
        return Err(error).context("terminating QEMU");
    }

    let deadline = Instant::now()
        .checked_add(timeout)
        .context("QEMU stop deadline is not representable")?;
    loop {
        if exited(child).context("polling terminated QEMU")? {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            bail!(
                "QEMU did not exit within {} ms after termination",
                timeout.as_millis()
            );
        }
        std::thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
    }
}

fn irq_count(report: &str, irq: u8) -> Option<u64> {
    let mut recognized = false;
    let mut total = 0u64;
    for line in report.lines() {
        if line.contains("IRQ statistics") {
            recognized = true;
            continue;
        }
        let Some((label, count)) = line.split_once(':') else {
            continue;
        };
        let label = label.trim().strip_prefix("IRQ").unwrap_or(label.trim());
        if <u8 as core::str::FromStr>::from_str(label.trim()).ok() != Some(irq) {
            continue;
        }
        let mut fields = count.split_whitespace();
        let count =
            <u64 as core::str::FromStr>::from_str(core::iter::Iterator::next(&mut fields)?).ok()?;
        total = total.checked_add(count)?;
    }
    recognized.then_some(total)
}

fn keyboard_irq_count(qmp: &mut Qmp) -> Result<u64> {
    let report = qmp
        .human(
            "info irq",
            Instant::now() + SHIPPED_CHALLENGE_TIMEOUT,
            &NOT_CANCELLED,
        )
        .context("querying keyboard IRQ progress")?;
    irq_count(&report, 1).context("QEMU monitor returned no IRQ statistics")
}

/// Boot the image the kernel ships and require it to reach its steady state.
///
/// The self-test build exits at the debug-exit device, so nothing it does
/// covers what the shipped kernel does afterwards. The verdict here is what
/// the serial log says: the settled marker, and no panic.
pub fn run_boot_check(paths: &Paths, image: &Path, settle: Duration) -> Result<()> {
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;
    let serial = SerialChannel::create(paths.target_dir())?;

    println!("[xtask] booting the shipped kernel");

    let mut child = Command::new(&installation.emulator)
        .args(machine_args(Remapping::Present))
        .args(["-display", "none", "-no-reboot"])
        .args(serial.arguments())
        .args(QMP_ON_STDIO)
        .args(drive_file(
            "if=pflash,format=raw,readonly=on",
            &installation.code_firmware,
        ))
        .args(drive_file("if=pflash,format=raw", vars.path()))
        .args(snapshot_drive_file("format=raw", image))
        .args(storage(paths.root())?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

    let serial_reader = match serial.connect(&mut child, Instant::now() + QMP_NEGOTIATION_TIMEOUT) {
        Ok(reader) => reader,
        Err(error) => {
            stop_qemu(&mut child).context("stopping QEMU after serial connection failed")?;
            return Err(error);
        }
    };

    let qmp = attach_qmp(
        &mut child,
        Instant::now() + QMP_NEGOTIATION_TIMEOUT,
        &NOT_CANCELLED,
    );
    let settled = Arc::new(AtomicBool::new(false));
    let panicked = Arc::new(AtomicBool::new(false));
    let serial_failed = Arc::new(AtomicBool::new(false));
    let required: Vec<_> = SHIPPED_REQUIRED_MARKERS
        .iter()
        .copied()
        .map(|marker| (marker, Arc::new(AtomicBool::new(false))))
        .collect();
    let forwarder = watch_serial(
        serial_reader,
        Arc::clone(&settled),
        Arc::clone(&panicked),
        Arc::clone(&serial_failed),
        required.clone(),
    );

    let outcome = qmp.and_then(|mut qmp| {
        watch(
            &mut child,
            &panicked,
            &serial_failed,
            BOOT_TIMEOUT,
            Some(&settled),
        )?;
        let before = keyboard_irq_count(&mut qmp)?;
        qmp.inject_key(
            SHIPPED_CHALLENGE_QCODE,
            Instant::now() + SHIPPED_CHALLENGE_TIMEOUT,
            &NOT_CANCELLED,
        )
        .context("injecting the shipped-kernel liveness key")?;
        watch(
            &mut child,
            &panicked,
            &serial_failed,
            SHIPPED_CHALLENGE_WINDOW,
            None,
        )?;
        let after = keyboard_irq_count(&mut qmp)?;
        ensure!(
            after == before,
            "the shipped kernel unmasked legacy keyboard IRQ1 ({before} -> {after})"
        );
        println!("[xtask] shipped liveness: keyboard input left IRQ1 masked at {after}");
        watch(&mut child, &panicked, &serial_failed, settle, None)
    });

    let stop_outcome = stop_qemu(&mut child);
    serial.close();
    let serial_outcome = finish_boot_serial(forwarder);

    stop_outcome?;
    let serial_observation = serial_outcome?;
    if let Some(transcript) =
        completed_panic(serial_observation.panic, serial_observation.panic_complete)?
    {
        bail!("the shipped kernel panicked during the settle window: {transcript}");
    }
    outcome?;
    for (record, seen) in required {
        if !seen.load(Ordering::Acquire) {
            bail!(
                "the shipped kernel settled without reporting {:?}",
                record.message
            );
        }
    }
    println!("[xtask] shipped kernel falls back, boots, responds, and settles");
    Ok(())
}

/// How long to allow for a cold start before the marker must have appeared.
///
/// Three human keystroke rounds fit inside it with room to spare, so a round
/// nobody answers ends as the kernel's own red check rather than as this
/// killing the machine — the verdict stays the kernel's (D-240).
const BOOT_TIMEOUT: Duration = Duration::from_secs(180);

/// A processor with no x2APIC or optional SMEP/SMAP/UMIP hardening.
///
/// The last `-cpu` wins, so this overrides [`MACHINE`]'s.
const DEGRADED_CPU: [&str; 2] = [
    "-cpu",
    "qemu64,-x2apic,-smep,-smap,-umip,hypervisor=on,tcg-cpuid=on",
];

/// What the kernel must say when it finds no x2APIC.
///
/// The refusal D-33 chose and D-90 made a gate check: xAPIC needs page tables
/// that do not exist when the APIC is brought up, so there is no fallback —
/// only a diagnosis.
const NO_X2APIC_DIAGNOSIS: &str = "local apic: this processor has no x2apic mode, and xapic needs page tables; interrupts stay off";
const NO_X2APIC_RECORD: RequiredLogRecord = RequiredLogRecord::warning(NO_X2APIC_DIAGNOSIS);

/// The forced configuration-port refusal in the degraded image.
const NO_PCI_MARKER: &str = "pci: the configuration ports are already claimed; no bus";

/// The hardware-free namespace witness targeted by the PW-207 mutation.
pub const DEGRADED_FILESYSTEM_MARKER: &str = "tmpfs: 1 file(s) and 1 directory(ies) at the root";

/// Both device names must resolve through devfs without a bus.
const DEGRADED_STORAGE_MARKER: &str = "devfs: storage is a block device";
const DEGRADED_CONSOLE_MARKER: &str = "devfs: console is a character device";

/// Optional hardening is absent, while mandatory WP and NX remain active.
const DEGRADED_PROTECTIONS_MARKER: &str = "cpu: protections wp nx";

/// The degraded CPU independently proves that extended state is unavailable.
pub const DEGRADED_XSTATE_MARKER: &str = "cpu: extended state unavailable; ring 3 stays closed";

/// Missing x2APIC leaves interrupts unadmitted, the first user-launch veto.
pub const DEGRADED_USER_MARKER: &str =
    "user: no program: maskable interrupts are not admitted; ring 3 is closed";

const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[
    RequiredLogRecord::info(QEMU_TCG_ISOLATION_MARKER),
    NO_X2APIC_RECORD,
    RequiredLogRecord::warning(NO_PCI_MARKER),
    RequiredLogRecord::info(DEGRADED_FILESYSTEM_MARKER),
    RequiredLogRecord::info(DEGRADED_STORAGE_MARKER),
    RequiredLogRecord::info(DEGRADED_CONSOLE_MARKER),
    RequiredLogRecord::info(DEGRADED_PROTECTIONS_MARKER),
    RequiredLogRecord::warning(DEGRADED_XSTATE_MARKER),
    RequiredLogRecord::warning(DEGRADED_USER_MARKER),
];

/// What a storage candidate says when no remapping unit can isolate it (D-357).
pub const NO_ISOLATION_MARKER: &str = "storage: no DMA isolation";

/// With every candidate refused, selection ends with no controller.
pub const NO_USABLE_CONTROLLER_MARKER: &str = "nvme: no usable controller";

/// The unisolated boot must refuse every candidate and still settle; the
/// first candidate's clean retirement proves the refusal path returns the
/// lease exactly.
const UNISOLATED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[
    RequiredLogRecord::info(QEMU_TCG_ISOLATION_MARKER),
    RequiredLogRecord::warning(NO_ISOLATION_MARKER),
    RequiredLogRecord::info(NVME_FALLBACK_MARKER),
    RequiredLogRecord::info(NO_USABLE_CONTROLLER_MARKER),
];

/// One degraded machine: which facility is missing, and what the boot must say.
#[derive(Debug, Clone, Copy)]
struct DegradedProfile {
    announcement: &'static str,
    cpu: Option<&'static [&'static str; 2]>,
    remapping: Remapping,
    required: &'static [RequiredLogRecord],
}

const NO_PCI_PROFILE: DegradedProfile = DegradedProfile {
    announcement: "booting a kernel with no PCI configuration or x2apic",
    cpu: Some(&DEGRADED_CPU),
    remapping: Remapping::Present,
    required: DEGRADED_REQUIRED_MARKERS,
};

const UNISOLATED_PROFILE: DegradedProfile = DegradedProfile {
    announcement: "booting the shipped kernel with no DMA-remapping unit",
    cpu: None,
    remapping: Remapping::Absent,
    required: UNISOLATED_REQUIRED_MARKERS,
};

/// Boot with two independent platform prerequisites absent and require both.
///
/// The negative half of D-32 and D-33: every other boot in the gate runs on
/// `-cpu max` with live PCI configuration, so nothing exercises what the
/// kernel does when either is missing. A precondition that is only ever met is
/// indistinguishable from one that is never checked (D-90, D-198).
///
/// The bar is deliberately not "it works": the kernel reports both refusals,
/// exercises the namespaces which need neither facility, and still reaches
/// its steady state rather than triple-faulting on a missing register.
/// Boot the degraded image while preserving an exact terminal panic.
pub fn observe_degraded(paths: &Paths, image: &Path, settle: Duration) -> Result<DegradedOutcome> {
    observe_profile(paths, image, settle, NO_PCI_PROFILE)
}

/// Boot the shipped image on a machine with no remapping unit and require
/// storage to refuse every candidate rather than run unisolated (D-357).
pub fn observe_unisolated(
    paths: &Paths,
    image: &Path,
    settle: Duration,
) -> Result<DegradedOutcome> {
    observe_profile(paths, image, settle, UNISOLATED_PROFILE)
}

fn observe_profile(
    paths: &Paths,
    image: &Path,
    settle: Duration,
    profile: DegradedProfile,
) -> Result<DegradedOutcome> {
    let installation = installation()?;
    let vars = RunVars::create(paths.target_dir(), &installation.vars_firmware)?;

    println!("[xtask] {}", profile.announcement);

    let mut child = Command::new(&installation.emulator)
        .args(machine_args(profile.remapping))
        .args(profile.cpu.map_or(&[][..], |cpu| &cpu[..]))
        .args(["-display", "none", "-serial", "stdio", "-no-reboot"])
        .args(drive_file(
            "if=pflash,format=raw,readonly=on",
            &installation.code_firmware,
        ))
        .args(drive_file("if=pflash,format=raw", vars.path()))
        .args(snapshot_drive_file("format=raw", image))
        .args(storage(paths.root())?)
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not launch {}", installation.emulator.display()))?;

    let stdout = child.stdout.take().context("taking QEMU's stdout pipe")?;
    let settled = Arc::new(AtomicBool::new(false));
    let panicked = Arc::new(AtomicBool::new(false));
    let serial_failed = Arc::new(AtomicBool::new(false));
    let required: Vec<_> = profile
        .required
        .iter()
        .copied()
        .map(|marker| (marker, Arc::new(AtomicBool::new(false))))
        .collect();
    let forwarder = watch_serial(
        stdout,
        Arc::clone(&settled),
        Arc::clone(&panicked),
        Arc::clone(&serial_failed),
        required.clone(),
    );

    let outcome = watch_degraded(&mut child, &panicked, &serial_failed, &settled, settle);

    let stop_outcome = stop_qemu(&mut child);
    let serial_outcome = finish_boot_serial(forwarder);

    stop_outcome?;
    outcome?;
    let serial_observation = serial_outcome?;
    if let Some(transcript) =
        completed_panic(serial_observation.panic, serial_observation.panic_complete)?
    {
        return Ok(DegradedOutcome::Panicked {
            transcript,
            no_x2apic_diagnosed: serial_observation.no_x2apic_before_panic,
        });
    }
    for (record, seen) in required {
        if !seen.load(Ordering::Acquire) {
            bail!(
                "the degraded kernel settled without reporting {:?}",
                record.message
            );
        }
    }
    Ok(DegradedOutcome::Settled)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DegradedWatch {
    Complete,
    Panicked,
}

fn watch_degraded(
    child: &mut Child,
    panicked: &AtomicBool,
    serial_failed: &AtomicBool,
    settled: &AtomicBool,
    settle: Duration,
) -> Result<DegradedWatch> {
    match watch_degraded_window(child, panicked, serial_failed, BOOT_TIMEOUT, Some(settled))? {
        DegradedWatch::Panicked => Ok(DegradedWatch::Panicked),
        DegradedWatch::Complete => {
            watch_degraded_window(child, panicked, serial_failed, settle, None)
        }
    }
}

fn watch_degraded_window(
    child: &mut Child,
    panicked: &AtomicBool,
    serial_failed: &AtomicBool,
    window: Duration,
    until: Option<&AtomicBool>,
) -> Result<DegradedWatch> {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        if panicked.load(Ordering::Acquire) {
            return Ok(DegradedWatch::Panicked);
        }
        if let Some(status) = child.try_wait().context("waiting for QEMU")? {
            bail!("QEMU exited before the degraded kernel settled: {status}");
        }
        if serial_failed.load(Ordering::Acquire) {
            bail!("reading the degraded kernel's serial output failed");
        }
        if until.is_some_and(|marker| marker.load(Ordering::Acquire)) {
            return Ok(DegradedWatch::Complete);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    match until {
        Some(_) => bail!("the degraded kernel never reached its steady state ({SETTLED_MARKER:?})"),
        None => Ok(DegradedWatch::Complete),
    }
}

/// Watch the serial flags for `window`, failing on a panic or an early exit.
///
/// With `until`, the window is a deadline that flag must beat; without one,
/// the whole window is watched.
fn watch(
    child: &mut Child,
    panicked: &AtomicBool,
    serial_failed: &AtomicBool,
    window: Duration,
    until: Option<&AtomicBool>,
) -> Result<()> {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        if panicked.load(Ordering::Acquire) {
            bail!("the shipped kernel panicked: see the log above");
        }
        if let Some(status) = child.try_wait().context("waiting for QEMU")? {
            bail!("the shipped kernel stopped early ({status})");
        }
        if serial_failed.load(Ordering::Acquire) {
            bail!("the shipped kernel's serial reader stopped");
        }
        if until.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Ok(());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    match until {
        Some(_) => bail!("the shipped kernel never reached its steady state ({SETTLED_MARKER:?})"),
        None => Ok(()),
    }
}

/// Forward every serial line, flagging the markers the boot check requires.
fn watch_serial(
    serial: impl Read + Send + 'static,
    settled: Arc<AtomicBool>,
    panicked: Arc<AtomicBool>,
    serial_failed: Arc<AtomicBool>,
    required: Vec<(RequiredLogRecord, Arc<AtomicBool>)>,
) -> JoinHandle<Result<BootSerialObservation>> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(serial);
        let mut line = Vec::new();
        let mut total = 0;
        let mut observation = BootSerialObservation::default();
        loop {
            line.clear();
            let read = match read_serial_line(&mut reader, &mut line, &mut total) {
                Ok(read) => read,
                Err(error) => {
                    serial_failed.store(true, Ordering::Release);
                    return Err(error).context("reading shipped QEMU serial output");
                }
            };
            if read == 0 {
                return Ok(observation);
            }
            let text = String::from_utf8_lossy(&line);
            echo_serial_line(&line);
            if is_settled_record(&text) {
                settled.store(true, Ordering::Release);
            }
            for (record, seen) in &required {
                let present = is_required_boot_marker(&text, *record);
                if present {
                    seen.store(true, Ordering::Release);
                }
            }
            match observe_boot_line(&mut observation, &text) {
                Ok(transition) => transition.publish(&panicked),
                Err(error) => {
                    serial_failed.store(true, Ordering::Release);
                    return Err(error).context("reading the kernel's boot transcript");
                }
            }
        }
    })
}

fn finish_boot_serial(
    forwarder: JoinHandle<Result<BootSerialObservation>>,
) -> Result<BootSerialObservation> {
    forwarder
        .join()
        .map_err(|_| anyhow!("the shipped-kernel serial reader panicked"))?
}
#[cfg(test)]
mod tests {

    #[test]
    fn the_human_wait_fits_the_harness_patience() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root")
            .to_path_buf();
        let input = std::fs::read_to_string(root.join("kernel/src/input.rs")).expect("input.rs");
        let wait_ms: u64 = input
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("pub(crate) const HUMAN_WAIT_MS: u64 = ")
            })
            .and_then(|value| {
                let digits = value.trim_end_matches(';').replace('_', "");
                <u64 as core::str::FromStr>::from_str(&digits).ok()
            })
            .expect("no HUMAN_WAIT_MS in input.rs");

        const ROUNDS: u64 = 3;
        let unanswered = Duration::from_millis(wait_ms * ROUNDS);
        assert!(
            unanswered < BOOT_TIMEOUT,
            "{ROUNDS} rounds of {wait_ms}ms exceed the {BOOT_TIMEOUT:?} boot timeout"
        );
    }
    use super::*;

    #[test]
    fn a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal() {
        let mut observation = BootSerialObservation::default();
        let panicked = AtomicBool::new(false);
        let diagnosis = format!("[warn ] {NO_X2APIC_DIAGNOSIS}\r\n");
        observe_boot_line(&mut observation, &diagnosis)
            .unwrap()
            .publish(&panicked);
        observe_boot_line(
            &mut observation,
            "[error] KERNEL PANIC on thread 'boot': exact cause\\nat source.rs:1 \t\r\n",
        )
        .unwrap()
        .publish(&panicked);

        assert!(observation.no_x2apic_before_panic);
        assert!(!observation.panic_complete);
        assert!(!panicked.load(Ordering::Acquire));
        assert_eq!(
            observation.panic.as_deref(),
            Some("[error] KERNEL PANIC on thread 'boot': exact cause\\nat source.rs:1 \t")
        );
        assert!(completed_panic(observation.panic.clone(), observation.panic_complete).is_err());

        assert!(observe_boot_line(&mut observation, "continuation\r\n").is_err());
        assert!(
            observe_boot_line(
                &mut observation,
                "[error] KERNEL PANIC on thread 'second': duplicate\r\n"
            )
            .is_err()
        );

        for near_miss in [
            format!("[error] {COMPLETE_MARKER}"),
            format!("[warn ] {COMPLETE_MARKER}\r\n"),
            format!("[error] {COMPLETE_MARKER} trailing\r\n"),
            format!("[error] {COMPLETE_MARKER}\r\r\n"),
        ] {
            assert!(!is_complete_panic_record(&near_miss), "{near_miss:?}");
        }

        let terminal = format!("[error] {COMPLETE_MARKER}\r\n");
        let mut unopened = BootSerialObservation::default();
        assert!(observe_boot_line(&mut unopened, &terminal).is_err());
        let transition = observe_boot_line(&mut observation, &terminal).unwrap();
        assert_eq!(transition, PanicTransition::Completed);
        transition.publish(&panicked);
        assert!(observation.panic_complete);
        assert!(panicked.load(Ordering::Acquire));
        assert_eq!(
            completed_panic(observation.panic.clone(), observation.panic_complete).unwrap(),
            Some(
                "[error] KERNEL PANIC on thread 'boot': exact cause\\nat source.rs:1 \t"
                    .to_string()
            )
        );
        assert!(observe_boot_line(&mut observation, &terminal).is_err());
        assert!(observe_boot_line(&mut observation, "after terminal\r\n").is_err());

        let mut simultaneous = BootSerialObservation::default();
        observe_boot_line(
            &mut simultaneous,
            "[error] KERNEL PANIC on thread 'boot': no x2apic mode is not a prior diagnosis\r\n",
        )
        .unwrap();
        assert!(!simultaneous.no_x2apic_before_panic);

        for near_miss in [
            format!("[info ] {NO_X2APIC_DIAGNOSIS}\r\n"),
            format!("[warn ] prefix {NO_X2APIC_DIAGNOSIS}\r\n"),
            format!("[warn ] {NO_X2APIC_DIAGNOSIS} trailing\r\n"),
            "[warn ] no x2apic mode\r\n".to_string(),
            format!("[warn ] {NO_X2APIC_DIAGNOSIS}"),
        ] {
            let mut rejected = BootSerialObservation::default();
            observe_boot_line(&mut rejected, &near_miss).unwrap();
            assert!(!rejected.no_x2apic_before_panic, "accepted {near_miss:?}");
        }

        for near_miss in [
            "KERNEL PANIC on thread 'boot': missing severity\r\n",
            "[warn ] KERNEL PANIC on thread 'boot': wrong severity\r\n",
            "prefix [error] KERNEL PANIC on thread 'boot': not at column zero\r\n",
            "[error] KERNEL PANIC: missing thread prefix\r\n",
        ] {
            let mut rejected = BootSerialObservation::default();
            assert_eq!(
                observe_boot_line(&mut rejected, near_miss).unwrap(),
                PanicTransition::Unchanged
            );
            assert!(rejected.panic.is_none(), "accepted {near_miss:?}");
        }

        let mut unterminated = BootSerialObservation::default();
        assert!(
            observe_boot_line(
                &mut unterminated,
                "[error] KERNEL PANIC on thread 'boot': missing newline"
            )
            .is_err()
        );
        assert!(unterminated.panic.is_none());

        for malformed in [
            "[error] KERNEL PANIC on thread 'boot': internal\rreturn\r\n",
            "[error] KERNEL PANIC on thread 'boot': doubled ending\r\r\n",
        ] {
            let mut rejected = BootSerialObservation::default();
            assert!(observe_boot_line(&mut rejected, malformed).is_err());
            assert!(rejected.panic.is_none(), "accepted {malformed:?}");
        }
    }

    #[test]
    fn the_degraded_no_x2apic_requirement_uses_the_exact_warning_record() {
        let required = NO_X2APIC_RECORD;
        assert!(DEGRADED_REQUIRED_MARKERS.contains(&required));
        assert!(
            !DEGRADED_REQUIRED_MARKERS
                .iter()
                .any(|record| record.message == "no x2apic mode")
        );

        let exact = format!("[warn ] {NO_X2APIC_DIAGNOSIS}\r\n");
        assert!(is_required_boot_marker(&exact, required));
        for near_miss in [
            format!("[info ] {NO_X2APIC_DIAGNOSIS}\r\n"),
            format!("[warn ] prefix {NO_X2APIC_DIAGNOSIS}\r\n"),
            format!("[warn ] {NO_X2APIC_DIAGNOSIS} trailing\r\n"),
            "[warn ] no x2apic mode\r\n".to_string(),
            format!("[warn ] {NO_X2APIC_DIAGNOSIS}"),
        ] {
            assert!(
                !is_required_boot_marker(&near_miss, required),
                "accepted {near_miss:?}"
            );
        }
    }

    #[test]
    fn required_boot_records_refuse_framing_and_message_near_misses() {
        for record in SHIPPED_REQUIRED_MARKERS
            .iter()
            .chain(DEGRADED_REQUIRED_MARKERS)
            .copied()
        {
            for ending in ["\n", "\r\n"] {
                let exact = format!("{}{}{ending}", record.severity.prefix(), record.message);
                assert!(
                    is_required_boot_marker(&exact, record),
                    "rejected {exact:?}"
                );
            }

            let wrong_severity = if record.severity == LogSeverity::Info {
                LogSeverity::Warning
            } else {
                LogSeverity::Info
            };
            for near_miss in [
                format!("{}{}\r\n", wrong_severity.prefix(), record.message),
                format!("prefix {}{}\r\n", record.severity.prefix(), record.message),
                format!("{}{} suffix\r\n", record.severity.prefix(), record.message),
                format!("{}{} \r\n", record.severity.prefix(), record.message),
                format!("{}{}\t\r\n", record.severity.prefix(), record.message),
                format!("{}{}\u{a0}\r\n", record.severity.prefix(), record.message),
                format!("{}{}", record.severity.prefix(), record.message),
                format!("{}{}\r\r\n", record.severity.prefix(), record.message),
                format!("{}{}\rinside\r\n", record.severity.prefix(), record.message),
            ] {
                assert!(
                    !is_required_boot_marker(&near_miss, record),
                    "accepted {near_miss:?} for {record:?}"
                );
            }
        }
    }

    #[test]
    fn failure_records_cannot_satisfy_required_boot_evidence() {
        let cases = [
            (
                RequiredLogRecord::info("cpu: user isolation shared kernel mappings: qemu_tcg"),
                "[warn ] cpu: user isolation shared kernel mappings: refused; ring 3 stays closed\r\n",
            ),
            (
                RequiredLogRecord::info("nvme: candidate 1 retired cleanly; trying next"),
                "[warn ] nvme: candidate 1 left uncertain controller state after reset\r\n",
            ),
            (
                RequiredLogRecord::warning(
                    "local apic: this processor has no x2apic mode, and xapic needs page tables; interrupts stay off",
                ),
                "[warn ] local apic: no x2apic mode\r\n",
            ),
            (
                RequiredLogRecord::warning(
                    "pci: the configuration ports are already claimed; no bus",
                ),
                "[warn ] pci: no storage controllers found\r\n",
            ),
            (
                RequiredLogRecord::warning("storage: no DMA isolation"),
                "[info ] storage: no DMA isolation\r\n",
            ),
            (
                RequiredLogRecord::info("tmpfs: 1 file(s) and 1 directory(ies) at the root"),
                "[warn ] tmpfs: the root would not list: NotFound\r\n",
            ),
            (
                RequiredLogRecord::info("devfs: storage is a block device"),
                "[warn ] devfs: a device would not resolve: NotFound\r\n",
            ),
            (
                RequiredLogRecord::info("devfs: console is a character device"),
                "[warn ] devfs: a device resolved to something with bytes in it\r\n",
            ),
            (
                RequiredLogRecord::info("cpu: protections wp nx"),
                "[info ] cpu: protections wp smep nx\r\n",
            ),
            (
                RequiredLogRecord::warning("cpu: extended state unavailable; ring 3 stays closed"),
                "[info ] cpu: extended state 832 bytes, xcr0 0x7, compacted\r\n",
            ),
            (
                RequiredLogRecord::warning(
                    "user: no program: maskable interrupts are not admitted; ring 3 is closed",
                ),
                "[info ] user: program running in slot 0\r\n",
            ),
        ];

        for (record, failure) in cases {
            assert!(
                !is_required_boot_marker(failure, record),
                "accepted failure producer {failure:?} for {record:?}"
            );
        }
    }

    #[test]
    fn steady_state_requires_one_exact_dynamic_info_record() {
        for exact in [
            "[info ] kernel: reading the keyboard, uptime 0 ms\n",
            "[info ] kernel: reading the keyboard, uptime 1 ms\r\n",
            "[info ] kernel: reading the keyboard, uptime 18446744073709551615 ms\r\n",
        ] {
            assert!(is_settled_record(exact), "rejected {exact:?}");
        }

        for near_miss in [
            "[warn ] kernel: reading the keyboard, uptime 1 ms\r\n",
            "prefix [info ] kernel: reading the keyboard, uptime 1 ms\r\n",
            "[info ] prefix kernel: reading the keyboard, uptime 1 ms\r\n",
            "[info ] kernel: reading the keyboard, uptime 1 ms trailing\r\n",
            "[info ] kernel: reading the keyboard, uptime 1 ms",
            "[info ] kernel: reading the keyboard, uptime 1 ms \r\n",
            "[info ] kernel: reading the keyboard, uptime 1 ms\t\r\n",
            "[info ] kernel: reading the keyboard, uptime 1 ms\u{a0}\r\n",
            "[info ] kernel: reading the keyboard, uptime 01 ms\r\n",
            "[info ] kernel: reading the keyboard, uptime +1 ms\r\n",
            "[info ] kernel: reading the keyboard, uptime -1 ms\r\n",
            "[info ] kernel: reading the keyboard, uptime \u{0661} ms\r\n",
            "[info ] kernel: reading the keyboard, uptime 18446744073709551616 ms\r\n",
            "[info ] kernel: reading the keyboard, uptime  ms\r\n",
            "[info ] kernel: reading the keyboard, uptime 1\r\n",
            "[info ] kernel: reading the keyboard, uptime 1\r ms\r\n",
            "[info ] kernel: entering idle loop\r\n",
        ] {
            assert!(!is_settled_record(near_miss), "accepted {near_miss:?}");
        }
    }

    #[test]
    fn interactive_input_requires_exact_terminated_info_records() {
        for record in INTERACTIVE_MARKERS {
            let exact = format!("[info ] {}\r\n", record.message);
            let mut next = INTERACTIVE_MARKERS
                .iter()
                .position(|candidate| candidate == record)
                .unwrap();
            assert!(
                observe_interactive_marker(&exact, &mut next, InteractiveMarkerOrder::Strict)
                    .unwrap()
            );

            for near_miss in [
                format!("[warn ] {}\r\n", record.message),
                format!("[error] {}\r\n", record.message),
                format!("prefix [info ] {}\r\n", record.message),
                format!("[info ] prefix {}\r\n", record.message),
                format!("[info ] {} suffix\r\n", record.message),
                format!("[info ] {} \r\n", record.message),
                format!("[info ] {}\t\r\n", record.message),
                format!("[info ] {}\u{a0}\r\n", record.message),
                format!("[info ] {}", record.message),
                format!("[info ] {}\r\r\n", record.message),
            ] {
                let before = next;
                assert!(
                    !observe_interactive_marker(
                        &near_miss,
                        &mut next,
                        InteractiveMarkerOrder::Strict,
                    )
                    .unwrap(),
                    "accepted {near_miss:?}"
                );
                assert_eq!(next, before);
            }
        }
    }

    #[test]
    fn interactive_input_markers_are_unique_and_ordered() {
        let records: Vec<String> = INTERACTIVE_MARKERS
            .iter()
            .map(|record| format!("[info ] {}\r\n", record.message))
            .collect();
        let mut next = 0;
        assert!(
            observe_interactive_marker(&records[1], &mut next, InteractiveMarkerOrder::Strict,)
                .is_err()
        );
        assert_eq!(next, 0);
        assert!(
            observe_interactive_marker(&records[0], &mut next, InteractiveMarkerOrder::Strict,)
                .unwrap()
        );
        assert_eq!(next, 1);
        assert!(
            observe_interactive_marker(&records[0], &mut next, InteractiveMarkerOrder::Strict,)
                .is_err()
        );
        assert_eq!(next, 1);
        assert!(
            observe_interactive_marker(&records[1], &mut next, InteractiveMarkerOrder::Strict,)
                .unwrap()
        );
        assert!(
            observe_interactive_marker(&records[2], &mut next, InteractiveMarkerOrder::Strict,)
                .unwrap()
        );
        assert_eq!(next, INTERACTIVE_MARKERS.len());
        assert!(
            !observe_interactive_marker(
                "[info ] unrelated\r\n",
                &mut next,
                InteractiveMarkerOrder::Strict,
            )
            .unwrap()
        );

        let mut mutation_next = 0;
        assert!(
            observe_interactive_marker(
                &records[1],
                &mut mutation_next,
                InteractiveMarkerOrder::Mutation,
            )
            .unwrap()
        );
        assert_eq!(mutation_next, 2);
        assert!(
            observe_interactive_marker(
                &records[0],
                &mut mutation_next,
                InteractiveMarkerOrder::Mutation,
            )
            .is_err()
        );
        assert!(
            observe_interactive_marker(
                &records[2],
                &mut mutation_next,
                InteractiveMarkerOrder::Mutation,
            )
            .unwrap()
        );
        assert_eq!(injection_action(0, 1), InjectionAction::Wait);
        assert_eq!(injection_action(1, 1), InjectionAction::Inject);
        assert_eq!(injection_action(2, 1), InjectionAction::Skip);
    }

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct QemuFixture {
        root: PathBuf,
        template: PathBuf,
    }

    impl QemuFixture {
        fn create() -> Self {
            let root = std::env::temp_dir().join(format!(
                "wizard qemu,fixture-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).expect("QEMU fixture directory");
            let template = root.join("firmware template,pristine.fd");
            std::fs::write(&template, b"pristine firmware variables")
                .expect("QEMU variable-store template");
            Self { root, template }
        }
    }

    impl Drop for QemuFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating storage file symlink");
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_file(target, link).expect("creating storage file symlink");
    }

    #[cfg(unix)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating QEMU directory symlink");
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_dir(target, link).expect("creating QEMU directory symlink");
    }

    #[cfg(unix)]
    fn remove_directory_symlink(path: &Path) {
        std::fs::remove_file(path).expect("removing QEMU directory symlink");
    }

    #[cfg(windows)]
    fn remove_directory_symlink(path: &Path) {
        std::fs::remove_dir(path).expect("removing QEMU directory symlink");
    }

    fn publish_fixture(parent: &Path, bytes: &'static [u8]) -> Result<PathBuf> {
        let backing = parent.join("nvme.img");
        publish_namespace(parent, &backing, |file| {
            file.write_all(bytes).context("writing fixture namespace")
        })?;
        Ok(backing)
    }

    struct FakeQemuChild {
        exited: bool,
        killed: bool,
        reject_kill: bool,
        exit_on_rejected_kill: bool,
        polls_after_kill: usize,
        exit_after_kill_polls: Option<usize>,
    }

    impl FakeQemuChild {
        fn exited(&mut self) -> std::io::Result<bool> {
            if self.exited || (self.reject_kill && self.exit_on_rejected_kill && self.killed) {
                return Ok(true);
            }
            if self.killed && !self.reject_kill {
                self.polls_after_kill += 1;
                return Ok(self
                    .exit_after_kill_polls
                    .is_some_and(|limit| self.polls_after_kill >= limit));
            }
            Ok(false)
        }

        fn terminate(&mut self) -> std::io::Result<()> {
            self.killed = true;
            if self.reject_kill {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "fixture refused termination",
                ))
            } else {
                Ok(())
            }
        }
    }

    fn fake_qemu() -> FakeQemuChild {
        FakeQemuChild {
            exited: false,
            killed: false,
            reject_kill: false,
            exit_on_rejected_kill: false,
            polls_after_kill: 0,
            exit_after_kill_polls: Some(1),
        }
    }

    #[test]
    fn the_machine_type_is_a_versioned_q35_not_an_alias() {
        let Some(version) = MACHINE_TYPE.strip_prefix("pc-q35-") else {
            panic!("the QEMU machine is not a q35 model");
        };
        let Some((major, minor)) = version.split_once('.') else {
            panic!("the QEMU machine has no explicit version");
        };
        assert!(
            !major.is_empty()
                && !minor.is_empty()
                && major.bytes().all(|byte| byte.is_ascii_digit())
                && minor.bytes().all(|byte| byte.is_ascii_digit())
        );
    }

    #[test]
    fn every_qemu_runner_pins_the_exact_tcg_profile() {
        let machine = machine_args(Remapping::Present);
        assert_eq!(
            machine
                .iter()
                .filter(|argument| **argument == "-no-user-config")
                .count(),
            1,
            "ambient QEMU configuration can rewrite the fixed profile"
        );

        let accelerator: Vec<_> = machine
            .windows(2)
            .filter_map(|pair| (pair[0] == "-accel").then_some(pair[1]))
            .collect();
        assert_eq!(accelerator, ["tcg"]);

        let cpu: Vec<_> = machine
            .windows(2)
            .filter_map(|pair| (pair[0] == "-cpu").then_some(pair[1]))
            .collect();
        assert_eq!(cpu, ["max,hypervisor=on,tcg-cpuid=on"]);

        // The unit precedes every PCI device, and only the unisolated boot
        // leaves it out; nothing else about the two machines differs.
        let devices: Vec<_> = machine
            .windows(2)
            .filter_map(|pair| (pair[0] == "-device").then_some(pair[1]))
            .collect();
        assert_eq!(
            devices,
            [
                "intel-iommu,intremap=on,eim=on,caching-mode=on",
                "pci-bridge,id=behind,chassis_nr=1"
            ]
        );
        let unisolated = machine_args(Remapping::Absent);
        let unit = machine
            .iter()
            .position(|argument| *argument == IOMMU_DEVICE[1])
            .expect("the ordinary machine carries the unit");
        let mut without_unit = machine.clone();
        without_unit.drain(unit - 1..=unit);
        assert_eq!(unisolated, without_unit);
        assert_eq!(
            DEGRADED_CPU,
            [
                "-cpu",
                "qemu64,-x2apic,-smep,-smap,-umip,hypervisor=on,tcg-cpuid=on"
            ]
        );

        let source = include_str!("qemu.rs");
        let command = ["Command::new", "(&installation.emulator)"].concat();
        let profile = [".args", "(machine_args("].concat();
        let commands = source.matches(&command).count();
        assert_ne!(commands, 0, "no QEMU process construction was found");
        assert_eq!(
            source.matches(&profile).count(),
            commands,
            "a QEMU process bypasses the shared machine profile"
        );
    }

    #[test]
    fn interactive_arguments_cannot_replace_the_tcg_profile() {
        for conflict in [
            vec!["-accel", "whpx"],
            vec!["-accel=whpx"],
            vec!["--accel", "whpx"],
            vec!["--accel=whpx"],
            vec!["-enable-kvm"],
            vec!["--enable-kvm=true"],
            vec!["-machine", "pc-q35-9.2,accel=whpx"],
            vec!["-machine", "accel=whpx"],
            vec!["-machine=pc-q35-9.2,accel=whpx"],
            vec!["--machine", "accel=whpx"],
            vec!["--machine=pc-q35-9.2,accel=whpx"],
            vec!["-M", "pc"],
            vec!["-M", "accel=whpx"],
            vec!["-M=pc"],
            vec!["-cpu", "host"],
            vec!["-cpu=host"],
            vec!["--cpu", "host"],
            vec!["--cpu=max,hv-vendor-id=TCGTCGTCGTCG"],
            vec!["-global", "max-x86_64-cpu.hypervisor=off"],
            vec!["-global=max-x86_64-cpu.tcg-cpuid=off"],
            vec![
                "--global",
                "driver=max-x86_64-cpu,property=hypervisor,value=off",
            ],
            vec!["-readconfig", "hostile.cfg"],
            vec!["-readconfig=hostile.cfg"],
            vec!["--readconfig", "hostile.cfg"],
        ] {
            let conflict: Vec<_> = conflict.into_iter().map(OsString::from).collect();
            let error = validate_run_arguments(&conflict).unwrap_err();
            assert!(error.to_string().contains("fixed TCG machine profile"));
        }

        let diagnostic: Vec<_> = [
            "-d",
            "int",
            "-display",
            "gtk",
            "-set",
            "drive.disk.snapshot=on",
            "--set",
            "chardev.serial.logfile=serial.log",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        validate_run_arguments(&diagnostic).unwrap();

        let source = include_str!("qemu.rs");
        let start = source
            .find("pub fn run(paths: &Paths, image: &Path, extra: &[OsString])")
            .expect("interactive QEMU runner");
        let run = &source[start..];
        let validation = run
            .find(&["validate_run_arguments", "(extra)?;"].concat())
            .expect("interactive argument validation");
        let installation = run.find("let installation = installation()?;").unwrap();
        let forwarding = run.find(".args(extra)").unwrap();
        assert!(validation < installation && installation < forwarding);
    }

    #[test]
    fn automated_boots_require_the_exact_producer_records() {
        assert_eq!(
            INTERACTIVE_MARKERS,
            &[
                RequiredLogRecord::info("selftest: awaiting storage-delay keystroke"),
                RequiredLogRecord::info("selftest: awaiting injected keystroke"),
                RequiredLogRecord::info("selftest: awaiting translated keystroke"),
            ]
        );
        assert_eq!(
            SHIPPED_REQUIRED_MARKERS,
            &[
                RequiredLogRecord::info("cpu: user isolation shared kernel mappings: qemu_tcg"),
                RequiredLogRecord::info("nvme: candidate 1 retired cleanly; trying next"),
                RequiredLogRecord::info(
                    "keyboard: PCI interrupt isolation keeps every io-apic line masked; polling",
                ),
                RequiredLogRecord::info("keyboard: polled every 10 ms"),
            ]
        );
        assert_eq!(
            DEGRADED_REQUIRED_MARKERS,
            &[
                RequiredLogRecord::info("cpu: user isolation shared kernel mappings: qemu_tcg"),
                RequiredLogRecord::warning(
                    "local apic: this processor has no x2apic mode, and xapic needs page tables; interrupts stay off"
                ),
                RequiredLogRecord::warning(
                    "pci: the configuration ports are already claimed; no bus"
                ),
                RequiredLogRecord::info("tmpfs: 1 file(s) and 1 directory(ies) at the root"),
                RequiredLogRecord::info("devfs: storage is a block device"),
                RequiredLogRecord::info("devfs: console is a character device"),
                RequiredLogRecord::info("cpu: protections wp nx"),
                RequiredLogRecord::warning("cpu: extended state unavailable; ring 3 stays closed"),
                RequiredLogRecord::warning(
                    "user: no program: maskable interrupts are not admitted; ring 3 is closed"
                ),
            ]
        );
    }

    #[test]
    fn exe_name_carries_the_platform_suffix() {
        let name = exe_name("qemu-system-x86_64");
        assert!(name.ends_with(std::env::consts::EXE_SUFFIX));
        assert!(name.starts_with("qemu-system-x86_64"));
    }

    #[test]
    fn which_in_finds_a_binary_that_is_there() {
        let this = std::env::current_exe().unwrap();
        let dir = this.parent().unwrap().to_path_buf();
        let name = this.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(which_in([dir], &name), Some(this));
    }

    #[test]
    fn which_in_reports_a_binary_that_is_not() {
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        assert_eq!(which_in([dir], "definitely-not-installed-xyz"), None);
    }

    #[test]
    fn drive_paths_escape_commas_without_quoting_spaces() {
        assert_eq!(
            drive_file("format=raw", Path::new("a space/disk,name.img")),
            [
                OsString::from("-drive"),
                OsString::from("format=raw,file=a space/disk,,name.img")
            ]
        );
    }

    #[test]
    fn shared_media_writes_use_a_private_snapshot() {
        assert_eq!(
            snapshot_drive_file("format=raw", Path::new("disk.img")),
            [
                OsString::from("-drive"),
                OsString::from("format=raw,snapshot=on,file=disk.img")
            ]
        );
    }

    #[test]
    fn qemu_control_stays_on_the_child_s_own_streams() {
        assert_eq!(QMP_ON_STDIO, ["-qmp", "stdio"]);
        let source = include_str!("qemu.rs");
        let (production, _) = source
            .split_once("#[cfg(test)]\nmod tests")
            .expect("qemu.rs has its test module");
        for forbidden in ["TcpListener", "\"-monitor\"", "server=off"] {
            assert!(
                !production.contains(forbidden),
                "QEMU control regained `{forbidden}`"
            );
        }
    }

    #[test]
    fn automated_serial_uses_a_private_backpressured_transport() {
        let fixture = QemuFixture::create();
        let first = SerialChannel::create(&fixture.root).unwrap();
        let second = SerialChannel::create(&fixture.root).unwrap();
        let arguments = first.arguments();
        let backend = arguments[1].to_string_lossy();
        assert_eq!(arguments[0], "-chardev");
        assert_eq!(arguments[2], "-serial");
        assert_eq!(arguments[3], "chardev:serial0");
        assert!(!backend.starts_with("file,"), "serial regained a disk sink");
        #[cfg(windows)]
        {
            assert!(backend.starts_with("pipe,id=serial0,path=wizard-bedrock-serial-"));
            assert_ne!(first.name, second.name, "serial pipe names repeated");
        }
        #[cfg(unix)]
        {
            assert!(backend.starts_with("socket,id=serial0,path="));
            assert_ne!(first.socket, second.socket, "serial socket names repeated");
        }
    }

    #[test]
    fn a_serial_reader_reads_every_piped_byte_and_ends_after_close() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        let closed = Arc::new(AtomicBool::new(false));
        let mut follower = SerialReader {
            inner: Box::new(reader),
            closed: Arc::clone(&closed),
        };
        let writer = std::thread::spawn(move || {
            for chunk in [b"first ".as_slice(), b"second\n", b"late tail"] {
                writer.write_all(chunk).unwrap();
                writer.flush().unwrap();
                std::thread::sleep(Duration::from_millis(40));
            }
        });
        let reader = std::thread::spawn(move || {
            let mut text = Vec::new();
            follower.read_to_end(&mut text).unwrap();
            text
        });
        writer.join().unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            !reader.is_finished(),
            "the follower ended before QEMU exited"
        );
        closed.store(true, Ordering::Release);
        assert_eq!(reader.join().unwrap(), b"first second\nlate tail");
    }

    #[test]
    fn automated_serial_capture_bounds_each_record_and_the_whole_run() {
        let mut reader = BufReader::new(std::io::Cursor::new(b"abc\ndef\n"));
        let mut line = Vec::new();
        let mut total = 0;
        assert_eq!(
            read_serial_line_with_limits(&mut reader, &mut line, &mut total, 4, 8).unwrap(),
            4
        );
        assert_eq!(line, b"abc\n");
        assert_eq!(
            read_serial_line_with_limits(&mut reader, &mut line, &mut total, 4, 8).unwrap(),
            4
        );
        assert_eq!(total, 8);

        let mut long = BufReader::new(std::io::Cursor::new(b"abcd\n"));
        let error = read_serial_line_with_limits(&mut long, &mut line, &mut 0, 4, 32)
            .expect_err("oversized record");
        assert!(error.to_string().contains("serial line"));

        let mut aggregate = BufReader::new(std::io::Cursor::new(b"a\nb\n"));
        let mut total = 0;
        read_serial_line_with_limits(&mut aggregate, &mut line, &mut total, 8, 3).unwrap();
        let error = read_serial_line_with_limits(&mut aggregate, &mut line, &mut total, 8, 3)
            .expect_err("aggregate overrun");
        assert!(error.to_string().contains("transcript"));
    }

    #[test]
    fn automated_serial_display_escapes_guest_terminal_controls() {
        assert_eq!(
            escaped_serial_line(b"ok\x1b[2J\r\x08\0\x80\n"),
            r"ok\x1b[2J\x0d\x08\x00\x80"
        );
        assert_eq!(escaped_serial_line(b"plain ASCII\n"), "plain ASCII");
    }

    #[test]
    fn a_serial_reader_refuses_an_early_transport_close() {
        let (reader, writer) = std::io::pipe().unwrap();
        let mut follower = SerialReader {
            inner: Box::new(reader),
            closed: Arc::new(AtomicBool::new(false)),
        };
        drop(writer);
        let error = follower
            .read(&mut [0u8; 1])
            .expect_err("premature serial close");
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn the_debug_endpoint_is_allocated_and_loopback_only() {
        let report = "#chr072: filename=gdb\ngdb: filename=disconnected:tcp:127.0.0.1:56552,server=on\n(qemu)";
        assert_eq!(parse_debug_endpoint(report).unwrap(), "127.0.0.1:56552");

        for refused in [
            "serial0: filename=null",
            "gdb: filename=disconnected:tcp:0.0.0.0:56552,server=on",
            "gdb: filename=disconnected:tcp:127.0.0.1:0,server=on",
            "gdb: filename=disconnected:tcp:127.0.0.1:not-a-port,server=on",
        ] {
            assert!(parse_debug_endpoint(refused).is_err(), "accepted {refused}");
        }
    }

    #[test]
    fn simultaneous_runs_get_distinct_writable_firmware() {
        let fixture = QemuFixture::create();
        let target = Arc::new(fixture.root.join("target"));
        let template = Arc::new(fixture.template.clone());
        let barrier = Arc::new(std::sync::Barrier::new(3));

        let first = {
            let target = Arc::clone(&target);
            let template = Arc::clone(&template);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                RunVars::create(&target, &template).expect("first private variable store")
            })
        };
        let second = {
            let target = Arc::clone(&target);
            let template = Arc::clone(&template);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                RunVars::create(&target, &template).expect("second private variable store")
            })
        };
        barrier.wait();
        let first = first.join().expect("first firmware creator");
        let second = second.join().expect("second firmware creator");

        assert_ne!(first.path(), second.path());
        assert_eq!(
            std::fs::read(first.path()).unwrap(),
            b"pristine firmware variables"
        );
        assert_eq!(
            std::fs::read(second.path()).unwrap(),
            b"pristine firmware variables"
        );
        let first_path = first.path().to_path_buf();
        let second_path = second.path().to_path_buf();
        drop(first);
        drop(second);
        assert!(!first_path.exists());
        assert!(!second_path.exists());
    }

    #[test]
    fn private_firmware_refuses_a_redirected_run_root_before_writing() {
        let fixture = QemuFixture::create();
        let target = fixture.root.join("target");
        let outside = fixture.root.join("outside");
        std::fs::create_dir(&target).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"outside authority").unwrap();
        let run_root = target.join(crate::paths::QEMU_RUN_DIRECTORY);
        create_directory_symlink(&outside, &run_root);
        let reserved_outside = std::cell::Cell::new(false);

        assert!(
            RunVars::create_with(&target, &fixture.template, |_| reserved_outside.set(true))
                .is_err()
        );
        assert!(
            !reserved_outside.get(),
            "private firmware reserved an external output before refusal"
        );
        assert_eq!(
            crate::paths::directory_entries(&outside).unwrap().len(),
            1,
            "private firmware creation escaped its configured target"
        );
        assert_eq!(
            std::fs::read(outside.join("sentinel")).unwrap(),
            b"outside authority"
        );
        remove_directory_symlink(&run_root);
    }

    #[test]
    fn storage_construction_is_serialized_across_runners() {
        let fixture = QemuFixture::create();
        let lock_path = fixture.root.join("nvme.lock");
        let canonical = crate::mutation_workspace::canonical_existing_real_directory(
            &fixture.root,
            "storage fixture",
        )
        .unwrap();
        let held = acquire_storage_lock(&fixture.root, &lock_path, &canonical)
            .expect("first storage builder");
        let (contended_tx, contended_rx) = std::sync::mpsc::sync_channel(0);
        let (continue_tx, continue_rx) = std::sync::mpsc::sync_channel(0);
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let parent = fixture.root.clone();
        let canonical = canonical.clone();
        let contender = std::thread::spawn(move || {
            let _lock = acquire_storage_lock_with(&parent, &lock_path, &canonical, || {
                contended_tx.send(()).expect("announce contention");
                continue_rx.recv().expect("wait for release");
            })
            .expect("second storage builder");
            acquired_tx.send(()).expect("announce acquisition");
        });

        contended_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("contender observed the held lock");
        drop(held);
        continue_tx.send(()).expect("permit retry after release");
        acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("contender acquired released lock");
        contender.join().expect("storage contender");
    }

    #[test]
    fn an_open_storage_lock_must_remain_at_its_name() {
        let fixture = QemuFixture::create();
        let lock_path = fixture.root.join("nvme.lock");
        let canonical = crate::mutation_workspace::canonical_existing_real_directory(
            &fixture.root,
            "storage fixture",
        )
        .unwrap();

        let error = open_storage_lock_with(&fixture.root, &lock_path, &canonical, |opened| {
            std::fs::remove_file(opened).expect("unlink the opened lock")
        })
        .expect_err("an unlinked lock cannot serialize later builders");

        assert!(
            error
                .to_string()
                .contains("disappeared after it was opened")
        );
    }

    #[test]
    fn storage_publication_never_writes_through_an_external_link() {
        let fixture = QemuFixture::create();
        let parent = fixture.root.join("storage");
        let outside = fixture.root.join("outside");
        std::fs::create_dir(&parent).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let sentinel = outside.join("sentinel");
        let backing = parent.join("nvme.img");
        let lock = parent.join("nvme.lock");
        std::fs::write(&sentinel, b"outside authority").unwrap();

        std::fs::hard_link(&sentinel, &backing).unwrap();
        assert_eq!(publish_fixture(&parent, b"new namespace").unwrap(), backing);
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
        assert_eq!(std::fs::read(&backing).unwrap(), b"new namespace");

        std::fs::remove_file(&backing).unwrap();
        std::fs::write(&sentinel, b"same namespace").unwrap();
        std::fs::hard_link(&sentinel, &backing).unwrap();
        publish_fixture(&parent, b"same namespace").unwrap();
        std::fs::write(&backing, b"private namespace").unwrap();
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"same namespace");

        std::fs::remove_file(&backing).unwrap();
        std::fs::write(&sentinel, b"outside authority").unwrap();
        create_file_symlink(&sentinel, &backing);
        assert!(publish_fixture(&parent, b"forged").is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
        std::fs::remove_file(&backing).unwrap();

        std::fs::remove_file(&lock).unwrap();
        std::fs::hard_link(&sentinel, &lock).unwrap();
        publish_fixture(&parent, b"lock hardlink").unwrap();
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
        std::fs::remove_file(&backing).unwrap();
        std::fs::remove_file(&lock).unwrap();

        create_file_symlink(&sentinel, &lock);
        assert!(publish_fixture(&parent, b"lock symlink").is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
        std::fs::remove_file(&lock).unwrap();
    }

    #[test]
    fn storage_creation_refuses_a_redirected_parent_before_writing() {
        let fixture = QemuFixture::create();
        let target = fixture.root.join("target");
        let outside = fixture.root.join("outside-storage");
        std::fs::create_dir(&target).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"outside authority").unwrap();
        let storage = target.join("qemu storage,private");
        create_directory_symlink(&outside, &storage);

        assert!(storage_paths(&fixture.root).is_err());
        assert_eq!(crate::paths::directory_entries(&outside).unwrap().len(), 1);
        assert_eq!(
            std::fs::read(outside.join("sentinel")).unwrap(),
            b"outside authority"
        );
        remove_directory_symlink(&storage);
    }

    #[test]
    fn a_failed_storage_stage_preserves_the_complete_authoritative_image() {
        let fixture = QemuFixture::create();
        let parent = fixture.root.join("storage");
        std::fs::create_dir(&parent).unwrap();
        let backing = publish_fixture(&parent, b"complete old image").unwrap();

        let error = publish_namespace(&parent, &backing, |file| {
            file.write_all(b"partial")?;
            bail!("injected construction failure")
        })
        .unwrap_err();
        assert!(error.to_string().contains("injected construction failure"));
        assert_eq!(std::fs::read(&backing).unwrap(), b"complete old image");
        assert!(!parent.join(STORAGE_STAGING_NAME).exists());
    }

    #[test]
    fn storage_staging_supports_read_modify_write_builders() {
        let fixture = QemuFixture::create();
        let parent = fixture.root.join("storage");
        std::fs::create_dir(&parent).unwrap();
        let backing = parent.join("nvme.img");

        publish_namespace(&parent, &backing, |file| {
            file.write_all(b"format input")?;
            Seek::seek(&mut *file, SeekFrom::Start(0))?;
            let mut observed = Vec::new();
            file.read_to_end(&mut observed)?;
            ensure!(observed == b"format input", "staging handle changed bytes");
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::read(backing).unwrap(), b"format input");
    }

    #[test]
    fn the_nvme_namespace_is_streamed_to_its_exact_extent() {
        let fixture = QemuFixture::create();
        let path = fixture.root.join("streamed-nvme.img");
        let mut options = OpenOptions::new();
        OpenOptions::create_new(&mut options, true);
        OpenOptions::read(&mut options, true);
        OpenOptions::write(&mut options, true);
        let mut file = OpenOptions::open(&options, &path).unwrap();
        write_nvme_namespace(&mut file).unwrap();
        assert_eq!(file.metadata().unwrap().len(), NVME_BYTES);
        for lba in 0..NVME_SIGNED_BLOCKS {
            let mut signature = [0u8; 8];
            Seek::seek(&mut file, SeekFrom::Start(lba * NVME_BLOCK_BYTES as u64)).unwrap();
            file.read_exact(&mut signature).unwrap();
            assert_eq!(u64::from_le_bytes(signature), nvme_signature(lba));
        }
        let mut sector = [0u8; fat32::SECTOR_BYTES];
        Seek::seek(
            &mut file,
            SeekFrom::Start(NVME_VOLUME_SECTOR * NVME_BLOCK_BYTES as u64),
        )
        .unwrap();
        file.read_exact(&mut sector).unwrap();
        fat32::Bpb::parse(&sector).expect("streamed FAT32 corpus");
    }

    #[test]
    fn qemu_stop_polls_until_a_terminated_child_is_reaped() {
        let mut child = fake_qemu();
        child.exit_after_kill_polls = Some(3);
        stop_qemu_with(
            &mut child,
            FakeQemuChild::exited,
            FakeQemuChild::terminate,
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(child.killed);
        assert_eq!(child.polls_after_kill, 3);

        let mut exited = fake_qemu();
        exited.exited = true;
        stop_qemu_with(
            &mut exited,
            FakeQemuChild::exited,
            FakeQemuChild::terminate,
            Duration::ZERO,
        )
        .unwrap();
        assert!(!exited.killed);
    }

    #[test]
    fn qemu_stop_refuses_timeout_and_a_live_child_after_kill_refusal() {
        let mut stuck = fake_qemu();
        stuck.exit_after_kill_polls = None;
        let error = stop_qemu_with(
            &mut stuck,
            FakeQemuChild::exited,
            FakeQemuChild::terminate,
            Duration::ZERO,
        )
        .unwrap_err();
        assert!(error.to_string().contains("did not exit"));

        let mut refused = fake_qemu();
        refused.reject_kill = true;
        let error = stop_qemu_with(
            &mut refused,
            FakeQemuChild::exited,
            FakeQemuChild::terminate,
            Duration::ZERO,
        )
        .unwrap_err();
        assert!(error.to_string().contains("terminating QEMU"));

        let mut raced = fake_qemu();
        raced.reject_kill = true;
        raced.exit_on_rejected_kill = true;
        stop_qemu_with(
            &mut raced,
            FakeQemuChild::exited,
            FakeQemuChild::terminate,
            Duration::ZERO,
        )
        .unwrap();
    }

    fn passing_verdict() -> SelftestVerdict {
        SelftestVerdict {
            status: VerdictStatus::Pass,
            passed: EXPECTED_SELFTEST_CHECKS,
            failed: 0,
            expected: EXPECTED_SELFTEST_CHECKS,
        }
    }

    fn passing_stack_sample() -> StackSample {
        StackSample {
            used: 22_000,
            capacity: NVME_STACK_CAPACITY,
            headroom: NVME_STACK_CAPACITY - 22_000,
            leaf: 21_000,
        }
    }

    fn stack_line() -> &'static str {
        "[info ] selftest: nvme-stack v2 used=22000 capacity=32768 headroom=10768 leaf=21000\r\n"
    }

    #[test]
    fn a_failed_check_requires_one_exact_terminated_error_record() {
        for exact in [
            "[error] selftest: a bound space is not reclaimed FAILED\n",
            "[error] selftest: a bound space is not reclaimed FAILED\r\n",
        ] {
            assert_eq!(
                parse_failed_check(exact),
                Some("a bound space is not reclaimed")
            );
        }
        for near_miss in [
            "[info ] selftest: a thing FAILED\r\n",
            "[warn ] selftest: a thing FAILED\r\n",
            "prefix [error] selftest: a thing FAILED\r\n",
            "[error] prefix selftest: a thing FAILED\r\n",
            "[error] selftest: a thing FAILED suffix\r\n",
            "[error] selftest: a thing FAILED \r\n",
            "[error] selftest: a thing FAILED\t\r\n",
            "[error] selftest: a thing FAILED\u{a0}\r\n",
            "[error] selftest: a thing FAILED",
            "[error] selftest: a thing FAILED\r\r\n",
            "[error] selftest: a thing FAILED\rinside\r\n",
            "[error] selftest:  FAILED\r\n",
            "[error] selftest: verdict v1 FAIL passed=1 failed=0 expected=1\r\n",
            "[info ] selftest: a thing\r\n",
            "[info ] keyboard: up\r\n",
        ] {
            assert_eq!(
                parse_failed_check(near_miss),
                None,
                "accepted {near_miss:?}"
            );
        }
    }

    #[test]
    fn verdict_records_require_exact_framing_and_status_severity() {
        let pass = format!(
            "[info ] selftest: verdict v1 PASS passed={} failed=0 expected={}\r\n",
            EXPECTED_SELFTEST_CHECKS, EXPECTED_SELFTEST_CHECKS
        );
        assert_eq!(
            parse_selftest_verdict(&pass).unwrap().unwrap(),
            passing_verdict()
        );
        let fail = format!(
            "[error] selftest: verdict v1 FAIL passed={} failed=1 expected={}\n",
            EXPECTED_SELFTEST_CHECKS - 1,
            EXPECTED_SELFTEST_CHECKS
        );
        assert_eq!(
            parse_selftest_verdict(&fail).unwrap().unwrap(),
            SelftestVerdict {
                status: VerdictStatus::Fail,
                passed: EXPECTED_SELFTEST_CHECKS - 1,
                failed: 1,
                expected: EXPECTED_SELFTEST_CHECKS,
            }
        );

        for near_miss in [
            pass.trim_start_matches("[info ] ").to_string(),
            pass.replacen("[info ]", "[warn ]", 1),
            format!("prefix {pass}"),
            pass.replacen("[info ] selftest:", "[info ] prefix selftest:", 1),
            pass.trim_end_matches(['\r', '\n']).to_string(),
            pass.replace("\r\n", "\r\r\n"),
            pass.replacen("[info ]", "[error]", 1),
            fail.replacen("[error]", "[info ]", 1),
        ] {
            assert!(
                !matches!(parse_selftest_verdict(&near_miss), Some(Ok(_))),
                "accepted {near_miss:?}"
            );
        }
    }

    #[test]
    fn verdict_fields_require_single_ascii_spaces() {
        let exact = "[info ] selftest: verdict v1 PASS passed=1 failed=0 expected=1\r\n";
        assert!(matches!(parse_selftest_verdict(exact), Some(Ok(_))));
        for near_miss in [
            exact.replace("v1 PASS", "v1  PASS"),
            exact.replace("v1 PASS", "v1\tPASS"),
            exact.replace("v1 PASS", "v1\u{a0}PASS"),
            exact.replace(" expected=1", "  expected=1"),
            exact.replace("\r\n", " \r\n"),
            exact.replace("\r\n", "\t\r\n"),
            exact.replace("\r\n", "\u{a0}\r\n"),
        ] {
            assert!(
                !matches!(parse_selftest_verdict(&near_miss), Some(Ok(_))),
                "accepted {near_miss:?}"
            );
        }
    }

    #[test]
    fn verdict_counts_use_canonical_decimals() {
        for near_miss in [
            "[info ] selftest: verdict v1 PASS passed=01 failed=0 expected=1\r\n",
            "[info ] selftest: verdict v1 PASS passed=+1 failed=0 expected=1\r\n",
            "[info ] selftest: verdict v1 PASS passed=-1 failed=0 expected=1\r\n",
            "[info ] selftest: verdict v1 PASS passed=\u{0661} failed=0 expected=1\r\n",
            "[info ] selftest: verdict v1 PASS passed=18446744073709551616 failed=0 expected=1\r\n",
        ] {
            assert!(
                matches!(parse_selftest_verdict(near_miss), Some(Err(_))),
                "accepted {near_miss:?}"
            );
        }
    }

    fn baseline_line() -> &'static str {
        "[info ] selftest: baseline v1 boot_ns=11 frames_free=12 heap_slab=13 heap_large=14 heap_live=15 handles_committed=16 handles_live=17 trap_ticks=18 storage_bytes=4096 storage_ticks=19 counter_hz=20\r\n"
    }

    #[test]
    fn baseline_record_is_versioned_ordered_and_unique() {
        assert_eq!(
            parse_performance_sample(baseline_line()).unwrap().unwrap(),
            PerformanceSample {
                boot_ns: 11,
                frames_free: 12,
                heap_slab: 13,
                heap_large: 14,
                heap_live: 15,
                handles_committed: 16,
                handles_live: 17,
                trap_ticks: 18,
                storage_bytes: 4096,
                storage_ticks: 19,
                counter_hz: 20,
            }
        );
        for line in [
            baseline_line().replace("v1", "v2"),
            baseline_line().replace("boot_ns=11 frames_free=12", "frames_free=12 boot_ns=11"),
            baseline_line().replace(" counter_hz=20", ""),
            baseline_line().replace("\r\n", " trailing=true\r\n"),
        ] {
            assert!(parse_performance_sample(&line).unwrap().is_err(), "{line}");
        }

        let mut observation = SerialObservation::default();
        observe_selftest_line(&mut observation, baseline_line()).unwrap();
        assert!(observe_selftest_line(&mut observation, baseline_line()).is_err());
    }

    #[test]
    fn baseline_records_require_exact_info_framing() {
        for near_miss in [
            baseline_line().trim_start_matches("[info ] ").to_string(),
            baseline_line().replacen("[info ]", "[warn ]", 1),
            baseline_line().replacen("[info ]", "[error]", 1),
            format!("prefix {}", baseline_line()),
            baseline_line().replacen("selftest:", "prefix selftest:", 1),
            baseline_line().trim_end_matches(['\r', '\n']).to_string(),
            baseline_line().replace("\r\n", "\r\r\n"),
        ] {
            assert!(
                !matches!(parse_performance_sample(&near_miss), Some(Ok(_))),
                "accepted {near_miss:?}"
            );
        }
    }

    #[test]
    fn baseline_fields_require_single_ascii_spaces() {
        for near_miss in [
            baseline_line().replace("v1 boot_ns", "v1  boot_ns"),
            baseline_line().replace("v1 boot_ns", "v1\tboot_ns"),
            baseline_line().replace("v1 boot_ns", "v1\u{a0}boot_ns"),
            baseline_line().replace(" frames_free", "  frames_free"),
            baseline_line().replace("\r\n", " \r\n"),
            baseline_line().replace("\r\n", "\t\r\n"),
            baseline_line().replace("\r\n", "\u{a0}\r\n"),
        ] {
            assert!(
                matches!(parse_performance_sample(&near_miss), Some(Err(_))),
                "accepted {near_miss:?}"
            );
        }
    }

    #[test]
    fn baseline_values_use_canonical_decimals() {
        for near_miss in [
            baseline_line().replace("boot_ns=11", "boot_ns=011"),
            baseline_line().replace("boot_ns=11", "boot_ns=+11"),
            baseline_line().replace("boot_ns=11", "boot_ns=-11"),
            baseline_line().replace("boot_ns=11", "boot_ns=\u{0661}\u{0661}"),
            baseline_line().replace("boot_ns=11", "boot_ns=18446744073709551616"),
        ] {
            assert!(
                matches!(parse_performance_sample(&near_miss), Some(Err(_))),
                "accepted {near_miss:?}"
            );
        }
    }

    #[test]
    fn nvme_stack_record_is_versioned_ordered_canonical_and_unique() {
        assert_eq!(
            parse_stack_sample(stack_line()).unwrap().unwrap(),
            passing_stack_sample()
        );
        for line in [
            stack_line().replace("v2", "v1"),
            stack_line().replace(" leaf=21000", ""),
            stack_line().replace("leaf=21000", "leaf=021000"),
            stack_line().replace("headroom=10768 leaf=21000", "leaf=21000 headroom=10768"),
            stack_line().replace("used=22000 capacity=32768", "capacity=32768 used=22000"),
            stack_line().replace("used=22000", "used=022000"),
            stack_line().replace(" headroom=10768", ""),
            stack_line().replace("\r\n", " trailing=true\r\n"),
            stack_line().replacen("[info ]", "[warn ]", 1),
            stack_line().trim_end_matches(['\r', '\n']).to_string(),
        ] {
            assert!(
                !matches!(parse_stack_sample(&line), Some(Ok(_))),
                "accepted {line:?}"
            );
        }

        let mut observation = SerialObservation::default();
        observe_selftest_line(&mut observation, stack_line()).unwrap();
        assert!(observe_selftest_line(&mut observation, stack_line()).is_err());
    }

    #[test]
    fn a_panic_outcome_preserves_earlier_failures_and_its_exact_transcript() {
        let failed = "[error] selftest: an earlier ownership check FAILED\r\n";
        let line = "[error] KERNEL PANIC on thread 'boot': panicked at source\\nexact cause \t\r\n";
        let mut observation = SerialObservation::default();
        let panicked = AtomicBool::new(false);
        observe_selftest_line(&mut observation, failed)
            .unwrap()
            .publish(&panicked);
        observe_selftest_line(&mut observation, line)
            .unwrap()
            .publish(&panicked);
        assert!(!observation.panic_complete);
        assert!(!panicked.load(Ordering::Acquire));
        assert!(correlate_selftest(None, observation.clone()).is_err());
        assert!(observe_selftest_line(&mut observation, "continuation\r\n").is_err());

        let terminal = format!("[error] {COMPLETE_MARKER}\r\n");
        let transition = observe_selftest_line(&mut observation, &terminal).unwrap();
        assert_eq!(transition, PanicTransition::Completed);
        transition.publish(&panicked);
        assert!(panicked.load(Ordering::Acquire));
        assert_eq!(
            correlate_selftest(None, observation).unwrap(),
            SelftestOutcome::Panicked {
                transcript:
                    "[error] KERNEL PANIC on thread 'boot': panicked at source\\nexact cause \t"
                        .to_string(),
                failed: vec!["an earlier ownership check".to_string()],
            }
        );
    }

    #[test]
    fn a_failed_verdict_must_name_every_failed_check() {
        let observation = SerialObservation {
            verdict: Some(SelftestVerdict {
                status: VerdictStatus::Fail,
                passed: EXPECTED_SELFTEST_CHECKS - 1,
                failed: 1,
                expected: EXPECTED_SELFTEST_CHECKS,
            }),
            baseline: None,
            nvme_stack: None,
            qemu_tcg_isolation: true,
            failed: Vec::new(),
            panic: None,
            panic_complete: false,
        };
        let error = correlate_selftest(Some(SELFTEST_FAILED), observation).unwrap_err();
        assert!(error.to_string().contains("counted 1 failures but named 0"));
    }

    #[test]
    fn malformed_or_unknown_verdicts_are_rejected() {
        for line in [
            "[info ] selftest: verdict v2 PASS passed=76 failed=0 expected=76\r\n",
            "[info ] selftest: verdict v1 MAYBE passed=76 failed=0 expected=76\r\n",
            "[info ] selftest: verdict v1 PASS passed=many failed=0 expected=76\r\n",
            "[info ] selftest: verdict v1 PASS passed=76 failed=0 expected=76 trailing\r\n",
        ] {
            assert!(parse_selftest_verdict(line).unwrap().is_err(), "{line}");
        }
    }

    #[test]
    fn a_second_final_verdict_is_rejected() {
        let line = format!(
            "[info ] selftest: verdict v1 PASS passed={} failed=0 expected={}\r\n",
            EXPECTED_SELFTEST_CHECKS, EXPECTED_SELFTEST_CHECKS
        );
        let mut observation = SerialObservation::default();
        observe_selftest_line(&mut observation, &line).unwrap();
        assert!(observe_selftest_line(&mut observation, &line).is_err());
    }

    #[test]
    fn failed_check_names_are_unique_and_precede_the_terminal_verdict() {
        let failed = "[error] selftest: exact failed check FAILED\r\n";
        let mut duplicate = SerialObservation::default();
        observe_selftest_line(&mut duplicate, failed).unwrap();
        assert!(observe_selftest_line(&mut duplicate, failed).is_err());

        let verdict = format!(
            "[info ] selftest: verdict v1 PASS passed={} failed=0 expected={}\r\n",
            EXPECTED_SELFTEST_CHECKS, EXPECTED_SELFTEST_CHECKS
        );
        let mut terminal = SerialObservation::default();
        observe_selftest_line(&mut terminal, &verdict).unwrap();
        assert!(observe_selftest_line(&mut terminal, failed).is_err());
        assert!(observe_selftest_line(&mut terminal, "[info ] after verdict\r\n").is_err());
    }

    #[test]
    fn a_valid_serial_pass_must_match_debug_exit() {
        let observation = SerialObservation {
            failed: Vec::new(),
            verdict: Some(passing_verdict()),
            baseline: None,
            nvme_stack: Some(passing_stack_sample()),
            qemu_tcg_isolation: true,
            panic: None,
            panic_complete: false,
        };
        assert!(correlate_selftest(Some(SELFTEST_PASSED), observation).is_ok());

        let observation = SerialObservation {
            failed: Vec::new(),
            verdict: Some(passing_verdict()),
            baseline: None,
            nvme_stack: Some(passing_stack_sample()),
            qemu_tcg_isolation: true,
            panic: None,
            panic_complete: false,
        };
        let error = correlate_selftest(Some(SELFTEST_FAILED), observation).unwrap_err();
        assert!(error.to_string().contains("debug-exit"));
    }

    #[test]
    fn a_passing_selftest_requires_bounded_conservative_nvme_stack_evidence() {
        for (sample, expected) in [
            (None, "without an NVMe stack record"),
            (
                Some(StackSample {
                    capacity: NVME_STACK_CAPACITY - 1,
                    ..passing_stack_sample()
                }),
                "expected 32768",
            ),
            (
                Some(StackSample {
                    headroom: passing_stack_sample().headroom - 1,
                    ..passing_stack_sample()
                }),
                "does not conserve",
            ),
            (
                Some(StackSample {
                    leaf: passing_stack_sample().used + 1,
                    ..passing_stack_sample()
                }),
                "exceeds its painted high-water mark",
            ),
            (
                Some(StackSample {
                    used: NVME_STACK_CAPACITY - NVME_STACK_MIN_HEADROOM + 1,
                    headroom: NVME_STACK_MIN_HEADROOM - 1,
                    ..passing_stack_sample()
                }),
                "retained only",
            ),
        ] {
            let observation = SerialObservation {
                failed: Vec::new(),
                verdict: Some(passing_verdict()),
                baseline: None,
                nvme_stack: sample,
                qemu_tcg_isolation: true,
                panic: None,
                panic_complete: false,
            };
            let error = correlate_selftest(Some(SELFTEST_PASSED), observation).unwrap_err();
            assert!(error.to_string().contains(expected), "{error:#}");
        }
    }

    #[test]
    fn a_passing_selftest_requires_the_exact_consumed_tcg_policy() {
        let without_policy = SerialObservation {
            failed: Vec::new(),
            verdict: Some(passing_verdict()),
            baseline: None,
            nvme_stack: Some(passing_stack_sample()),
            qemu_tcg_isolation: false,
            panic: None,
            panic_complete: false,
        };
        let error = correlate_selftest(Some(SELFTEST_PASSED), without_policy).unwrap_err();
        assert!(error.to_string().contains("without consuming"));

        for near_miss in [
            QEMU_TCG_ISOLATION_MARKER.to_string(),
            format!("[warn ] {QEMU_TCG_ISOLATION_MARKER}"),
            format!("[info ] {QEMU_TCG_ISOLATION_MARKER} trailing"),
            "[info ] cpu: user isolation shared kernel mappings: qemu-tcg".to_string(),
        ] {
            assert!(!is_exact_log_message(
                &near_miss,
                LogSeverity::Info,
                QEMU_TCG_ISOLATION_MARKER
            ));
        }

        let exact = format!("[info ] {QEMU_TCG_ISOLATION_MARKER}\r\n");
        let mut observation = SerialObservation::default();
        observe_selftest_line(&mut observation, &exact).unwrap();
        observe_selftest_line(&mut observation, stack_line()).unwrap();
        observation.verdict = Some(passing_verdict());
        assert_eq!(
            correlate_selftest(Some(SELFTEST_PASSED), observation).unwrap(),
            SelftestOutcome::Passed
        );

        let mut duplicate = SerialObservation::default();
        observe_selftest_line(&mut duplicate, &exact).unwrap();
        assert!(observe_selftest_line(&mut duplicate, &exact).is_err());
    }

    #[test]
    fn zero_or_skipped_checks_cannot_pass() {
        for passed in [0, EXPECTED_SELFTEST_CHECKS - 1] {
            let observation = SerialObservation {
                failed: Vec::new(),
                verdict: Some(SelftestVerdict {
                    status: VerdictStatus::Pass,
                    passed,
                    failed: 0,
                    expected: EXPECTED_SELFTEST_CHECKS,
                }),
                baseline: None,
                nvme_stack: Some(passing_stack_sample()),
                qemu_tcg_isolation: true,
                panic: None,
                panic_complete: false,
            };
            let error = correlate_selftest(Some(SELFTEST_PASSED), observation).unwrap_err();
            assert!(error.to_string().contains("counted"));
        }
    }

    #[test]
    fn harness_and_kernel_must_agree_on_the_expected_count() {
        let observation = SerialObservation {
            failed: Vec::new(),
            verdict: Some(SelftestVerdict {
                expected: EXPECTED_SELFTEST_CHECKS - 1,
                passed: EXPECTED_SELFTEST_CHECKS - 1,
                ..passing_verdict()
            }),
            baseline: None,
            nvme_stack: Some(passing_stack_sample()),
            qemu_tcg_isolation: true,
            panic: None,
            panic_complete: false,
        };
        let error = correlate_selftest(Some(SELFTEST_PASSED), observation).unwrap_err();
        assert!(error.to_string().contains("harness requires"));
    }

    #[test]
    fn serial_status_must_agree_with_its_counts() {
        let observation = SerialObservation {
            failed: Vec::new(),
            verdict: Some(SelftestVerdict {
                status: VerdictStatus::Fail,
                ..passing_verdict()
            }),
            baseline: None,
            nvme_stack: Some(passing_stack_sample()),
            qemu_tcg_isolation: true,
            panic: None,
            panic_complete: false,
        };
        let error = correlate_selftest(Some(SELFTEST_FAILED), observation).unwrap_err();
        assert!(error.to_string().contains("contradicts its counts"));
    }

    #[test]
    fn a_debug_exit_without_a_serial_verdict_is_rejected() {
        let error =
            correlate_selftest(Some(SELFTEST_PASSED), SerialObservation::default()).unwrap_err();
        assert!(error.to_string().contains("without a serial"));
    }

    #[test]
    fn irq_statistics_sum_matching_controllers() {
        let report = "IRQ statistics for i8259:\r\n 0: 12\r\n 1: 2\r\n\
                      IRQ statistics for ioapic:\r\n IRQ1: 3\r\n 12: 99\r\n(qemu)";
        assert_eq!(irq_count(report, 1), Some(5));
    }

    #[test]
    fn non_irq_monitor_output_is_not_a_zero_count() {
        assert_eq!(irq_count("unknown command: info irq\r\n(qemu)", 1), None);
    }

    #[test]
    fn the_exit_codes_are_the_kernels_doubled() {
        fn qemu_exit_code(written: i32) -> i32 {
            (written << 1) | 1
        }

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let debug_exit = std::fs::read_to_string(root.join("kernel/src/arch/x86_64/debug_exit.rs"))
            .expect("debug_exit.rs");

        let value = |name: &str| -> i32 {
            let marked = format!("const {name}: u32 = ");
            let rest = debug_exit
                .split_once(&marked)
                .unwrap_or_else(|| panic!("no `{name}` in debug_exit.rs"))
                .1;
            let literal = rest
                .split_once(';')
                .expect("a terminated constant")
                .0
                .trim()
                .trim_start_matches("0x");
            i32::from_str_radix(literal, 16).expect("a hex literal")
        };

        assert_eq!(
            qemu_exit_code(value("PASSED")),
            SELFTEST_PASSED,
            "the kernel's PASSED does not arrive as the code the harness waits for"
        );
        assert_eq!(
            qemu_exit_code(value("FAILED")),
            SELFTEST_FAILED,
            "the kernel's FAILED does not arrive as the code the harness waits for"
        );
        assert_ne!(SELFTEST_PASSED, SELFTEST_FAILED);
    }

    #[derive(Debug, PartialEq, Eq)]
    struct DevfsEntry {
        variant: String,
        name: Vec<u8>,
        description: String,
    }

    fn device_variant(path: &syn::Path, owner: &str) -> Result<String> {
        ensure!(
            path.segments.len() == 2
                && path.segments[0].ident == owner
                && path
                    .segments
                    .iter()
                    .all(|segment| segment.arguments.is_empty()),
            "expected {owner}::Variant"
        );
        Ok(path.segments[1].ident.to_string())
    }

    fn devfs_method<'a>(syntax: &'a syn::File, name: &str) -> Result<&'a syn::ImplItemFn> {
        let methods = syntax
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Impl(item) if item.trait_.is_none() => Some(item),
                _ => None,
            })
            .filter(|item| {
                matches!(
                    item.self_ty.as_ref(),
                    syn::Type::Path(path)
                        if path.qself.is_none() && path.path.is_ident("Device")
                )
            })
            .flat_map(|item| &item.items)
            .filter_map(|item| match item {
                syn::ImplItem::Fn(method) if method.sig.ident == name => Some(method),
                _ => None,
            })
            .collect::<Vec<_>>();
        ensure!(methods.len() == 1, "expected one Device::{name} method");
        Ok(methods[0])
    }

    fn devfs_match<T>(
        syntax: &syn::File,
        method: &str,
        literal: impl Fn(&syn::Lit) -> Option<T>,
    ) -> Result<std::collections::BTreeMap<String, T>> {
        let body = &devfs_method(syntax, method)?.block.stmts;
        let [syn::Stmt::Expr(syn::Expr::Match(matched), None)] = body.as_slice() else {
            bail!("Device::{method} must contain exactly one match expression");
        };
        ensure!(
            matches!(
                matched.expr.as_ref(),
                syn::Expr::Path(path) if path.qself.is_none() && path.path.is_ident("self")
            ),
            "Device::{method} must match self"
        );

        let mut values = std::collections::BTreeMap::new();
        for arm in &matched.arms {
            ensure!(arm.guard.is_none(), "Device::{method} has a guarded arm");
            let syn::Pat::Path(pattern) = &arm.pat else {
                bail!("Device::{method} has a non-variant pattern");
            };
            ensure!(
                pattern.qself.is_none(),
                "Device::{method} has a qualified pattern"
            );
            let variant = device_variant(&pattern.path, "Self")?;
            let syn::Expr::Lit(value) = arm.body.as_ref() else {
                bail!("Device::{method}::{variant} is not a literal");
            };
            let value = literal(&value.lit).with_context(|| {
                format!("Device::{method}::{variant} has the wrong literal kind")
            })?;
            ensure!(
                values.insert(variant.clone(), value).is_none(),
                "Device::{method} repeats {variant}"
            );
        }
        Ok(values)
    }

    fn devfs_contract(source: &str) -> Result<Vec<DevfsEntry>> {
        let syntax = syn::parse_file(source).context("parsing kernel/src/devfs.rs")?;
        let enums = syntax
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Enum(item) if item.ident == "Device" => Some(item),
                _ => None,
            })
            .collect::<Vec<_>>();
        ensure!(enums.len() == 1, "expected one Device enum");

        let mut variants = std::collections::BTreeSet::new();
        for variant in &enums[0].variants {
            ensure!(
                matches!(variant.fields, syn::Fields::Unit),
                "Device::{} is not a unit variant",
                variant.ident
            );
            ensure!(
                variants.insert(variant.ident.to_string()),
                "Device repeats {}",
                variant.ident
            );
        }

        let declarations = syntax
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Const(item) if item.ident == "DEVICES" => Some(item),
                _ => None,
            })
            .collect::<Vec<_>>();
        ensure!(declarations.len() == 1, "expected one DEVICES constant");
        let syn::Expr::Array(array) = declarations[0].expr.as_ref() else {
            bail!("DEVICES is not an array literal");
        };
        let mut listed = Vec::new();
        for element in &array.elems {
            let syn::Expr::Path(path) = element else {
                bail!("DEVICES contains a non-variant expression");
            };
            ensure!(
                path.qself.is_none(),
                "DEVICES contains a qualified expression"
            );
            listed.push(device_variant(&path.path, "Device")?);
        }
        let listed_set = listed
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        ensure!(
            listed.len() == listed_set.len(),
            "DEVICES contains a duplicate variant"
        );
        ensure!(
            listed_set == variants,
            "DEVICES does not match Device variants: listed {listed_set:?}, declared {variants:?}"
        );

        let mut names = devfs_match(&syntax, "name", |literal| match literal {
            syn::Lit::ByteStr(value) => Some(value.value()),
            _ => None,
        })?;
        let mut descriptions = devfs_match(&syntax, "describes", |literal| match literal {
            syn::Lit::Str(value) => Some(value.value()),
            _ => None,
        })?;
        ensure!(
            names
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                == variants,
            "Device::name does not match Device variants"
        );
        ensure!(
            descriptions
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                == variants,
            "Device::describes does not match Device variants"
        );

        let mut unique_names = std::collections::BTreeSet::new();
        listed
            .into_iter()
            .map(|variant| {
                let name = names
                    .remove(&variant)
                    .context("a listed device has no name")?;
                let description = descriptions
                    .remove(&variant)
                    .context("a listed device has no description")?;
                ensure!(!name.is_empty(), "Device::{variant} has an empty name");
                ensure!(
                    unique_names.insert(name.clone()),
                    "Device::{variant} repeats a device name"
                );
                ensure!(
                    !description.is_empty(),
                    "Device::{variant} has an empty description"
                );
                Ok(DevfsEntry {
                    variant,
                    name,
                    description,
                })
            })
            .collect()
    }

    fn devfs_fixture(variants: &str, listed: &str, names: &str, descriptions: &str) -> String {
        format!(
            "enum Device {{ {variants} }}\n\
             impl Device {{\n\
                 fn name(self) -> &'static [u8] {{ match self {{ {names} }} }}\n\
                 fn describes(self) -> &'static str {{ match self {{ {descriptions} }} }}\n\
             }}\n\
             const DEVICES: [Device; 3] = [{listed}];\n"
        )
    }

    #[test]
    fn a_complete_devfs_inventory_is_accepted() {
        let fixture = devfs_fixture(
            "Storage, Console, Timer",
            "Device::Storage, Device::Console, Device::Timer",
            "Self::Storage => b\"storage\", Self::Console => b\"console\", Self::Timer => b\"timer\",",
            "Self::Storage => \"block\", Self::Console => \"character\", Self::Timer => \"clock\",",
        );
        assert_eq!(
            devfs_contract(&fixture).expect("complete inventory").len(),
            3
        );
    }

    #[test]
    fn a_devfs_variant_omitted_from_the_listing_is_refused() {
        let fixture = devfs_fixture(
            "Storage, Console, Timer",
            "Device::Storage, Device::Console",
            "Self::Storage => b\"storage\", Self::Console => b\"console\", Self::Timer => b\"timer\",",
            "Self::Storage => \"block\", Self::Console => \"character\", Self::Timer => \"clock\",",
        );
        let error = devfs_contract(&fixture).expect_err("Timer is absent from DEVICES");
        assert!(error.to_string().contains("DEVICES does not match"));
    }

    #[test]
    fn a_devfs_variant_omitted_from_either_description_is_refused() {
        for (names, descriptions, expected) in [
            (
                "Self::Storage => b\"storage\", Self::Console => b\"console\",",
                "Self::Storage => \"block\", Self::Console => \"character\", Self::Timer => \"clock\",",
                "Device::name",
            ),
            (
                "Self::Storage => b\"storage\", Self::Console => b\"console\", Self::Timer => b\"timer\",",
                "Self::Storage => \"block\", Self::Console => \"character\",",
                "Device::describes",
            ),
        ] {
            let fixture = devfs_fixture(
                "Storage, Console, Timer",
                "Device::Storage, Device::Console, Device::Timer",
                names,
                descriptions,
            );
            assert!(
                devfs_contract(&fixture)
                    .expect_err("a descriptive match omitted Timer")
                    .to_string()
                    .contains(expected)
            );
        }
    }

    #[test]
    fn duplicate_devfs_authority_is_refused() {
        let duplicate_listing = devfs_fixture(
            "Storage, Console",
            "Device::Storage, Device::Console, Device::Storage",
            "Self::Storage => b\"storage\", Self::Console => b\"console\",",
            "Self::Storage => \"block\", Self::Console => \"character\",",
        );
        assert!(
            devfs_contract(&duplicate_listing)
                .expect_err("duplicate listing")
                .to_string()
                .contains("duplicate variant")
        );

        let duplicate_name = devfs_fixture(
            "Storage, Console, Timer",
            "Device::Storage, Device::Console, Device::Timer",
            "Self::Storage => b\"device\", Self::Console => b\"console\", Self::Timer => b\"device\",",
            "Self::Storage => \"block\", Self::Console => \"character\", Self::Timer => \"clock\",",
        );
        assert!(
            devfs_contract(&duplicate_name)
                .expect_err("duplicate name")
                .to_string()
                .contains("repeats a device name")
        );
    }

    #[test]
    fn the_devfs_source_inventory_matches_every_variant() {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let source =
            std::fs::read_to_string(repository.join("kernel/src/devfs.rs")).expect("devfs.rs");
        devfs_contract(&source).expect("complete devfs source inventory");
    }

    #[test]
    fn the_serial_contract_matches_the_kernel_source() {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let root = repository.join("kernel").join("src");
        let selftest = std::fs::read_to_string(root.join("selftest.rs")).expect("selftest.rs");
        let main = std::fs::read_to_string(root.join("main.rs")).expect("main.rs");
        let devfs = std::fs::read_to_string(root.join("devfs.rs")).expect("devfs.rs");
        let tmpfs = std::fs::read_to_string(root.join("tmpfs.rs")).expect("tmpfs.rs");
        let panic = std::fs::read_to_string(root.join("panic.rs")).expect("panic.rs");
        let user = std::fs::read_to_string(root.join("user.rs")).expect("user.rs");
        let apic = std::fs::read_to_string(repository.join("wost/src/arch/x86_64/apic.rs"))
            .expect("wost apic.rs");
        let cpu = std::fs::read_to_string(repository.join("wost/src/arch/x86_64/cpu.rs"))
            .expect("wost cpu.rs");
        let cpuid = std::fs::read_to_string(repository.join("libs/x86-cpuid/src/lib.rs"))
            .expect("x86-cpuid lib.rs");

        let constant = |source: &str, name: &str| -> String {
            source
                .lines()
                .find_map(|line| line.trim().strip_prefix(&format!("const {name}: usize = ")))
                .map(|value| value.trim_end_matches(';').to_string())
                .unwrap_or_else(|| panic!("no `{name}` in the kernel source"))
        };

        assert_eq!(
            constant(&selftest, "EXPECTED_CHECKS"),
            EXPECTED_SELFTEST_CHECKS.to_string(),
            "the check count drifted"
        );
        assert_eq!(
            format!("v{}", constant(&selftest, "VERDICT_VERSION")),
            SELFTEST_VERDICT_VERSION,
            "the verdict version drifted"
        );
        assert!(
            selftest.contains("info!(\"selftest: {name}\");")
                && selftest.contains("error!(\"selftest: {name} FAILED\");"),
            "the exact self-test check records drifted from their producer"
        );
        assert!(
            selftest.contains("\"selftest: verdict v{} PASS passed={} failed={} expected={}\"")
                && selftest
                    .contains("\"selftest: verdict v{} FAIL passed={} failed={} expected={}\""),
            "the exact self-test verdict records drifted from their producers"
        );
        assert!(
            selftest.contains(
                "\"selftest: baseline v1 boot_ns={} frames_free={} heap_slab={} heap_large={} heap_live={} handles_committed={} handles_live={} trap_ticks={} storage_bytes={} storage_ticks={} counter_hz={}\""
            ),
            "the exact self-test baseline record drifted from its producer"
        );
        assert!(
            selftest.contains("fn awaiting(marker: &str, press: &str) {\n    info!(\"{marker}\");")
                && selftest
                    .contains("awaiting(\n        \"selftest: awaiting storage-delay keystroke\",")
                && selftest
                    .contains("awaiting(\n        \"selftest: awaiting injected keystroke\",")
                && selftest
                    .contains("awaiting(\n        \"selftest: awaiting translated keystroke\","),
            "the exact interactive info records drifted from their producer"
        );
        assert!(
            selftest.contains(
                "let woke_through_polling = runs_until(1_000, || {\n        READER_SAW.load(Ordering::Acquire) != 0 && input::poller_passes() > passes_before\n    });"
            ),
            "the polled-reader witness does not await both consequences"
        );
        assert!(
            main.contains("\"kernel: reading the keyboard, uptime {} ms\"")
                && SETTLED_MARKER == "kernel: reading the keyboard, uptime ",
            "the exact steady-state record drifted from its producer"
        );
        assert!(
            main.contains("info!(\"cpu: user isolation shared kernel mappings: {reason}\")")
                && cpuid.contains("SharedKernelMappingReasonKind::QemuTcg => \"qemu_tcg\"")
                && QEMU_TCG_ISOLATION_MARKER
                    == "cpu: user isolation shared kernel mappings: qemu_tcg",
            "the exact QEMU TCG evidence drifted from its producers"
        );
        assert!(
            main.contains("info!(\"nvme: candidate {ordinal} retired cleanly; trying next\")")
                && NVME_FALLBACK_MARKER == "nvme: candidate 1 retired cleanly; trying next",
            "the exact first-candidate retirement record drifted from its producer"
        );
        assert!(
            main.contains("warn!(\"storage: no DMA isolation\")")
                && NO_ISOLATION_MARKER == "storage: no DMA isolation"
                && main
                    .contains("StorageSelection::Seeking => info!(\"nvme: no usable controller\")")
                && NO_USABLE_CONTROLLER_MARKER == "nvme: no usable controller",
            "the exact unisolated refusal records drifted from their producers"
        );
        assert_eq!(
            UNISOLATED_REQUIRED_MARKERS.len(),
            4,
            "the unisolated boot requires the TCG policy, the refusal, the clean retirement and the empty selection"
        );
        let qemu = include_str!("qemu.rs");
        let storage = qemu
            .split_once("fn storage(root: &Path)")
            .expect("the QEMU storage profile")
            .1
            .split_once("const SELFTEST_PASSED")
            .expect("the end of the QEMU storage profile")
            .0;
        assert!(
            storage.find("nvme,serial=wizard-empty")
                < storage.find("nvme,serial=wizard0,drive=wizardnvm")
                && main.contains(
                    "storage += 1;\n            if matches!(selection, StorageSelection::Seeking) {\n                selection = try_controller(function.at, storage);"
                ),
            "QEMU's deliberately empty controller is no longer candidate 1"
        );
        assert!(
            main.contains("warn!(\"pci: the configuration ports are already claimed; no bus\")"),
            "the exact no-PCI warning drifted from its producer"
        );
        let no_x2apic_reason = NO_X2APIC_DIAGNOSIS
            .strip_prefix("local apic: ")
            .and_then(|message| message.strip_suffix("; interrupts stay off"))
            .expect("the no-x2APIC diagnosis has the canonical envelope");
        assert!(
            main.contains("warn!(\"local apic: {reason}; interrupts stay off\")")
                && apic.contains(&format!(
                    "Self::NoExtendedMode => core::fmt::Write::write_str(\n                f,\n                \"{no_x2apic_reason}\",\n            ),"
                )),
            "the exact no-x2APIC diagnosis drifted from its two producers"
        );
        assert!(
            main.contains("warn!(\"cpu: extended state unavailable; ring 3 stays closed\")"),
            "the exact extended-state warning drifted from its producer"
        );
        let user_reason = DEGRADED_USER_MARKER
            .strip_prefix("user: no program: ")
            .expect("the degraded user marker has the boot prefix");
        assert!(
            main.contains("Err(reason) => warn!(\"user: no program: {reason}\")")
                && user.contains(user_reason),
            "the kernel does not compose {DEGRADED_USER_MARKER:?}"
        );
        assert_eq!(
            DEGRADED_FILESYSTEM_MARKER,
            "tmpfs: 1 file(s) and 1 directory(ies) at the root"
        );
        assert!(
            main.contains(
                "info!(\"tmpfs: {files} file(s) and {directories} directory(ies) at the root\")"
            ) && tmpfs.contains(
                "add(\n        &mut arena,\n        ROOT,\n        b\"MOTD\",\n        Some("
            ) && tmpfs.contains("let notes = add(&mut arena, ROOT, b\"NOTES\", None);")
                && tmpfs.matches("        ROOT,\n").count() == 1
                && tmpfs.matches("add(&mut arena, ROOT,").count() == 1,
            "the exact degraded tmpfs success record drifted from its producers"
        );
        let devices = devfs_contract(&devfs).expect("complete devfs source inventory");
        let storage = devices
            .iter()
            .find(|device| device.variant == "Storage")
            .expect("a Storage device");
        let console = devices
            .iter()
            .find(|device| device.variant == "Console")
            .expect("a Console device");
        let storage_record = format!(
            "devfs: {} is {}",
            core::str::from_utf8(&storage.name).expect("an ASCII storage name"),
            storage.description
        );
        let console_record = format!(
            "devfs: {} is {}",
            core::str::from_utf8(&console.name).expect("an ASCII console name"),
            console.description
        );
        assert!(
            main.contains("Ok(vfs::Node::Device(found)) => info!(")
                && main.contains("\"devfs: {} is {}\"")
                && storage_record == DEGRADED_STORAGE_MARKER
                && console_record == DEGRADED_CONSOLE_MARKER,
            "the devfs degraded markers no longer compose from the kernel sources"
        );
        assert!(
            main.contains("info!(\"cpu: protections {protections}\")")
                && cpu.contains(
                    "(self.write_protect, \"wp\"),\n            (self.smep, \"smep\"),\n            (self.smap, \"smap\"),\n            (self.umip, \"umip\"),\n            (self.nx, \"nx\"),"
                )
                && DEGRADED_PROTECTIONS_MARKER == "cpu: protections wp nx",
            "the degraded protection record drifted from its ordered producer"
        );
        let watcher = qemu
            .split_once("fn watch_serial(")
            .expect("the serial watcher")
            .1
            .split_once("fn finish_boot_serial(")
            .expect("the end of the serial watcher")
            .0;
        assert!(
            watcher.contains("if is_settled_record(&text)")
                && watcher.contains("is_required_boot_marker(&text, *record)")
                && !watcher.contains("text.contains(SETTLED_MARKER)"),
            "the serial watcher bypasses exact boot-record matching"
        );
        let forwarder = qemu
            .split_once("fn forward_serial(")
            .expect("the self-test serial forwarder")
            .1
            .split_once("fn finish_selftest_serial(")
            .expect("the end of the self-test serial forwarder")
            .0;
        assert!(
            forwarder.contains("match observe_interactive_marker(")
                && forwarder.contains("&mut next_interactive_marker,")
                && forwarder.contains("marker_order,")
                && forwarder
                    .contains("markers_seen.store(next_interactive_marker, Ordering::Release)")
                && !forwarder.contains("text.contains(STORAGE_DELAY_MARKER)")
                && !forwarder.contains("text.contains(READY_MARKER)")
                && !forwarder.contains("text.contains(TRANSLATED_MARKER)"),
            "the self-test forwarder bypasses ordered exact interactive records"
        );
        assert!(
            panic.contains("use panic_record::write_record;")
                && panic.contains("let _ = write_record(uart, thread, info);"),
            "the kernel does not consume the shared panic-record contract"
        );
    }

    #[test]
    fn the_uart_reports_every_refused_byte_to_its_formatter() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let uart = std::fs::read_to_string(root.join("kernel/src/arch/x86_64/uart.rs"))
            .expect("kernel uart.rs");

        assert!(
            uart.contains(
                "fn write_byte(&mut self, byte: u8) -> fmt::Result {\n        if !self.wait_for(line_status::TRANSMIT_HOLDING_EMPTY) {\n            return Err(fmt::Error);\n        }\n        PortBlock::write(&mut self.registers, register::DATA, byte);\n        Ok(())\n    }"
            ),
            "the UART can report success after refusing a byte"
        );
        assert!(
            uart.contains("self.write_byte(b'\\r')?;") && uart.contains("self.write_byte(byte)?;"),
            "the UART formatter discards a byte-write refusal"
        );
    }

    #[test]
    fn the_namespaces_signature_is_the_one_the_kernel_looks_for() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let selftest =
            std::fs::read_to_string(root.join("kernel/src/selftest.rs")).expect("selftest.rs");

        let kernel = selftest
            .lines()
            .find_map(|line| line.trim().strip_prefix("0x5749_5A44_0000_0000 | "))
            .expect("no disk signature in selftest.rs");
        assert_eq!(
            kernel, "lba",
            "the kernel's signature is not the base value ored with the block"
        );

        assert_eq!(super::nvme_signature(0), 0x5749_5A44_0000_0000);
        for lba in [0u64, 1, 3, 7, super::NVME_SIGNED_BLOCKS - 1] {
            assert_eq!(
                super::nvme_signature(lba),
                0x5749_5A44_0000_0000 | lba,
                "block {lba}"
            );
        }
        // Distinct signatures expose an LBA ignored by the reader.
        for lba in 0..super::NVME_SIGNED_BLOCKS {
            for earlier in 0..lba {
                assert_ne!(super::nvme_signature(lba), super::nvme_signature(earlier));
            }
        }
    }

    #[test]
    fn nvme_cleanup_and_bus_master_authority_precede_the_enable_write() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let source = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/nvme.rs")).expect("kernel nvme source"),
        );
        let finish = source
            .split_once("fn finish_preparation(")
            .and_then(|(_, source)| source.split_once("fn retire_disabled("))
            .map(|(source, _)| source)
            .expect("no bounded controller-preparation completion body");
        let finish_owner = finish
            .find("let controller = Controller {")
            .expect("no cleanup owner in controller-preparation completion");
        assert!(
            !finish[..finish_owner].contains("write("),
            "controller preparation writes hardware before cleanup owns every published queue"
        );
        let preparation = source
            .find("let preparation = ControllerPreparation::new(registers);")
            .expect("no preparation owner for the mapped controller");
        let owner = source
            .find("let controller = Controller {")
            .expect("no cleanup-owning controller construction");
        let prepared = source
            .find("PrepareOutcome::Prepared(DisabledController {\n        controller,")
            .expect("the cleanup owner is not retained by the disabled controller");
        let admission = source
            .find("match device.admit_bus_master(authority, domain, interrupts) {")
            .expect("no verified bus-master transition");
        let mint = source
            .find("Ok(()) => BusMasterAdmission::Admitted(BusMasterAdmittedController(self))")
            .expect("successful bus-master admission does not mint start authority");
        let start = source
            .find("pub(crate) fn start(")
            .expect("no typed controller start door");
        let consume = source[start..]
            .find("let BusMasterAdmittedController(DisabledController {")
            .map(|offset| start + offset)
            .expect("controller start does not consume admitted ownership");
        let enable = source[start..]
            .find("write(registers, register::CC, configuration)")
            .map(|offset| start + offset)
            .expect("no controller enable write through the cleanup owner");

        assert!(
            preparation < owner
                && owner < prepared
                && prepared < start
                && start < consume
                && consume < enable,
            "controller enable is not ordered after typed cleanup ownership"
        );
        assert!(
            source
                .contains("pub(crate) fn admit_bus_master(\n        mut self,\n        device: &mut crate::pci::Device,")
                && source.contains(") -> BusMasterAdmission")
                && source
                    .matches(
                        "Ok(()) => BusMasterAdmission::Admitted(BusMasterAdmittedController(self))"
                    )
                    .count()
                    == 1
                && admission < mint
                && source[start..].contains("prepared: BusMasterAdmittedController")
                && !source[start..consume].contains("prepared: DisabledController"),
            "controller start can bypass consumed bus-master admission authority"
        );
        assert!(
            source
                .matches("write(registers, register::CC, configuration)")
                .count()
                == 1,
            "controller enable is not confined to the cleanup-owning start path"
        );
    }

    #[test]
    fn the_corpus_the_kernel_looks_for_is_the_one_that_is_written() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let selftest =
            std::fs::read_to_string(root.join("kernel/src/selftest.rs")).expect("selftest.rs");
        let fs = std::fs::read_to_string(root.join("kernel/src/fs.rs")).expect("fs.rs");
        use crate::image::corpus;

        let kernel_base = fs
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("pub(crate) const VOLUME_SECTOR: u64 = ")
            })
            .map(|value| value.trim_end_matches(';').to_string())
            .expect("no VOLUME_SECTOR in fs.rs");
        assert_eq!(kernel_base, super::NVME_VOLUME_SECTOR.to_string());

        for (constant, expected) in [
            ("SMALL_NAME", corpus::SMALL_NAME.to_string()),
            ("LARGE_NAME", corpus::LARGE_NAME.to_string()),
            ("DIRECTORY_NAME", corpus::DIRECTORY_NAME.to_string()),
            ("NESTED_NAME", corpus::NESTED_NAME.to_string()),
        ] {
            let declared = selftest
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix(&format!("const {constant}: &[u8] = b\""))
                })
                .and_then(|rest| rest.split_once('\"').map(|(value, _)| value.to_string()))
                .unwrap_or_else(|| panic!("no {constant} in selftest.rs"));
            assert_eq!(declared, expected, "{constant} drifted");
        }

        assert!(
            selftest.contains(&format!(
                "const LARGE_BYTES: usize = {} * 1024;",
                corpus::LARGE_BYTES / 1024
            )),
            "LARGE_BYTES drifted from the corpus"
        );
        assert!(
            selftest.contains("((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8"),
            "the kernel fills the large file differently from xtask"
        );
        // Compare the independent literal with the corpus implementation too.
        for index in [0usize, 1, 511, 4096, corpus::LARGE_BYTES - 1] {
            assert_eq!(
                corpus::large_byte(index),
                ((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8
            );
        }
    }

    fn cpp_raw_string_end(bytes: &[u8], start: usize) -> Option<usize> {
        if bytes.get(start..start + 2) != Some(b"R\"") {
            return None;
        }
        let identifier = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        let boundary = |at: usize| at == 0 || !identifier(bytes[at - 1]);
        let has_prefix = boundary(start)
            || (start >= 1
                && matches!(bytes[start - 1], b'u' | b'U' | b'L')
                && boundary(start - 1))
            || (start >= 2 && &bytes[start - 2..start] == b"u8" && boundary(start - 2));
        if !has_prefix {
            return None;
        }
        let delimiter_at = start + 2;
        let open = bytes[delimiter_at..]
            .iter()
            .position(|byte| *byte == b'(')
            .map(|offset| delimiter_at + offset)
            .expect("unterminated C++ raw-string delimiter");
        let delimiter = &bytes[delimiter_at..open];
        assert!(delimiter.len() <= 16, "oversized C++ raw-string delimiter");
        assert!(
            delimiter.iter().all(|byte| {
                !byte.is_ascii_whitespace() && !matches!(*byte, b'(' | b')' | b'\\')
            }),
            "invalid C++ raw-string delimiter"
        );
        let terminator_bytes = 2 + delimiter.len();
        let close = bytes[open + 1..]
            .windows(terminator_bytes)
            .position(|window| {
                window[0] == b')'
                    && &window[1..1 + delimiter.len()] == delimiter
                    && window[1 + delimiter.len()] == b'"'
            })
            .map(|offset| open + 1 + offset + terminator_bytes)
            .expect("unterminated C++ raw string literal");
        Some(close)
    }

    fn cpp_without_comments(source: &str) -> (String, Vec<String>) {
        for trigraph in [
            "??=", "??/", "??'", "??(", "??)", "??!", "??<", "??>", "??-",
        ] {
            assert!(
                !source.contains(trigraph),
                "C/C++ trigraph {trigraph} is not admitted"
            );
        }
        for (line_number, line) in source.lines().enumerate() {
            assert!(
                !line.trim_end().ends_with('\\'),
                "C/C++ line continuation on line {}",
                line_number + 1
            );
        }
        let bytes = source.as_bytes();
        let mut active = bytes.to_vec();
        let mut line_comments = Vec::new();
        let mut index = 0;

        while index < bytes.len() {
            if let Some(end) = cpp_raw_string_end(bytes, index) {
                index = end;
                continue;
            }
            match bytes[index] {
                quote @ (b'\'' | b'"') => {
                    index += 1;
                    let mut closed = false;
                    while index < bytes.len() {
                        if bytes[index] == b'\\' {
                            index = (index + 2).min(bytes.len());
                        } else if bytes[index] == quote {
                            index += 1;
                            closed = true;
                            break;
                        } else {
                            index += 1;
                        }
                    }
                    assert!(closed, "unterminated C++ literal");
                }
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    let end = bytes[index + 2..]
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .map_or(bytes.len(), |offset| index + 2 + offset);
                    line_comments.push(
                        String::from_utf8_lossy(&bytes[index + 2..end])
                            .trim()
                            .to_string(),
                    );
                    active[index..end].fill(b' ');
                    index = end;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    let start = index;
                    index += 2;
                    let mut closed = false;
                    while index + 1 < bytes.len() {
                        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
                            index += 2;
                            closed = true;
                            break;
                        }
                        index += 1;
                    }
                    assert!(closed, "unterminated C++ block comment");
                    for byte in &mut active[start..index] {
                        if *byte != b'\n' && *byte != b'\r' {
                            *byte = b' ';
                        }
                    }
                }
                _ => index += 1,
            }
        }

        (
            String::from_utf8(active).expect("comment masking kept UTF-8"),
            line_comments,
        )
    }

    fn cpp_structure(active: &str) -> String {
        let bytes = active.as_bytes();
        let mut structure = bytes.to_vec();
        let mut index = 0;
        while index < bytes.len() {
            if let Some(end) = cpp_raw_string_end(bytes, index) {
                for byte in &mut structure[index..end] {
                    if *byte != b'\n' && *byte != b'\r' {
                        *byte = b' ';
                    }
                }
                index = end;
                continue;
            }
            let quote = bytes[index];
            if quote != b'\'' && quote != b'"' {
                index += 1;
                continue;
            }
            let start = index;
            index += 1;
            let mut closed = false;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index = (index + 2).min(bytes.len());
                } else if bytes[index] == quote {
                    index += 1;
                    closed = true;
                    break;
                } else {
                    index += 1;
                }
            }
            assert!(closed, "unterminated C++ literal");
            for byte in &mut structure[start..index] {
                if *byte != b'\n' && *byte != b'\r' {
                    *byte = b' ';
                }
            }
        }
        String::from_utf8(structure).expect("literal masking kept UTF-8")
    }

    fn rust_without_comments(source: &str) -> String {
        let bytes = source.as_bytes();
        let mut active = bytes.to_vec();
        let mut index = 0usize;

        while index < bytes.len() {
            if bytes[index] == b'r' {
                let mut quote = index + 1;
                while bytes.get(quote) == Some(&b'#') {
                    quote += 1;
                }
                if bytes.get(quote) == Some(&b'"') {
                    let hashes = quote - index - 1;
                    let mut end = quote + 1;
                    let mut closed = None;
                    while end < bytes.len() {
                        if bytes[end] == b'"'
                            && bytes.get(end + 1..end + 1 + hashes)
                                == Some(&bytes[index + 1..quote])
                        {
                            closed = Some(end + 1 + hashes);
                            break;
                        }
                        end += 1;
                    }
                    let end = closed.expect("unterminated Rust raw string literal");
                    for byte in &mut active[index..end] {
                        if *byte != b'\n' && *byte != b'\r' {
                            *byte = b' ';
                        }
                    }
                    index = end;
                    continue;
                }
            }
            match bytes[index] {
                b'"' => {
                    let start = index;
                    index += 1;
                    let mut closed = false;
                    while index < bytes.len() {
                        if bytes[index] == b'\\' {
                            index = (index + 2).min(bytes.len());
                        } else if bytes[index] == b'"' {
                            index += 1;
                            closed = true;
                            break;
                        } else {
                            index += 1;
                        }
                    }
                    assert!(closed, "unterminated Rust string literal");
                    for byte in &mut active[start..index] {
                        if *byte != b'\n' && *byte != b'\r' {
                            *byte = b' ';
                        }
                    }
                }
                b'\'' => {
                    let start = index;
                    let mut cursor = index + 1;
                    let mut escaped = false;
                    let mut close = None;
                    while cursor < bytes.len() && bytes[cursor] != b'\n' {
                        if !escaped && bytes[cursor] == b'\'' {
                            close = Some(cursor + 1);
                            break;
                        }
                        escaped = !escaped && bytes[cursor] == b'\\';
                        cursor += 1;
                    }
                    let is_character = close.is_some_and(|end| {
                        let contents = &source[start + 1..end - 1];
                        contents.starts_with('\\') || contents.chars().count() == 1
                    });
                    if is_character {
                        index = close.expect("a character literal has a close");
                        active[start..index].fill(b' ');
                    } else {
                        index += 1;
                    }
                }
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    let end = bytes[index + 2..]
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .map_or(bytes.len(), |offset| index + 2 + offset);
                    active[index..end].fill(b' ');
                    index = end;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    let start = index;
                    let mut depth = 1usize;
                    index += 2;
                    while index < bytes.len() && depth != 0 {
                        if bytes.get(index + 1) == Some(&b'*') && bytes[index] == b'/' {
                            depth += 1;
                            index += 2;
                        } else if bytes.get(index + 1) == Some(&b'/') && bytes[index] == b'*' {
                            depth -= 1;
                            index += 2;
                        } else {
                            index += 1;
                        }
                    }
                    assert_eq!(depth, 0, "unterminated Rust block comment");
                    for byte in &mut active[start..index] {
                        if *byte != b'\n' && *byte != b'\r' {
                            *byte = b' ';
                        }
                    }
                }
                _ => index += 1,
            }
        }

        String::from_utf8(active).expect("Rust comment masking kept UTF-8")
    }

    fn cpp_directive(line: &str) -> Option<(&str, &str)> {
        let directive = line.trim_start().strip_prefix('#')?.trim_start();
        let split = directive
            .bytes()
            .position(|byte| byte.is_ascii_whitespace())
            .unwrap_or(directive.len());
        Some((&directive[..split], directive[split..].trim()))
    }

    fn checked_header(source: &str) -> (String, std::collections::BTreeMap<String, String>) {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Phase {
            BeforeGuard,
            GuardDefinition,
            Includes,
            LinkageOpen,
            Body,
            LinkageClose,
            GuardClose,
            Done,
        }

        let (active, _) = cpp_without_comments(source);
        let structure = cpp_structure(&active);
        let mut macros = std::collections::BTreeMap::new();
        let mut includes = Vec::new();
        let mut linkage_body = String::new();
        let mut phase = Phase::BeforeGuard;

        for (line_number, (line, structural_line)) in
            active.lines().zip(structure.lines()).enumerate()
        {
            let structural_line = structural_line.trim();
            assert!(
                !structural_line.starts_with("%:") && !structural_line.starts_with("??="),
                "alternate preprocessor directive on line {}",
                line_number + 1
            );
            assert!(
                !structural_line
                    .split(|character: char| {
                        !character.is_ascii_alphanumeric() && character != '_'
                    })
                    .any(|word| matches!(word, "_Pragma" | "__pragma")),
                "pragma operator on line {}",
                line_number + 1
            );
            if cpp_directive(structural_line).is_some() {
                let (kind, rest) = cpp_directive(line).expect("structural directive has source");
                assert!(
                    !line.trim_end().ends_with('\\'),
                    "continued header directive on line {}",
                    line_number + 1
                );
                match kind {
                    "ifndef" => {
                        assert_eq!(phase, Phase::BeforeGuard, "misplaced include guard");
                        assert_eq!(rest, "WIZARD_ABI_H", "unexpected header guard");
                        phase = Phase::GuardDefinition;
                    }
                    "ifdef" => {
                        assert_eq!(rest, "__cplusplus", "unexpected header conditional");
                        phase = match phase {
                            Phase::Includes => {
                                assert_eq!(
                                    includes,
                                    ["<stddef.h>", "<stdint.h>"],
                                    "the fixed header includes changed"
                                );
                                linkage_body.clear();
                                Phase::LinkageOpen
                            }
                            Phase::Body => {
                                linkage_body.clear();
                                Phase::LinkageClose
                            }
                            _ => panic!("misplaced __cplusplus guard"),
                        };
                    }
                    "endif" => {
                        assert!(rest.is_empty(), "unexpected tokens after #endif");
                        phase = match phase {
                            Phase::LinkageOpen => {
                                assert_eq!(
                                    linkage_body, "extern\"C\"{",
                                    "the first __cplusplus guard is not the linkage opener"
                                );
                                Phase::Body
                            }
                            Phase::LinkageClose => {
                                assert_eq!(
                                    linkage_body, "}",
                                    "the second __cplusplus guard is not the linkage closer"
                                );
                                Phase::GuardClose
                            }
                            Phase::GuardClose => Phase::Done,
                            _ => panic!("#endif without the expected opener"),
                        };
                    }
                    "include" => {
                        assert_eq!(phase, Phase::Includes, "misplaced header include");
                        assert!(
                            matches!(rest, "<stddef.h>" | "<stdint.h>"),
                            "unexpected header include `{rest}`"
                        );
                        assert!(!includes.contains(&rest), "duplicate include `{rest}`");
                        includes.push(rest);
                    }
                    "define" => {
                        let split = rest
                            .bytes()
                            .position(|byte| byte.is_ascii_whitespace())
                            .unwrap_or(rest.len());
                        let name = &rest[..split];
                        let value = rest[split..].trim();
                        if name == "WIZARD_ABI_H" {
                            assert_eq!(
                                phase,
                                Phase::GuardDefinition,
                                "misplaced include guard definition"
                            );
                            assert!(value.is_empty(), "the include guard has a value");
                            phase = Phase::Includes;
                            continue;
                        }
                        assert_eq!(phase, Phase::Body, "macro outside the linkage body");
                        assert!(name.starts_with("WIZ_"), "unexpected macro `{name}`");
                        assert!(
                            name.bytes().all(|byte| byte.is_ascii_uppercase()
                                || byte.is_ascii_digit()
                                || byte == b'_'),
                            "non-object macro `{name}`"
                        );
                        assert!(!value.is_empty(), "macro `{name}` has no value");
                        assert!(
                            macros.insert(name.to_string(), value.to_string()).is_none(),
                            "duplicate macro `{name}`"
                        );
                    }
                    other => panic!("unexpected header directive #{other}"),
                }
                continue;
            }

            let code = compact_cpp(line);
            match phase {
                Phase::LinkageOpen | Phase::LinkageClose => linkage_body.push_str(&code),
                Phase::Body => {}
                _ => assert!(
                    code.is_empty(),
                    "code outside the header's declared linkage body"
                ),
            }
        }

        assert_eq!(phase, Phase::Done, "incomplete header conditional scaffold");
        (active, macros)
    }

    fn matching(source: &[u8], open: usize, left: u8, right: u8) -> usize {
        let mut depth = 0usize;
        for (offset, byte) in source[open..].iter().enumerate() {
            if *byte == left {
                depth += 1;
            } else if *byte == right {
                depth = depth.checked_sub(1).expect("closing delimiter before open");
                if depth == 0 {
                    return open + offset;
                }
            }
        }
        panic!("unterminated delimiter at byte {open}");
    }

    fn cpp_bodies(active: &str) -> std::collections::BTreeMap<String, (&str, &str)> {
        let structure = cpp_structure(active);
        let bytes = structure.as_bytes();
        let outer_marker = "namespace mlibc";
        let outer_markers: Vec<_> = structure.match_indices(outer_marker).collect();
        assert_eq!(outer_markers.len(), 1, "expected one namespace mlibc");
        assert!(
            outer_markers[0].0 == 0
                || !bytes[outer_markers[0].0 - 1].is_ascii_alphanumeric()
                    && bytes[outer_markers[0].0 - 1] != b'_',
            "namespace mlibc is embedded in an identifier"
        );
        let mut outer_open = outer_markers[0].0 + outer_marker.len();
        while bytes.get(outer_open).is_some_and(u8::is_ascii_whitespace) {
            outer_open += 1;
        }
        assert_eq!(
            bytes.get(outer_open),
            Some(&b'{'),
            "namespace mlibc has unexpected qualifiers"
        );
        let outer_close = matching(bytes, outer_open, b'{', b'}');
        assert!(
            structure[outer_close + 1..].trim().is_empty(),
            "active C++ follows namespace mlibc"
        );
        let inner_marker = "namespace {";
        let inner_markers: Vec<_> = structure[outer_open + 1..outer_close]
            .match_indices(inner_marker)
            .map(|(at, marker)| (outer_open + 1 + at, marker))
            .filter(|(at, _)| {
                *at == 0 || !bytes[*at - 1].is_ascii_alphanumeric() && bytes[*at - 1] != b'_'
            })
            .collect();
        assert_eq!(inner_markers.len(), 1, "expected one anonymous namespace");
        let inner_open = inner_markers[0].0 + inner_marker.len() - 1;
        let inner_close = matching(bytes, inner_open, b'{', b'}');
        assert!(
            inner_close < outer_close,
            "anonymous namespace escapes mlibc"
        );
        let mut bodies = std::collections::BTreeMap::new();
        let mut index = 0usize;
        let mut depth = 0usize;

        while index < bytes.len() {
            if bytes[index] == b'{' {
                depth += 1;
                index += 1;
                continue;
            }
            if bytes[index] == b'}' {
                depth = depth.checked_sub(1).expect("unmatched C++ closing brace");
                index += 1;
                continue;
            }
            if !bytes[index].is_ascii_alphabetic() && bytes[index] != b'_' {
                index += 1;
                continue;
            }
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let name = &structure[start..index];
            if !name.starts_with("sys_")
                && !matches!(
                    name,
                    "call1" | "as_errno" | "read_counter" | "monotonic_fallback"
                )
            {
                continue;
            }
            let mut cursor = index;
            while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b'(') {
                continue;
            }
            let close_parameters = matching(bytes, cursor, b'(', b')');
            cursor = close_parameters + 1;
            while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b'{') {
                continue;
            }
            let (expected_depth, in_expected_namespace) = if name.starts_with("sys_") {
                (
                    1,
                    start > outer_open
                        && start < outer_close
                        && !(start > inner_open && start < inner_close),
                )
            } else {
                (2, start > inner_open && start < inner_close)
            };
            assert_eq!(
                depth, expected_depth,
                "{name} is not defined at its expected mlibc namespace depth"
            );
            assert!(
                in_expected_namespace,
                "{name} is outside its expected mlibc namespace"
            );
            let close_body = matching(bytes, cursor, b'{', b'}');
            let declaration_start = structure[..start]
                .rfind('\n')
                .map_or(0, |newline| newline + 1);
            assert!(
                bodies
                    .insert(
                        name.to_string(),
                        (
                            active[declaration_start..cursor].trim(),
                            &active[cursor + 1..close_body],
                        ),
                    )
                    .is_none(),
                "two active definitions of {name}"
            );
            index = close_body + 1;
        }
        assert_eq!(depth, 0, "unbalanced C++ braces");
        bodies
    }

    fn sysdep_annotations(
        line_comments: &[String],
    ) -> std::collections::BTreeMap<String, (String, String)> {
        let mut annotations = std::collections::BTreeMap::new();
        for comment in line_comments {
            let Some(rest) = comment.strip_prefix("sys_") else {
                continue;
            };
            let Some((name, states)) = rest.split_once(". BODY=") else {
                continue;
            };
            let (body, remainder) = states
                .split_once("; REMAINDER=")
                .unwrap_or_else(|| panic!("sys_{name} has no REMAINDER state"));
            let remainder = remainder
                .strip_suffix('.')
                .unwrap_or_else(|| panic!("sys_{name}'s state marker has no full stop"));
            assert!(
                matches!(body, "WIRED" | "COMPOSED" | "PARTIAL" | "STUB"),
                "sys_{name} has unknown BODY={body}"
            );
            assert!(
                matches!(
                    remainder,
                    "WORKS" | "COMPAT" | "DOOR" | "PARTIAL" | "MISSING"
                ),
                "sys_{name} has unknown REMAINDER={remainder}"
            );
            assert!(
                annotations
                    .insert(
                        format!("sys_{name}"),
                        (body.to_string(), remainder.to_string())
                    )
                    .is_none(),
                "sys_{name} has two state markers"
            );
        }
        annotations
    }

    fn compact_cpp(source: &str) -> String {
        source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect()
    }

    fn assert_clock_arithmetic(source: &str) {
        let compact = compact_cpp(&cpp_structure(source));
        let start = compact
            .find("uint64_tticks=")
            .expect("clock counter-delta calculation");
        let end = compact[start..]
            .find("*secs=")
            .map(|offset| start + offset)
            .expect("clock result stores");
        assert_eq!(
            &compact[start..end],
            concat!(
                "uint64_tticks=stamp-last;",
                "unsigned__int128advanced=static_cast<unsigned__int128>(fraction)",
                "+static_cast<unsigned__int128>(ticks)*multiplier;",
                "unsigned__int128whole=advanced>>shift;",
                "uint64_tns;",
                "if(whole>UINT64_MAX){ns=UINT64_MAX;}",
                "else{uint64_tdelta=static_cast<uint64_t>(whole);",
                "ns=base>UINT64_MAX-delta?UINT64_MAX:base+delta;}"
            ),
            "the clock arithmetic no longer widens before multiplying and saturates afterwards"
        );
    }

    struct CIntegerParser<'a> {
        source: &'a [u8],
        at: usize,
    }

    impl CIntegerParser<'_> {
        fn skip_space(&mut self) {
            while self
                .source
                .get(self.at)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.at += 1;
            }
        }

        fn take(&mut self, token: &[u8]) -> bool {
            self.skip_space();
            if self.source.get(self.at..self.at + token.len()) == Some(token) {
                self.at += token.len();
                true
            } else {
                false
            }
        }

        fn identifier(&mut self) -> Option<String> {
            self.skip_space();
            let start = self.at;
            if !self
                .source
                .get(self.at)
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            {
                return None;
            }
            self.at += 1;
            while self
                .source
                .get(self.at)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                self.at += 1;
            }
            Some(
                std::str::from_utf8(&self.source[start..self.at])
                    .expect("ASCII identifier")
                    .to_string(),
            )
        }

        fn number(&mut self) -> Result<u64, String> {
            self.skip_space();
            let start = self.at;
            let (radix, digits_at) = if self.source.get(self.at..self.at + 2) == Some(b"0x")
                || self.source.get(self.at..self.at + 2) == Some(b"0X")
            {
                self.at += 2;
                (16, self.at)
            } else {
                (10, self.at)
            };
            while self.source.get(self.at).is_some_and(|byte| match radix {
                16 => byte.is_ascii_hexdigit(),
                _ => byte.is_ascii_digit(),
            }) {
                self.at += 1;
            }
            if self.at == digits_at {
                return Err(format!("expected an integer literal at byte {start}"));
            }
            let digits = std::str::from_utf8(&self.source[digits_at..self.at])
                .expect("ASCII integer literal");
            if radix == 10 && digits.len() > 1 && digits.starts_with('0') {
                return Err(format!(
                    "octal-looking integer literal `{digits}` is not admitted"
                ));
            }
            let suffix_at = self.at;
            while self
                .source
                .get(self.at)
                .is_some_and(u8::is_ascii_alphabetic)
            {
                self.at += 1;
            }
            let suffix = std::str::from_utf8(&self.source[suffix_at..self.at])
                .expect("ASCII integer suffix")
                .to_ascii_lowercase();
            if !matches!(
                suffix.as_str(),
                "" | "u" | "l" | "ul" | "lu" | "ll" | "ull" | "llu"
            ) {
                return Err(format!("unsupported integer suffix `{suffix}`"));
            }
            u64::from_str_radix(digits, radix)
                .map_err(|_| format!("integer literal `{digits}` does not fit in u64"))
        }

        fn primary(&mut self) -> Result<u64, String> {
            self.skip_space();
            if !self.take(b"(") {
                return self.number();
            }

            let inside = self.at;
            if let Some(kind) = self.identifier() {
                self.skip_space();
                if self.take(b")") {
                    if !matches!(
                        kind.as_str(),
                        "wiz_handle_t" | "wiz_rights_t" | "uintptr_t" | "uint64_t"
                    ) {
                        return Err(format!("unsupported C cast `{kind}`"));
                    }
                    return self.primary();
                }
            }

            self.at = inside;
            let value = self.shift()?;
            if !self.take(b")") {
                return Err(format!(
                    "unterminated parenthesized integer at byte {inside}"
                ));
            }
            Ok(value)
        }

        fn shift(&mut self) -> Result<u64, String> {
            let mut value = self.primary()?;
            while self.take(b"<<") {
                let shift = self.primary()?;
                if shift >= u64::from(u32::BITS) || value > (u64::from(u32::MAX) >> shift) {
                    return Err("overflowing integer shift".to_string());
                }
                value <<= shift;
            }
            Ok(value)
        }
    }

    fn parse_c_uint(expression: &str) -> Result<u64, String> {
        let mut parser = CIntegerParser {
            source: expression.as_bytes(),
            at: 0,
        };
        let value = parser.shift()?;
        parser.skip_space();
        if parser.at != parser.source.len() {
            return Err(format!(
                "unparsed C integer expression `{}`",
                &expression[parser.at..]
            ));
        }
        Ok(value)
    }

    fn parse_rust_uint(literal: &str) -> Result<u64, String> {
        let literal = literal.trim();
        let (radix, prefix) = if literal.starts_with("0x") {
            (16, 2)
        } else if literal.starts_with("0b") {
            (2, 2)
        } else if literal.starts_with("0o") {
            (8, 2)
        } else {
            (10, 0)
        };
        let mut end = prefix;
        for (offset, character) in literal[prefix..].char_indices() {
            let digit = character == '_'
                || match radix {
                    16 => character.is_ascii_hexdigit(),
                    10 => character.is_ascii_digit(),
                    8 => matches!(character, '0'..='7'),
                    2 => matches!(character, '0' | '1'),
                    _ => unreachable!(),
                };
            if !digit {
                break;
            }
            end = prefix + offset + character.len_utf8();
        }
        if end == prefix {
            return Err(format!("`{literal}` is not an unsigned Rust literal"));
        }
        let suffix = &literal[end..];
        if !matches!(
            suffix,
            "" | "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize"
        ) {
            return Err(format!("unsupported Rust integer suffix `{suffix}`"));
        }
        let digits: String = literal[prefix..end]
            .chars()
            .filter(|character| *character != '_')
            .collect();
        u64::from_str_radix(&digits, radix)
            .map_err(|_| format!("Rust integer literal `{literal}` does not fit in u64"))
    }

    fn rust_delimiter_depth(source: &str, end: usize) -> [usize; 3] {
        let mut depth = [0usize; 3];
        for byte in &source.as_bytes()[..end] {
            match byte {
                b'{' => depth[0] += 1,
                b'}' => {
                    depth[0] = depth[0]
                        .checked_sub(1)
                        .expect("unmatched Rust closing brace")
                }
                b'(' => depth[1] += 1,
                b')' => {
                    depth[1] = depth[1]
                        .checked_sub(1)
                        .expect("unmatched Rust closing parenthesis")
                }
                b'[' => depth[2] += 1,
                b']' => {
                    depth[2] = depth[2]
                        .checked_sub(1)
                        .expect("unmatched Rust closing bracket")
                }
                _ => {}
            }
        }
        depth
    }

    fn immediately_preceding_outer_rust_attribute(source: &str, line_start: usize) -> bool {
        let prefix = source[..line_start].trim_end();
        let bytes = prefix.as_bytes();
        if bytes.last() != Some(&b']') {
            return false;
        }
        let mut depth = 0usize;
        for at in (0..bytes.len()).rev() {
            match bytes[at] {
                b']' => depth += 1,
                b'[' => {
                    depth = depth
                        .checked_sub(1)
                        .expect("opening Rust bracket before its close");
                    if depth == 0 {
                        return bytes.get(at.wrapping_sub(1)) == Some(&b'#');
                    }
                }
                _ => {}
            }
        }
        panic!("unterminated Rust attribute before an ABI witness");
    }

    fn assert_unconditional_rust_line(source: &str, at: usize, depth: usize, label: &str) {
        let line_start = source[..at].rfind('\n').map_or(0, |newline| newline + 1);
        assert!(
            source[line_start..at].trim().is_empty(),
            "{label} is not a direct Rust item"
        );
        let nesting = rust_delimiter_depth(source, at);
        assert_eq!(nesting[0], depth, "{label} is at the wrong brace depth");
        assert_eq!(
            [nesting[1], nesting[2]],
            [0, 0],
            "{label} is nested in a Rust macro or expression"
        );
        assert!(
            !immediately_preceding_outer_rust_attribute(source, line_start),
            "{label} is controlled by a Rust attribute"
        );
    }

    fn rust_expression(source: &str, declaration: &str) -> String {
        let source = rust_without_comments(source);
        let prefix = format!("{declaration} = ");
        let matches: Vec<_> = source.match_indices(&prefix).collect();
        assert_eq!(matches.len(), 1, "expected one `{declaration}` declaration");
        let at = matches[0].0;
        assert_unconditional_rust_line(&source, at, 0, declaration);
        let value_start = at + prefix.len();
        let line_end = source[value_start..]
            .find('\n')
            .map_or(source.len(), |offset| value_start + offset);
        source[value_start..line_end]
            .trim()
            .strip_suffix(';')
            .unwrap_or_else(|| panic!("unterminated `{declaration}` declaration"))
            .trim()
            .to_string()
    }

    fn rust_function_body(source: &str, signature: &str) -> String {
        let source = rust_without_comments(source);
        let matches: Vec<_> = source.match_indices(signature).collect();
        assert_eq!(matches.len(), 1, "expected one `{signature}`");
        let at = matches[0].0;
        assert_unconditional_rust_line(&source, at, 0, signature);
        let open = at + signature.len() - 1;
        assert_eq!(source.as_bytes().get(open), Some(&b'{'));
        let close = matching(source.as_bytes(), open, b'{', b'}');
        source[open + 1..close].to_string()
    }

    fn rust_uint(source: &str, declaration: &str) -> u64 {
        let expression = rust_expression(source, declaration);
        parse_rust_uint(&expression)
            .unwrap_or_else(|error| panic!("unparsed `{declaration}`: {error}"))
    }

    fn rust_initializer(source: &str, declaration: &str) -> String {
        let source = rust_without_comments(source);
        let prefix = format!("{declaration} = ");
        let matches: Vec<_> = source.match_indices(&prefix).collect();
        assert_eq!(matches.len(), 1, "expected one `{declaration}` declaration");
        let at = matches[0].0;
        assert_unconditional_rust_line(&source, at, 0, declaration);
        let value_start = at + prefix.len();
        let bytes = source.as_bytes();
        let mut depth = [0usize; 3];
        let mut value_end = None;
        for (offset, byte) in bytes.iter().copied().enumerate().skip(value_start) {
            match byte {
                b'{' => depth[0] += 1,
                b'}' => {
                    depth[0] = depth[0]
                        .checked_sub(1)
                        .unwrap_or_else(|| panic!("unmatched brace in `{declaration}`"));
                }
                b'(' => depth[1] += 1,
                b')' => {
                    depth[1] = depth[1]
                        .checked_sub(1)
                        .unwrap_or_else(|| panic!("unmatched parenthesis in `{declaration}`"));
                }
                b'[' => depth[2] += 1,
                b']' => {
                    depth[2] = depth[2]
                        .checked_sub(1)
                        .unwrap_or_else(|| panic!("unmatched bracket in `{declaration}`"));
                }
                b';' if depth == [0, 0, 0] => {
                    value_end = Some(offset);
                    break;
                }
                _ => {}
            }
        }
        let value_end = value_end.unwrap_or_else(|| panic!("unterminated `{declaration}`"));
        source[value_start..value_end].trim().to_string()
    }

    fn rust_interrupt_vector(source: &str, name: &str) -> u64 {
        const ASSIGNMENT_PREFIX: &str = "const SOURCES: [InterruptVector; ";

        let source = rust_without_comments(source);
        let assignments: Vec<_> = source.match_indices(ASSIGNMENT_PREFIX).collect();
        assert_eq!(assignments.len(), 1, "expected one typed vector assignment");
        let assignment_at = assignments[0].0;
        assert_unconditional_rust_line(&source, assignment_at, 0, "typed vector assignment");
        let length_start = assignment_at + ASSIGNMENT_PREFIX.len();
        let length_end = source[length_start..]
            .find(']')
            .map(|offset| length_start + offset)
            .expect("unterminated typed vector array type");
        let length_literal = &source[length_start..length_end];
        let declared_length = usize::try_from(
            parse_rust_uint(length_literal)
                .unwrap_or_else(|error| panic!("unparsed vector count: {error}")),
        )
        .expect("typed vector count does not fit in usize");
        let declaration = format!("{ASSIGNMENT_PREFIX}{length_literal}]");
        let initializer = rust_initializer(&source, &declaration);
        let compact: String = initializer
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        let (literals, success_and_failure) = compact
            .strip_prefix("matchAssignment::new([")
            .and_then(|rest| rest.split_once("]){"))
            .expect("typed vectors do not come directly from Assignment::new");
        assert_eq!(
            success_and_failure, "Some(assignment)=>assignment.into_array(),None=>panic!(),}",
            "typed vector assignment does not return the admitted array"
        );
        let mut vectors = Vec::new();
        for literal in literals.split(',') {
            let value = parse_rust_uint(literal)
                .unwrap_or_else(|error| panic!("unparsed interrupt vector: {error}"));
            let value = u8::try_from(value).expect("interrupt vector does not fit in u8");
            vectors.push(u64::from(value));
        }
        assert_eq!(
            vectors.len(),
            declared_length,
            "typed vector count differs from its array type"
        );

        let declaration = format!("pub(crate) const {name}: InterruptVector");
        let expression = rust_expression(&source, &declaration);
        let compact: String = expression
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        let index = compact
            .strip_prefix("SOURCES[")
            .and_then(|value| value.strip_suffix(']'))
            .and_then(|value| parse_rust_uint(value).ok())
            .and_then(|value| usize::try_from(value).ok())
            .expect("typed vector is not a direct SOURCES index");
        *vectors
            .get(index)
            .unwrap_or_else(|| panic!("typed vector index {index} is out of bounds"))
    }

    fn rust_negated_errno(source: &str, name: &str) -> u64 {
        let expression = rust_expression(source, &format!("const {name}: u64"));
        let compact: String = expression
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        let magnitude = compact
            .strip_prefix("(-")
            .and_then(|value| value.strip_suffix("i64)asu64"))
            .unwrap_or_else(|| panic!("{name} is not a negated i64 errno"));
        parse_rust_uint(magnitude).unwrap_or_else(|error| panic!("unparsed {name}: {error}"))
    }

    fn rust_status_constants(source: &str) -> std::collections::BTreeMap<String, u64> {
        let source = rust_without_comments(source);
        let marker = "STATUS_";
        let canonical = "pub(crate) const STATUS_";
        let mut statuses = std::collections::BTreeMap::new();
        let mut numbers = std::collections::BTreeSet::new();
        for (at, _) in source.match_indices(marker) {
            if rust_delimiter_depth(&source, at) != [0, 0, 0] {
                continue;
            }
            let line_start = source[..at].rfind('\n').map_or(0, |newline| newline + 1);
            assert_unconditional_rust_line(&source, line_start, 0, "kernel status constant");
            let line_end = source[at..]
                .find('\n')
                .map_or(source.len(), |offset| at + offset);
            let line = source[line_start..line_end].trim();
            let rest = line
                .strip_prefix(canonical)
                .unwrap_or_else(|| panic!("non-canonical file-scope status `{line}`"));
            let (name, expression) = rest
                .split_once(": u32 = ")
                .unwrap_or_else(|| panic!("malformed kernel status `{line}`"));
            assert!(
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte == b'_'),
                "malformed kernel status name `{name}`"
            );
            let expression = expression
                .strip_suffix(';')
                .unwrap_or_else(|| panic!("unterminated kernel status {name}"));
            let number = parse_rust_uint(expression)
                .unwrap_or_else(|error| panic!("unparsed kernel status {name}: {error}"));
            assert!(numbers.insert(number), "two statuses use {number}");
            assert!(
                statuses.insert(name.to_string(), number).is_none(),
                "two kernel status constants are named {name}"
            );
        }
        assert!(!statuses.is_empty(), "the kernel defines no statuses");
        statuses
    }

    fn simple_decoder_arms(
        source: &str,
        owner: &str,
        signature: &str,
        unknown: &str,
    ) -> std::collections::BTreeMap<u64, String> {
        let source = rust_without_comments(source);
        let impl_marker = format!("impl {owner} {{");
        let implementations: Vec<_> = source.match_indices(&impl_marker).collect();
        assert_eq!(
            implementations.len(),
            1,
            "expected one direct `impl {owner}`"
        );
        let impl_at = implementations[0].0;
        assert_unconditional_rust_line(&source, impl_at, 0, &format!("impl {owner}"));
        let impl_open = impl_at + impl_marker.len() - 1;
        let impl_close = matching(source.as_bytes(), impl_open, b'{', b'}');
        let signatures: Vec<_> = source.match_indices(signature).collect();
        assert_eq!(signatures.len(), 1, "expected one `{signature}`");
        let signature_at = signatures[0].0;
        assert!(
            signature_at > impl_open && signature_at < impl_close,
            "`{signature}` is outside impl {owner}"
        );
        assert_unconditional_rust_line(&source, signature_at, 1, signature);
        assert!(
            source[impl_open + 1..signature_at].trim().is_empty(),
            "impl {owner} has an item before `{signature}`"
        );
        let open = signature_at + signature.len() - 1;
        assert_eq!(source.as_bytes().get(open), Some(&b'{'));
        let close = matching(source.as_bytes(), open, b'{', b'}');
        assert!(
            source[close + 1..impl_close].trim().is_empty(),
            "impl {owner} has an item after `{signature}`"
        );
        let body: String = source[open + 1..close]
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        let arms = body
            .strip_prefix("matchvalue{")
            .and_then(|body| body.strip_suffix('}'))
            .unwrap_or_else(|| panic!("`{signature}` is not only a value match"));
        let mut decoded = std::collections::BTreeMap::new();
        let mut variants = std::collections::BTreeSet::new();
        let mut saw_unknown = false;
        let members: Vec<_> = arms
            .split(',')
            .filter(|member| !member.is_empty())
            .collect();
        for (index, member) in members.iter().enumerate() {
            if *member == unknown {
                assert!(!saw_unknown, "duplicate unknown decoder arm");
                assert_eq!(index + 1, members.len(), "unknown decoder arm is not last");
                saw_unknown = true;
                continue;
            }
            assert!(!saw_unknown, "decoder arm follows the unknown arm");
            let (number, target) = member
                .split_once("=>Self::")
                .unwrap_or_else(|| panic!("non-canonical decoder arm `{member}`"));
            assert!(
                !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()),
                "decoder pattern `{number}` is not one decimal literal"
            );
            let number = <u64 as core::str::FromStr>::from_str(number)
                .unwrap_or_else(|_| panic!("decoder number `{number}` does not fit in u64"));
            assert!(
                !target.is_empty()
                    && target
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    && target
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphabetic),
                "decoder target `{target}` is not a plain variant"
            );
            assert!(
                variants.insert(target),
                "duplicate decoder target `{target}`"
            );
            assert!(
                decoded.insert(number, target.to_string()).is_none(),
                "duplicate decoder number {number}"
            );
        }
        assert!(saw_unknown, "decoder has no canonical unknown arm");
        assert!(!decoded.is_empty(), "decoder has no known arms");
        decoded
    }

    fn c_enum_values(source: &str, name: &str) -> std::collections::BTreeMap<String, u64> {
        let source = cpp_structure(source);
        let marker = format!("enum {name}");
        let markers: Vec<_> = source.match_indices(&marker).collect();
        assert_eq!(markers.len(), 1, "expected one C enum {name}");
        let start = markers[0].0;
        assert!(
            start == 0
                || !source.as_bytes()[start - 1].is_ascii_alphanumeric()
                    && source.as_bytes()[start - 1] != b'_',
            "C enum {name} is embedded in an identifier"
        );
        let mut open = start + marker.len();
        while source
            .as_bytes()
            .get(open)
            .is_some_and(u8::is_ascii_whitespace)
        {
            open += 1;
        }
        assert_eq!(
            source.as_bytes().get(open),
            Some(&b'{'),
            "unexpected tokens before C enum {name}'s body"
        );
        let close = matching(source.as_bytes(), open, b'{', b'}');
        let mut after = close + 1;
        while source
            .as_bytes()
            .get(after)
            .is_some_and(u8::is_ascii_whitespace)
        {
            after += 1;
        }
        assert_eq!(
            source.as_bytes().get(after),
            Some(&b';'),
            "C enum {name} is not terminated"
        );
        let mut values = std::collections::BTreeMap::new();
        let mut numbers = std::collections::BTreeSet::new();
        for member in source[open + 1..close]
            .split(',')
            .map(str::trim)
            .filter(|member| !member.is_empty())
        {
            let (member_name, expression) = member
                .split_once('=')
                .unwrap_or_else(|| panic!("implicit or malformed C enum member `{member}`"));
            let member_name = member_name.trim();
            assert!(
                !member_name.is_empty()
                    && member_name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    && member_name
                        .as_bytes()
                        .first()
                        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_'),
                "malformed C enum member `{member_name}`"
            );
            let number = parse_c_uint(expression)
                .unwrap_or_else(|error| panic!("unparsed C enum member `{member}`: {error}"));
            assert!(numbers.insert(number), "aliased C enum value {number}");
            assert!(
                values.insert(member_name.to_string(), number).is_none(),
                "duplicate C enum member `{member_name}`"
            );
        }
        assert!(!values.is_empty(), "C enum {name} has no members");
        values
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ProgramOperands {
        pops: Vec<&'static str>,
        rcx_imm8_rejections: Vec<(u8, usize)>,
        ring_bases: Vec<u64>,
        stack_immediates: Vec<u64>,
        eax_immediates: Vec<u32>,
        interrupts: Vec<u8>,
        writes32: Vec<(u32, u32)>,
        writes32_from_esi: Vec<u32>,
        writes64_from_imm32: Vec<(u32, u32)>,
        short_writes32: Vec<(u8, u32)>,
        short_reads32: Vec<u8>,
        reads32: Vec<u32>,
        reads64: Vec<u32>,
        eax_imm8_comparisons: Vec<u8>,
        r8d_immediates: Vec<u32>,
        writes32_from_eax: Vec<u32>,
        fenced_timestamp_reads: usize,
        r8d_zeroes: usize,
        r8d_zero_tests: usize,
        r10d_timestamp_captures: usize,
        timestamp_subtractions: usize,
        ecx_zero_tests: usize,
        rcx_zero_tests: usize,
        ecx_esi_comparisons: usize,
    }

    fn sign_extended_imm8(value: u64) -> Option<u8> {
        i8::try_from(value)
            .ok()
            .map(|immediate| immediate.to_le_bytes()[0])
    }

    fn decode_program_operands(code: &[u8]) -> ProgramOperands {
        let mut operands = ProgramOperands {
            pops: Vec::new(),
            rcx_imm8_rejections: Vec::new(),
            ring_bases: Vec::new(),
            stack_immediates: Vec::new(),
            eax_immediates: Vec::new(),
            interrupts: Vec::new(),
            writes32: Vec::new(),
            writes32_from_esi: Vec::new(),
            writes64_from_imm32: Vec::new(),
            short_writes32: Vec::new(),
            short_reads32: Vec::new(),
            reads32: Vec::new(),
            reads64: Vec::new(),
            eax_imm8_comparisons: Vec::new(),
            r8d_immediates: Vec::new(),
            writes32_from_eax: Vec::new(),
            fenced_timestamp_reads: 0,
            r8d_zeroes: 0,
            r8d_zero_tests: 0,
            r10d_timestamp_captures: 0,
            timestamp_subtractions: 0,
            ecx_zero_tests: 0,
            rcx_zero_tests: 0,
            ecx_esi_comparisons: 0,
        };
        let mut at = 0usize;
        while at < code.len() {
            let rest = &code[at..];
            let immediate32 = |offset: usize| {
                u32::from_le_bytes(
                    rest.get(offset..offset + 4)
                        .unwrap_or_else(|| panic!("truncated imm32 at byte {at}"))
                        .try_into()
                        .expect("four immediate bytes"),
                )
            };
            let bytes = if rest.starts_with(&[0x59]) {
                operands.pops.push("rcx");
                1
            } else if rest.starts_with(&[0x58]) {
                operands.pops.push("rax");
                1
            } else if rest.starts_with(&[0x5E]) {
                operands.pops.push("rsi");
                1
            } else if rest.starts_with(&[0x48, 0x83, 0xF9]) {
                let immediate = *rest
                    .get(3)
                    .unwrap_or_else(|| panic!("truncated cmp rcx, imm8 at byte {at}"));
                let rejection = rest
                    .get(4..6)
                    .unwrap_or_else(|| panic!("cmp rcx, imm8 at byte {at} has no rejection"));
                assert_eq!(
                    rejection[0], 0x75,
                    "cmp rcx, imm8 at byte {at} is not followed by jne"
                );
                let target = i64::try_from(at).expect("program offset fits in i64")
                    + 6
                    + i64::from(rejection[1] as i8);
                let target = usize::try_from(target)
                    .unwrap_or_else(|_| panic!("version rejection before byte zero at byte {at}"));
                assert!(
                    target < code.len(),
                    "version rejection leaves the program at byte {target}"
                );
                operands.rcx_imm8_rejections.push((immediate, target));
                4
            } else if rest.starts_with(&[0x48, 0xBB]) {
                let immediate = rest
                    .get(2..10)
                    .unwrap_or_else(|| panic!("truncated mov rbx, imm64 at byte {at}"));
                operands.ring_bases.push(u64::from_le_bytes(
                    immediate.try_into().expect("eight immediate bytes"),
                ));
                10
            } else if rest.starts_with(&[0x48, 0xBC]) {
                let immediate = rest
                    .get(2..10)
                    .unwrap_or_else(|| panic!("truncated mov rsp, imm64 at byte {at}"));
                operands.stack_immediates.push(u64::from_le_bytes(
                    immediate.try_into().expect("eight immediate bytes"),
                ));
                10
            } else if rest.starts_with(&[0xCD]) {
                operands.interrupts.push(
                    *rest
                        .get(1)
                        .unwrap_or_else(|| panic!("truncated int imm8 at byte {at}")),
                );
                2
            } else if rest.starts_with(&[0x48, 0xC7, 0x83]) {
                operands
                    .writes64_from_imm32
                    .push((immediate32(3), immediate32(7)));
                11
            } else if rest.starts_with(&[0xC7, 0x83]) {
                operands.writes32.push((immediate32(2), immediate32(6)));
                10
            } else if rest.starts_with(&[0x89, 0xB3]) {
                operands.writes32_from_esi.push(immediate32(2));
                6
            } else if rest.starts_with(&[0x89, 0x83]) {
                operands.writes32_from_eax.push(immediate32(2));
                6
            } else if rest.starts_with(&[0x48, 0x81, 0x0C, 0x24]) {
                8
            } else if rest.starts_with(&[0x48, 0x8B, 0x8B]) {
                operands.reads64.push(immediate32(3));
                7
            } else if rest.starts_with(&[0x48, 0xC7, 0xC2]) {
                7
            } else if rest.starts_with(&[0xC7, 0x43]) {
                operands.short_writes32.push((
                    *rest
                        .get(2)
                        .unwrap_or_else(|| panic!("truncated disp8 at byte {at}")),
                    immediate32(3),
                ));
                7
            } else if rest.starts_with(&[0x8B, 0x43]) {
                operands.short_reads32.push(
                    *rest
                        .get(2)
                        .unwrap_or_else(|| panic!("truncated disp8 at byte {at}")),
                );
                3
            } else if rest.starts_with(&[0x8B, 0x8B]) {
                operands.reads32.push(immediate32(2));
                6
            } else if rest.starts_with(&[0xB8]) {
                operands.eax_immediates.push(immediate32(1));
                5
            } else if rest.starts_with(&[0x41, 0xB8]) {
                operands.r8d_immediates.push(immediate32(2));
                6
            } else if rest.starts_with(&[0xBF]) {
                5
            } else if rest.starts_with(&[0x48, 0x83, 0xF8]) {
                4
            } else if rest.starts_with(&[0x83, 0xFF]) {
                3
            } else if rest.starts_with(&[0x83, 0xF8]) {
                operands.eax_imm8_comparisons.push(
                    *rest
                        .get(2)
                        .unwrap_or_else(|| panic!("truncated cmp eax, imm8 at byte {at}")),
                );
                3
            } else if rest.starts_with(&[0x48, 0x85, 0xC9]) {
                operands.rcx_zero_tests += 1;
                3
            } else if rest.starts_with(&[0x48, 0x85, 0xC0]) || rest.starts_with(&[0x48, 0x39, 0xD1])
            {
                3
            } else if rest.starts_with(&[0x45, 0x31, 0xC0]) {
                operands.r8d_zeroes += 1;
                3
            } else if rest.starts_with(&[0x45, 0x85, 0xC0]) {
                operands.r8d_zero_tests += 1;
                3
            } else if rest.starts_with(&[0x0F, 0xAE, 0xE8, 0x0F, 0x31]) {
                operands.fenced_timestamp_reads += 1;
                5
            } else if rest.starts_with(&[0x41, 0x89, 0xC2]) {
                operands.r10d_timestamp_captures += 1;
                3
            } else if rest.starts_with(&[0x44, 0x29, 0xD0]) {
                operands.timestamp_subtractions += 1;
                3
            } else if rest.starts_with(&[0x31, 0xC0])
                || rest.starts_with(&[0x31, 0xFF])
                || rest.starts_with(&[0x85, 0xFF])
                || rest.starts_with(&[0xFF, 0xE0])
                || rest.starts_with(&[0x0F, 0x0B])
                || matches!(rest.first(), Some(0x74 | 0x75 | 0x78 | 0xEB))
            {
                2
            } else if rest.starts_with(&[0x85, 0xC9]) {
                operands.ecx_zero_tests += 1;
                2
            } else if rest.starts_with(&[0x39, 0xF1]) {
                operands.ecx_esi_comparisons += 1;
                2
            } else if matches!(rest.first(), Some(0x9C | 0x9D)) {
                1
            } else {
                panic!(
                    "unrecognized emitted instruction at byte {at}: {:02x?}",
                    &rest[..rest.len().min(12)]
                );
            };
            assert!(
                rest.len() >= bytes,
                "truncated emitted instruction at byte {at}"
            );
            at += bytes;
        }
        operands
    }

    fn c_struct_fields(source: &str, name: &str) -> Vec<(String, String)> {
        let (active, _) = cpp_without_comments(source);
        let structure = cpp_structure(&active);
        let marker = format!("struct {name}");
        let markers: Vec<_> = structure.match_indices(&marker).collect();
        assert_eq!(markers.len(), 1, "expected one C struct {name}");
        let start = markers[0].0;
        assert!(
            start == 0
                || !structure.as_bytes()[start - 1].is_ascii_alphanumeric()
                    && structure.as_bytes()[start - 1] != b'_',
            "C struct {name} is embedded in an identifier"
        );
        let mut open = start + marker.len();
        while structure
            .as_bytes()
            .get(open)
            .is_some_and(u8::is_ascii_whitespace)
        {
            open += 1;
        }
        assert_eq!(
            structure.as_bytes().get(open),
            Some(&b'{'),
            "unexpected tokens before C struct {name}'s body"
        );
        let close = matching(structure.as_bytes(), open, b'{', b'}');
        let mut after = close + 1;
        while structure
            .as_bytes()
            .get(after)
            .is_some_and(u8::is_ascii_whitespace)
        {
            after += 1;
        }
        assert_eq!(
            structure.as_bytes().get(after),
            Some(&b';'),
            "C struct {name} is not terminated"
        );
        let body = &active[open + 1..close];
        assert!(
            body.trim().is_empty() || body.trim_end().ends_with(';'),
            "C struct {name}'s last field is not terminated"
        );
        body.split(';')
            .filter_map(|declaration| {
                let words: Vec<_> = declaration.split_whitespace().collect();
                if words.is_empty() {
                    return None;
                }
                assert_eq!(
                    words.len(),
                    2,
                    "C struct {name} has an unparsed field `{declaration}`"
                );
                Some((words[0].to_string(), words[1].to_string()))
            })
            .collect()
    }

    fn c_typedef_kind(source: &str, name: &str) -> String {
        let (active, _) = cpp_without_comments(source);
        let structure = cpp_structure(&active);
        let mut found = Vec::new();
        for line in structure.lines() {
            let line = line.trim();
            let Some(declaration) = line.strip_prefix("typedef ") else {
                continue;
            };
            let declaration = declaration
                .strip_suffix(';')
                .unwrap_or_else(|| panic!("unterminated C typedef `{line}`"));
            let words: Vec<_> = declaration.split_whitespace().collect();
            assert_eq!(words.len(), 2, "unparsed C typedef `{line}`");
            if words[1] == name {
                found.push(words[0]);
            }
        }
        assert_eq!(found.len(), 1, "expected one C typedef {name}");
        found[0].to_string()
    }

    fn c_layout(fields: &[(String, String)]) -> std::collections::BTreeMap<String, usize> {
        let mut layout = std::collections::BTreeMap::new();
        let mut offset = 0usize;
        for (kind, name) in fields {
            let (size, alignment) = c_type_layout(kind);
            offset = (offset + alignment - 1) & !(alignment - 1);
            assert!(
                layout.insert(name.to_ascii_uppercase(), offset).is_none(),
                "duplicate C field {name}"
            );
            offset += size;
        }
        layout
    }

    fn c_type_layout(kind: &str) -> (usize, usize) {
        match kind {
            "uint32_t" => (4, 4),
            "uint64_t" => (8, 8),
            other => panic!("no checked C layout for `{other}`"),
        }
    }

    fn c_widths(fields: &[(String, String)]) -> std::collections::BTreeMap<String, usize> {
        fields
            .iter()
            .map(|(kind, name)| (name.to_ascii_uppercase(), c_type_layout(kind).0))
            .collect()
    }

    fn c_size(fields: &[(String, String)]) -> usize {
        let mut offset = 0usize;
        let mut widest = 1usize;
        for (kind, _) in fields {
            let (size, alignment) = c_type_layout(kind);
            offset = (offset + alignment - 1) & !(alignment - 1);
            offset += size;
            widest = widest.max(alignment);
        }
        (offset + widest - 1) & !(widest - 1)
    }

    fn rust_module_offsets(source: &str, name: &str) -> std::collections::BTreeMap<String, usize> {
        let source = rust_without_comments(source);
        let marker = format!("mod {name} {{");
        let markers: Vec<_> = source.match_indices(&marker).collect();
        assert_eq!(markers.len(), 1, "expected one Rust module {name}");
        let start = markers[0].0;
        assert!(
            start == 0
                || !source.as_bytes()[start - 1].is_ascii_alphanumeric()
                    && source.as_bytes()[start - 1] != b'_',
            "Rust module {name} is embedded in an identifier"
        );
        assert_unconditional_rust_line(&source, start, 0, &format!("Rust module {name}"));
        let open = start + marker.len() - 1;
        let close = matching(source.as_bytes(), open, b'{', b'}');
        let module = &source[open + 1..close];
        let mut offsets = std::collections::BTreeMap::new();
        for line in module.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let rest = line
                .strip_prefix("pub(super) const ")
                .unwrap_or_else(|| panic!("unparsed line in Rust module {name}: `{line}`"));
            let (field, value) = rest
                .split_once(": usize = ")
                .unwrap_or_else(|| panic!("unparsed offset `{line}`"));
            let value = <usize as core::str::FromStr>::from_str(
                value
                    .strip_suffix(';')
                    .unwrap_or_else(|| panic!("unterminated offset `{line}`")),
            )
            .unwrap_or_else(|_| panic!("non-numeric offset `{line}`"));
            assert!(
                offsets.insert(field.to_string(), value).is_none(),
                "duplicate Rust offset {field}"
            );
        }
        assert!(!offsets.is_empty(), "Rust module {name} has no offsets");
        offsets
    }

    fn rust_field_widths(source: &str, module: &str) -> std::collections::BTreeMap<String, usize> {
        let source = rust_without_comments(source);
        let mut widths = std::collections::BTreeMap::new();
        let marker = format!("{module}::");
        let identifier = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        for (operation, width) in [
            ("read_u32_volatile", 4usize),
            ("write_u32_volatile", 4),
            ("load_index", 4),
            ("publish_index", 4),
            ("read_u32", 4usize),
            ("write_u32", 4),
            ("read_u64", 8),
            ("write_u64", 8),
            ("read_entry_u32", 4),
            ("write_entry_u32", 4),
            ("read_entry_u64", 8),
            ("write_entry_u64", 8),
        ] {
            for (at, _) in source.match_indices(operation) {
                if at != 0 && identifier(source.as_bytes()[at - 1]) {
                    continue;
                }
                let mut open = at + operation.len();
                if source
                    .as_bytes()
                    .get(open)
                    .is_some_and(|byte| identifier(*byte))
                {
                    continue;
                }
                while source
                    .as_bytes()
                    .get(open)
                    .is_some_and(u8::is_ascii_whitespace)
                {
                    open += 1;
                }
                if source.as_bytes().get(open) != Some(&b'(') {
                    continue;
                }
                let close = matching(source.as_bytes(), open, b'(', b')');
                let call = &source[open + 1..close];
                let fields: Vec<_> = call
                    .match_indices(&marker)
                    .filter(|(field_at, _)| {
                        *field_at == 0 || !identifier(call.as_bytes()[*field_at - 1])
                    })
                    .collect();
                if fields.is_empty() {
                    continue;
                }
                assert_eq!(
                    fields.len(),
                    1,
                    "{operation} call names two {module} fields"
                );
                let field_at = fields[0].0 + marker.len();
                let field: String = call[field_at..]
                    .chars()
                    .take_while(|character| character.is_ascii_uppercase() || *character == '_')
                    .collect();
                assert!(
                    !field.is_empty(),
                    "unparsed {module} field in {operation} call"
                );
                assert!(
                    call.as_bytes()
                        .get(field_at + field.len())
                        .is_none_or(|byte| !identifier(*byte)),
                    "non-canonical {module} field in {operation} call"
                );
                if let Some(previous) = widths.insert(field.clone(), width) {
                    assert_eq!(previous, width, "kernel reads {module}::{field} two ways");
                }
            }
        }
        assert!(!widths.is_empty(), "kernel never accesses {module} fields");
        widths
    }

    #[test]
    fn the_header_inventory_accepts_spacing_and_rejects_hidden_definitions() {
        let header = r#"
# ifndef WIZARD_ABI_H
# define WIZARD_ABI_H
# include <stddef.h>
# include <stdint.h>
# ifdef __cplusplus
extern "C" {
# endif
#  define WIZ_VALUE ((uint64_t)0x1234ull)
enum wiz_call { WIZ_CALL_EXIT = 0 };
# ifdef __cplusplus
}
# endif
# endif
"#;
        let (_, macros) = checked_header(header);
        assert_eq!(macros["WIZ_VALUE"], "((uint64_t)0x1234ull)");
        let raw_decoy = header.replace(
            "#  define WIZ_VALUE ((uint64_t)0x1234ull)",
            "const char *decoy = R\"wiz(\n#  define WIZ_VALUE ((uint64_t)0x1234ull)\n)wiz\";",
        );
        let (_, raw_decoy_macros) = checked_header(&raw_decoy);
        assert!(
            !raw_decoy_macros.contains_key("WIZ_VALUE"),
            "a raw-string directive entered the active macro inventory"
        );

        for corrupted in [
            header.replace(
                "#  define WIZ_VALUE ((uint64_t)0x1234ull)",
                "# if 0\n#  define WIZ_VALUE ((uint64_t)0x1234ull)\n# endif",
            ),
            header.replace("enum wiz_call", "# define WIZ_VALUE 1u\nenum wiz_call"),
            header.replace("#  define WIZ_VALUE", "%:define WIZ_VALUE"),
            header.replace("enum wiz_call", "_Pragma(\"pack(1)\")\nenum wiz_call"),
        ] {
            assert!(
                std::panic::catch_unwind(|| checked_header(&corrupted)).is_err(),
                "a non-canonical header scaffold was accepted"
            );
        }
    }

    #[test]
    fn the_c_integer_parser_admits_only_exact_unsigned_values() {
        assert_eq!(parse_c_uint("((wiz_handle_t)0)"), Ok(0));
        assert_eq!(parse_c_uint("((wiz_rights_t)(1u << 14))"), Ok(1 << 14));
        assert_eq!(
            parse_c_uint("((uintptr_t)0x0000200000000000ull)"),
            Ok(0x0000_2000_0000_0000)
        );
        for expression in [
            "08",
            "1u | 2u",
            "1u + 1u",
            "1u << 32",
            "0xffffffffu << 1",
            "WIZ_OTHER",
            "((uint32_t)1)",
        ] {
            assert!(
                parse_c_uint(expression).is_err(),
                "accepted non-restricted C integer {expression}"
            );
        }
    }

    #[test]
    fn decoder_inventory_rejects_aliases_and_noncanonical_arms() {
        let signature = "const fn decode(value: u32) -> Self {";
        let unknown = "other=>Self::Unknown(other)";
        let valid = r#"
impl Opcode {
    const fn decode(value: u32) -> Self {
        match value {
            1 => Self::Echo,
            2 => Self::Uptime,
            other => Self::Unknown(other),
        }
    }
}
"#;
        assert_eq!(
            simple_decoder_arms(valid, "Opcode", signature, unknown).len(),
            2
        );
        for corrupted in [
            valid.replace("2 => Self::Uptime", "1 => Self::Uptime"),
            valid.replace("2 => Self::Uptime", "2 => Self::Echo"),
            valid.replace("2 => Self::Uptime", "1 | 2 => Self::Uptime"),
            valid.replace("2 => Self::Uptime", "2 => { Self::Uptime }"),
            format!("#[cfg(any())]\n{valid}"),
            format!("macro_rules! decoy (\n() => (\n{valid}\n);\n);\n"),
        ] {
            assert!(
                std::panic::catch_unwind(|| {
                    simple_decoder_arms(&corrupted, "Opcode", signature, unknown)
                })
                .is_err(),
                "accepted a decoder alias or noncanonical arm"
            );
        }
    }

    #[test]
    fn c_declaration_inventory_rejects_decoys_duplicates_and_missing_terminators() {
        let source = r#"
const char *decoy = "struct wire { uint64_t wrong; };";
typedef uint32_t wire_word_t;
struct wire {
    uint32_t first;
    uint64_t second;
};
enum wiz_call {
    WIZ_CALL_EXIT = 0,
    WIZ_CALL_WAIT = 1,
};
"#;
        assert_eq!(
            c_struct_fields(source, "wire"),
            [
                ("uint32_t".to_string(), "first".to_string()),
                ("uint64_t".to_string(), "second".to_string()),
            ]
        );
        assert_eq!(c_typedef_kind(source, "wire_word_t"), "uint32_t");
        assert_eq!(c_enum_values(source, "wiz_call").len(), 2);
        for corrupted in [
            format!("{source}\nstruct wire {{ uint32_t third; }};"),
            source.replace("uint64_t second;", "uint64_t second"),
            source.replace("WIZ_CALL_WAIT = 1", "WIZ_CALL_WAIT = 0"),
            format!("{source}\ntypedef uint64_t wire_word_t;"),
        ] {
            let struct_rejected =
                std::panic::catch_unwind(|| c_struct_fields(&corrupted, "wire")).is_err();
            let enum_rejected =
                std::panic::catch_unwind(|| c_enum_values(&corrupted, "wiz_call")).is_err();
            let typedef_rejected =
                std::panic::catch_unwind(|| c_typedef_kind(&corrupted, "wire_word_t")).is_err();
            assert!(
                struct_rejected || enum_rejected || typedef_rejected,
                "a malformed C declaration was accepted"
            );
        }
    }

    #[test]
    fn rust_offset_inventory_rejects_conditional_or_duplicate_modules() {
        let module = r#"
mod submission {
    pub(super) const OPCODE: usize = 0;
    pub(super) const TOKEN: usize = 4;
}
"#;
        assert_eq!(rust_module_offsets(module, "submission").len(), 2);
        for corrupted in [
            format!("#[cfg(any())]\n{module}"),
            format!("{module}\n{module}"),
            module.replace("TOKEN: usize = 4;", "TOKEN: usize = 2 + 2;"),
            format!("macro_rules! decoy (\n() => (\n{module}\n);\n);\n"),
        ] {
            assert!(
                std::panic::catch_unwind(|| rust_module_offsets(&corrupted, "submission")).is_err(),
                "a conditional, duplicate or computed offset module was accepted"
            );
        }
    }

    #[test]
    fn rust_field_width_inventory_follows_balanced_multiline_calls() {
        let source = r#"
fn live() {
    read_u32(
        page,
        header::VERSION,
    );
    write_entry_u64(
        bytes,
        submission::ARGUMENT,
        value,
    );
}
"#;
        assert_eq!(rust_field_widths(source, "header")["VERSION"], 4);
        assert_eq!(rust_field_widths(source, "submission")["ARGUMENT"], 8);
        let conflicting = source.replace("read_u32", "read_u64") + source;
        assert!(
            std::panic::catch_unwind(|| rust_field_widths(&conflicting, "header")).is_err(),
            "line-broken accesses hid conflicting field widths"
        );
    }

    #[test]
    fn rust_integer_witnesses_must_be_unconditional_file_items() {
        let declaration = "pub const VALUE: u64";
        let valid = "pub const VALUE: u64 = 0x20;\n";
        assert_eq!(rust_uint(valid, declaration), 0x20);
        for corrupted in [
            format!("#[cfg(any())]\n{valid}"),
            format!("mod decoy {{\n    {valid}}}\n"),
            format!("{valid}{valid}"),
            valid.trim_end_matches(";\n").to_string(),
            format!("macro_rules! decoy (\n() => (\n{valid}\n);\n);\n"),
        ] {
            assert!(
                std::panic::catch_unwind(|| rust_uint(&corrupted, declaration)).is_err(),
                "accepted a conditional, nested, duplicate or unterminated Rust integer"
            );
        }
    }

    #[test]
    fn typed_vector_witnesses_follow_the_checked_assignment() {
        let valid = concat!(
            "const SOURCES: [InterruptVector; 2] = match Assignment::new([0x40, 0x80]) {\n",
            "    Some(assignment) => assignment.into_array(),\n",
            "    None => panic!(\"invalid\"),\n",
            "};\n",
            "pub(crate) const SYSCALL: InterruptVector = SOURCES[1];\n",
        );
        assert_eq!(rust_interrupt_vector(valid, "SYSCALL"), 0x80);
        for corrupted in [
            valid.replace("const SOURCES", "#[cfg(any())]\nconst SOURCES"),
            format!("mod hidden {{\n{valid}}}\n"),
            format!("{valid}{valid}"),
            valid.replace("SYSCALL: InterruptVector", "SYSCALL: u8"),
            valid.replace("SOURCES[1]", "SOURCES[2]"),
            valid.replace("assignment.into_array()", "SOURCES"),
            valid.replace("[InterruptVector; 2]", "[InterruptVector; 3]"),
        ] {
            assert!(
                std::panic::catch_unwind(|| rust_interrupt_vector(&corrupted, "SYSCALL")).is_err(),
                "accepted a hidden, duplicate, mistyped or diverted typed vector"
            );
        }
    }

    #[test]
    fn rust_status_inventory_rejects_hidden_or_noncanonical_items() {
        let valid = concat!(
            "pub(crate) const STATUS_OK: u32 = 0;\n",
            "pub(crate) const STATUS_BAD: u32 = 1;\n",
        );
        assert_eq!(rust_status_constants(valid).len(), 2);
        for corrupted in [
            valid.replace("pub(crate) const STATUS_BAD", "const STATUS_BAD"),
            valid.replace("const STATUS_BAD", "const\nSTATUS_BAD"),
            format!("#[cfg(any())]\n{valid}"),
            format!("macro_rules! decoy (\n() => (\n{valid}\n);\n);\n"),
            valid.replace("STATUS_BAD: u32 = 1", "STATUS_BAD: u32 = 0"),
        ] {
            assert!(
                std::panic::catch_unwind(|| rust_status_constants(&corrupted)).is_err(),
                "accepted a hidden, noncanonical or aliased Rust status"
            );
        }
    }

    #[test]
    fn source_witnesses_ignore_comments_and_literals() {
        let rust = r###"
const NORMAL: &str = "DECOY_NORMAL";
const RAW: &str = r#"DECOY_RAW"#;
const BYTE: u8 = b'D';
// DECOY_LINE
/* DECOY_BLOCK */
const LIVE: u32 = 7;
"###;
        let active = rust_without_comments(rust);
        for decoy in ["DECOY_NORMAL", "DECOY_RAW", "DECOY_LINE", "DECOY_BLOCK"] {
            assert!(
                !active.contains(decoy),
                "Rust literal/comment decoy survived"
            );
        }
        assert!(active.contains("const LIVE: u32 = 7;"));

        let cpp = "auto a = R\"tag(DECOY_RAW // not a comment)tag\"; // DECOY_LINE\nint live;";
        let (active, comments) = cpp_without_comments(cpp);
        assert_eq!(comments, ["DECOY_LINE"]);
        let structure = cpp_structure(&active);
        assert!(!structure.contains("DECOY_RAW"));
        assert!(structure.contains("int live;"));
    }

    #[test]
    fn emitted_operand_decoder_skips_opcode_bytes_inside_immediates() {
        let ring = 0x0000_2000_0000_0000u64;
        let mut code = vec![0xB8, 0xCD, 0x81, 0, 0, 0xCD, 0x80, 0x48, 0xBB];
        code.extend_from_slice(&ring.to_le_bytes());
        let operands = decode_program_operands(&code);
        assert_eq!(operands.eax_immediates, [0x0000_81CD]);
        assert_eq!(operands.interrupts, [0x80]);
        assert_eq!(operands.ring_bases, [ring]);
        assert!(
            std::panic::catch_unwind(|| decode_program_operands(&[0x90])).is_err(),
            "the restricted decoder accepted an unregistered instruction"
        );
    }

    #[test]
    fn emitted_operand_decoder_tracks_the_entry_handle() {
        let code = [
            0x59, 0x48, 0x83, 0xF9, 0x01, 0x75, 0x0F, 0x58, 0x5E, 0x89, 0xB3, 0x08, 0x10, 0, 0,
            0x85, 0xC9, 0x48, 0x85, 0xC9, 0x39, 0xF1, 0x0F, 0x0B,
        ];
        let operands = decode_program_operands(&code);
        assert_eq!(operands.pops, ["rcx", "rax", "rsi"]);
        assert_eq!(operands.rcx_imm8_rejections, [(1, 22)]);
        assert_eq!(&code[operands.rcx_imm8_rejections[0].1..], &[0x0F, 0x0B]);
        assert_eq!(operands.writes32_from_esi, [0x1008]);
        assert_eq!(operands.ecx_zero_tests, 1);
        assert_eq!(operands.rcx_zero_tests, 1);
        assert_eq!(operands.ecx_esi_comparisons, 1);

        let narrowed = [0x59, 0x83, 0xF9, 0x01];
        assert!(
            std::panic::catch_unwind(|| decode_program_operands(&narrowed)).is_err(),
            "a 32-bit comparison accepted a version word with unchecked high bits"
        );
    }

    #[test]
    fn entry_version_must_fit_the_sign_extended_immediate() {
        assert_eq!(sign_extended_imm8(1), Some(1));
        assert_eq!(sign_extended_imm8(127), Some(127));
        assert_eq!(sign_extended_imm8(128), None);
        assert_eq!(sign_extended_imm8(u64::MAX), None);
    }

    #[test]
    fn clock_arithmetic_widens_before_multiplication() {
        let source = r#"
            uint64_t ticks = stamp - last;
            unsigned __int128 advanced =
                static_cast<unsigned __int128>(fraction)
                + static_cast<unsigned __int128>(ticks) * multiplier;
            unsigned __int128 whole = advanced >> shift;

            uint64_t ns;
            if(whole > UINT64_MAX) {
                ns = UINT64_MAX;
            } else {
                uint64_t delta = static_cast<uint64_t>(whole);
                ns = base > UINT64_MAX - delta ? UINT64_MAX : base + delta;
            }
            *secs = 0;
        "#;
        assert_clock_arithmetic(source);

        let narrowed = source.replace(
            "static_cast<unsigned __int128>(ticks) * multiplier",
            "ticks * multiplier",
        );
        assert!(
            std::panic::catch_unwind(|| assert_clock_arithmetic(&narrowed)).is_err(),
            "a 64-bit multiply was accepted merely because its destination is 128-bit"
        );
    }

    #[test]
    fn every_sysdep_definition_agrees_with_its_register_row() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let sysdeps =
            std::fs::read_to_string(root.join("fixtures/abi-pressure/sysdeps/wizard/generic.cpp"))
                .expect("generic.cpp");
        let readme = std::fs::read_to_string(root.join("fixtures/abi-pressure/README.md"))
            .expect("README.md");
        let (active, comments) = cpp_without_comments(&sysdeps);
        let structure = cpp_structure(&active);
        let mut includes = Vec::new();
        for (line, structural_line) in active.lines().zip(structure.lines()) {
            let structural_line = structural_line.trim_start();
            assert!(
                !structural_line.starts_with("%:"),
                "the pressure-test transcription uses a digraph directive `{line}`"
            );
            assert!(
                !structural_line
                    .split(|character: char| {
                        !character.is_ascii_alphanumeric() && character != '_'
                    })
                    .any(|word| matches!(word, "_Pragma" | "__pragma")),
                "the pressure-test transcription uses a pragma operator `{line}`"
            );
            if cpp_directive(structural_line).is_none() {
                continue;
            }
            let (kind, rest) = cpp_directive(line).expect("structural directive has source");
            assert_eq!(
                kind, "include",
                "the pressure-test transcription permits includes, not `{line}`"
            );
            includes.push(rest);
        }
        assert_eq!(
            includes,
            [
                "<mlibc/all-sysdeps.hpp>",
                "<mlibc/debug.hpp>",
                "<stdint.h>",
                "<time.h>",
                "\"wizard_abi.h\"",
            ],
            "the pressure-test transcription's include set changed"
        );
        let annotations = sysdep_annotations(&comments);
        let bodies = cpp_bodies(&active);
        let definition_names: std::collections::BTreeSet<_> = bodies
            .keys()
            .filter(|name| name.starts_with("sys_"))
            .cloned()
            .collect();
        assert_eq!(
            definition_names.len(),
            16,
            "D-114 inspected sixteen active sysdep definitions"
        );
        assert_eq!(
            definition_names,
            annotations.keys().cloned().collect(),
            "a definition is unregistered or a marker has no active definition"
        );

        let mut register = std::collections::BTreeMap::new();
        for line in readme.lines() {
            let cells: Vec<_> = line.split('|').map(str::trim).collect();
            if cells.len() < 5 {
                continue;
            }
            let name = cells[1].trim_matches('`');
            if !name.starts_with("sys_") {
                continue;
            }
            assert!(
                register
                    .insert(
                        name.to_string(),
                        (cells[2].to_string(), cells[3].to_string())
                    )
                    .is_none(),
                "{name} has two README rows"
            );
        }
        assert_eq!(
            register, annotations,
            "the README and C++ register disagree"
        );

        let vectors = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/arch/x86_64/vector.rs"))
                .expect("vector.rs"),
        );
        let syscall = u8::try_from(rust_interrupt_vector(&vectors, "SYSCALL"))
            .expect("the syscall vector fits in an interrupt immediate");
        assert_eq!(
            compact_cpp(bodies["call1"].0),
            "inlinelongcall1(longnumber,longa)",
            "call1's declarator is not the exact native-width gate"
        );
        assert_eq!(
            compact_cpp(bodies["call1"].1),
            format!(
                "longresult;asmvolatile(\"int${syscall:#x}\":\"=a\"(result):\"a\"(number),\"D\"(a):\"memory\");returnresult;"
            ),
            "call1 is not the exact one-argument native gate"
        );
        assert_eq!(
            compact_cpp(bodies["as_errno"].0),
            "inlineintas_errno(longresult)",
            "as_errno's declarator does not preserve signed native results"
        );
        assert_eq!(
            compact_cpp(bodies["as_errno"].1),
            "returnresult<0?static_cast<int>(-result):0;",
            "as_errno does not decode exactly the native negative-result convention"
        );

        for (name, (body_state, _)) in &annotations {
            let implementation = bodies
                .get(name)
                .unwrap_or_else(|| panic!("no active definition for {name}"))
                .1;
            match body_state.as_str() {
                "WIRED" if name == "sys_exit" => assert_eq!(
                    compact_cpp(implementation),
                    "call1(WIZ_CALL_EXIT,status);__builtin_unreachable();"
                ),
                "WIRED" if name == "sys_tcb_set" => assert_eq!(
                    compact_cpp(implementation),
                    "returnas_errno(call1(WIZ_CALL_TCB_SET,reinterpret_cast<long>(pointer)));"
                ),
                "COMPOSED" => {
                    assert_eq!(name, "sys_libc_panic");
                    assert_eq!(
                        compact_cpp(implementation),
                        "sys_libc_log(\"mlibc:panic\\n\");sys_exit(127);__builtin_unreachable();"
                    );
                }
                "PARTIAL" => assert_eq!(name, "sys_clock_get"),
                "STUB" if name == "sys_libc_log" => {
                    assert_eq!(compact_cpp(implementation), "(void)message;");
                }
                "STUB" => {
                    let compact = compact_cpp(implementation);
                    assert!(
                        compact.ends_with(';'),
                        "{name}'s stub has a partial statement"
                    );
                    let statements: Vec<_> = compact
                        .split(';')
                        .filter(|statement| !statement.is_empty())
                        .collect();
                    assert_eq!(
                        statements.last(),
                        Some(&"returnWIZ_ENOSYS"),
                        "{name} is not an explicit one-path stub"
                    );
                    for statement in &statements[..statements.len() - 1] {
                        let argument = statement.strip_prefix("(void)").unwrap_or_else(|| {
                            panic!("{name}'s stub executes `{statement}` before refusing")
                        });
                        assert!(
                            !argument.is_empty()
                                && argument
                                    .bytes()
                                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
                            "{name}'s discard `{statement}` is not one parameter"
                        );
                    }
                }
                state => panic!("{name} has unproved BODY={state}"),
            }
        }

        let clock = &cpp_structure(bodies["sys_clock_get"].1);
        for required in [
            "clock != CLOCK_MONOTONIC",
            "return WIZ_EINVAL;",
            "constants->version != WIZ_TIME_VERSION",
            "WIZ_TIME_CLOCK_AT + sizeof(wiz_time_clock)",
            "!constants->counter_hz",
            "before != after || (after & 1u)",
            "!multiplier || shift > 63",
            "fraction >= (uint64_t{1} << shift)",
            "uint64_t ticks = stamp - last;",
            "unsigned __int128 advanced",
            "unsigned __int128 whole",
            "whole > UINT64_MAX",
            "base > UINT64_MAX - delta",
            "ns = UINT64_MAX;",
            "ns = base > UINT64_MAX - delta ? UINT64_MAX : base + delta;",
            "*secs = static_cast<time_t>(ns / 1000000000ull);",
            "*nanos = static_cast<long>(ns % 1000000000ull);",
            "return 0;",
            "monotonic_fallback(secs, nanos)",
        ] {
            assert!(clock.contains(required), "clock reader lost `{required}`");
        }
        assert_clock_arithmetic(clock);
        let before = clock.find("uint32_t before").expect("clock sequence start");
        let stamp = clock.find("read_counter()").expect("clock counter read");
        let after = clock.find("uint32_t after").expect("clock sequence end");
        assert!(
            before < stamp && stamp < after,
            "the counter read escaped the sequence window"
        );
        let seconds = clock.find("*secs =").expect("clock seconds store");
        let nanos = clock.find("*nanos =").expect("clock nanoseconds store");
        let success = clock.find("return 0;").expect("clock success return");
        assert!(
            after < seconds && seconds < nanos && nanos < success,
            "the clock returns success before publishing its result"
        );
        assert_eq!(
            clock
                .matches("__atomic_thread_fence(__ATOMIC_ACQUIRE);")
                .count(),
            2,
            "the sequence snapshot needs a compiler fence on both sides"
        );
        let counter = bodies["read_counter"].1;
        assert!(
            counter.contains("lfence\\n\\trdtsc\\n\\tlfence"),
            "the userspace counter read is not fenced on both sides"
        );
        let counter = compact_cpp(counter);
        assert_eq!(
            counter,
            "uint32_tlow;uint32_thigh;asmvolatile(\"lfence\\n\\trdtsc\\n\\tlfence\":\"=a\"(low),\"=d\"(high)::\"memory\");return(static_cast<uint64_t>(high)<<32)|low;",
            "the counter reader is not the exact fenced TSC transcription"
        );
        let counter_structure = compact_cpp(&cpp_structure(bodies["read_counter"].1));
        for dead in ["if(false)", "if(0)", "while(false)", "while(0)"] {
            assert!(
                !counter_structure.contains(dead),
                "the counter reader contains a literal dead wrapper `{dead}`"
            );
        }
        let counter_returns: Vec<_> = counter_structure
            .match_indices("return")
            .map(|(at, _)| {
                counter_structure[at..]
                    .split_once(';')
                    .map(|(statement, _)| format!("{statement};"))
                    .expect("terminated counter return")
            })
            .collect();
        assert_eq!(
            counter_returns,
            ["return(static_cast<uint64_t>(high)<<32)|low;"],
            "the counter reader has an early return"
        );

        let clock_structure = compact_cpp(clock);
        assert_eq!(
            clock_structure.matches("ticks=").count(),
            1,
            "the measured counter delta is overwritten"
        );
        for dead in ["if(false)", "if(0)", "while(false)", "while(0)"] {
            assert!(
                !clock_structure.contains(dead),
                "the clock reader contains a literal dead path `{dead}`"
            );
        }
        assert!(
            clock_structure
                .starts_with("if(clock!=CLOCK_MONOTONIC)returnWIZ_EINVAL;auto*constants="),
            "the clock reader does not begin with its clock-domain guard"
        );
        assert!(
            !clock_structure.contains("goto") && !clock_structure.contains("__builtin_unreachable"),
            "the clock reader bypasses its structured success path"
        );
        let clock_returns: Vec<_> = clock_structure
            .match_indices("return")
            .map(|(at, _)| {
                clock_structure[at..]
                    .split_once(';')
                    .map(|(statement, _)| format!("{statement};"))
                    .expect("terminated clock return")
            })
            .collect();
        assert_eq!(
            clock_returns,
            [
                "returnWIZ_EINVAL;",
                "returnmonotonic_fallback(secs,nanos);",
                "returnmonotonic_fallback(secs,nanos);",
                "return0;",
                "returnmonotonic_fallback(secs,nanos);",
            ],
            "the clock reader gained an unaccounted return path"
        );
        assert_eq!(
            compact_cpp(bodies["monotonic_fallback"].1),
            "(void)secs;(void)nanos;returnWIZ_ENOSYS;",
            "the unfinished Uptime route does not refuse on one path"
        );
    }

    #[test]
    fn disputed_sysdep_remainders_have_platform_witnesses() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let sysdeps =
            std::fs::read_to_string(root.join("fixtures/abi-pressure/sysdeps/wizard/generic.cpp"))
                .expect("generic.cpp");
        let (_, comments) = cpp_without_comments(&sysdeps);
        let annotations = sysdep_annotations(&comments);
        let remainder = |name: &str| {
            annotations
                .get(name)
                .unwrap_or_else(|| panic!("no marker for {name}"))
                .1
                .as_str()
        };

        let queue = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/queue.rs")).expect("queue.rs"),
        );
        let vfs = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/vfs.rs")).expect("vfs.rs"),
        );
        assert_eq!(remainder("sys_read"), "COMPAT");
        assert_eq!(remainder("sys_seek"), "COMPAT");
        assert!(queue.contains("let offset = u32::try_from(submission.offset)"));
        assert!(vfs.contains("pub(crate) fn read(file: File, offset: u32,"));

        assert_eq!(remainder("sys_open"), "PARTIAL");
        assert!(queue.contains("6 => Self::OpenAt"));
        assert!(queue.contains("table.resolve(submission.handle)"));
        assert!(!queue.contains("Self::ChannelReceive"));

        let handles = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/handle.rs")).expect("handle.rs"),
        );
        let objects = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/object.rs")).expect("object.rs"),
        );
        assert_eq!(remainder("sys_close"), "MISSING");
        assert!(!handles.contains("pub(crate) fn close(&self, handle: u32)"));
        assert!(objects.contains("Channel(Arc<Channel>)"));
        assert!(!queue.contains("=> Self::Close"));

        assert_eq!(remainder("sys_write"), "MISSING");
        assert!(queue.contains("7 => Self::Read"));
        assert!(!queue.contains("=> Self::Write"));

        let wait = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/wait.rs")).expect("wait.rs"),
        );
        assert_eq!(remainder("sys_futex_wait"), "MISSING");
        assert_eq!(remainder("sys_futex_wake"), "MISSING");
        assert!(wait.contains("wait_unless(&self, context: &mut ThreadContext, ready:"));
        assert!(!wait.contains("user_address"));
        assert!(!queue.contains("Self::Futex"));

        let paging = rust_without_comments(
            &std::fs::read_to_string(root.join("wost/src/arch/x86_64/paging.rs"))
                .expect("paging.rs"),
        );
        let memory = rust_without_comments(
            &std::fs::read_to_string(root.join("wost/src/mm/mod.rs")).expect("mm/mod.rs"),
        );
        assert_eq!(remainder("sys_vm_unmap"), "PARTIAL");
        assert_eq!(remainder("sys_anon_free"), "PARTIAL");
        assert!(paging.contains("pub fn unmap(&mut self, virtual_base: u64, len: u64)"));
        assert!(paging.contains("impl Drop for Unmapped"));
        assert!(paging.contains("if frames.zero()"));
        assert!(memory.contains("impl Drop for Segment"));
        assert!(memory.contains("FRAMES.lock_leaf().free_contiguous(self.first, self.count)"));
        assert!(!queue.contains("=> Self::Unmap"));
        assert!(!queue.contains("=> Self::AnonFree"));

        let user = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/user.rs")).expect("user.rs"),
        );
        assert_eq!(remainder("sys_libc_panic"), "PARTIAL");
        assert_eq!(remainder("sys_exit"), "PARTIAL");
        let exit = user
            .split_once("Call::Exit => {")
            .and_then(|(_, source)| source.split_once("Call::RingEnter =>"))
            .map(|(body, _)| body)
            .expect("Call::Exit arm");
        assert!(exit.contains("thread::retire_from_trap(frame)"));
        assert!(!exit.contains("frame.rdi"));
    }

    #[test]
    fn the_c_header_and_the_kernel_agree_about_the_abi() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let header = std::fs::read_to_string(root.join("fixtures/abi-pressure/abi/wizard_abi.h"))
            .expect("fixtures/abi-pressure/abi/wizard_abi.h");
        let queue = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/queue.rs")).expect("queue.rs"),
        );
        let user = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/user.rs")).expect("user.rs"),
        );
        let proto = rust_without_comments(
            &std::fs::read_to_string(root.join("libs/ring-proto/src/lib.rs")).expect("ring-proto"),
        );
        let handle_space = rust_without_comments(
            &std::fs::read_to_string(root.join("libs/handle-space/src/lib.rs"))
                .expect("handle-space"),
        );
        let x86_paging = rust_without_comments(
            &std::fs::read_to_string(root.join("libs/x86-paging/src/lib.rs")).expect("x86-paging"),
        );
        let timepage_source = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/timepage.rs")).expect("timepage.rs"),
        );
        let vectors_source = rust_without_comments(
            &std::fs::read_to_string(root.join("kernel/src/arch/x86_64/vector.rs"))
                .expect("vector.rs"),
        );
        let paging_source = rust_without_comments(
            &std::fs::read_to_string(root.join("wost/src/arch/x86_64/paging.rs"))
                .expect("paging.rs"),
        );
        let (active_header, macro_expressions) = checked_header(&header);
        assert_eq!(c_typedef_kind(&active_header, "wiz_handle_t"), "uint32_t");
        assert_eq!(c_typedef_kind(&active_header, "wiz_rights_t"), "uint32_t");
        assert_eq!(c_typedef_kind(&active_header, "wiz_errno_t"), "int");

        let declared_macros: std::collections::BTreeMap<String, u64> = macro_expressions
            .iter()
            .map(|(name, expression)| {
                let value = parse_c_uint(expression)
                    .unwrap_or_else(|error| panic!("unparsed {name}: {error}"));
                (name.clone(), value)
            })
            .collect();
        let mut expected_macros = std::collections::BTreeMap::new();
        macro_rules! expect_macro {
            ($name:expr, $value:expr) => {
                assert!(
                    expected_macros.insert($name.to_string(), $value).is_none(),
                    "two kernel witnesses register {}",
                    $name
                )
            };
        }

        let reserved = rust_uint(&handle_space, "pub const RESERVED: u32");
        let reserved_bits = rust_uint(&handle_space, "pub const RESERVED_BITS: u32");
        assert!(reserved_bits < u64::from(u64::BITS));
        assert_eq!(reserved, (1u64 << reserved_bits) - 1);
        let invalid = reserved & !reserved;
        assert_ne!(
            invalid & reserved,
            reserved,
            "invalid handle passes decode's low-bit gate"
        );
        expect_macro!("WIZ_HANDLE_INVALID", invalid);

        let rights_marker = "impl Rights {";
        let rights_markers: Vec<_> = handle_space.match_indices(rights_marker).collect();
        assert_eq!(rights_markers.len(), 1, "expected one impl Rights");
        let rights_at = rights_markers[0].0;
        assert_unconditional_rust_line(&handle_space, rights_at, 0, "impl Rights");
        let rights_open = rights_at + rights_marker.len() - 1;
        let rights_close = matching(handle_space.as_bytes(), rights_open, b'{', b'}');
        let rights_impl = &handle_space[rights_open + 1..rights_close];
        assert!(
            !rights_impl.contains("#["),
            "impl Rights contains a conditionally visible item"
        );
        let compact_rights: String = rights_impl
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        let mut right_bits = std::collections::BTreeSet::new();
        let mut right_names = std::collections::BTreeSet::new();
        let mut at = 0usize;
        while let Some(found) = compact_rights[at..].find("pubconst") {
            let declaration_at = at + found;
            let rest = &compact_rights[declaration_at + "pubconst".len()..];
            let semicolon = rest
                .find(';')
                .unwrap_or_else(|| panic!("unterminated public item in impl Rights"));
            let declaration = &rest[..semicolon];
            at = declaration_at + "pubconst".len() + semicolon + 1;
            if declaration.starts_with("fn") {
                continue;
            }
            let (name, expression) = declaration
                .split_once(":Self=")
                .unwrap_or_else(|| panic!("unparsed public Rights item `{declaration}`"));
            if matches!(name, "NONE" | "UNIVERSAL" | "ALL") {
                continue;
            }
            assert!(
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte == b'_'),
                "malformed right name {name}"
            );
            assert!(right_names.insert(name), "duplicate right {name}");
            let bit = expression
                .strip_prefix("Self(1<<")
                .and_then(|value| value.strip_suffix(')'))
                .unwrap_or_else(|| panic!("right {name} is not one literal bit"));
            let bit = parse_rust_uint(bit)
                .unwrap_or_else(|error| panic!("unparsed right {name}: {error}"));
            assert!(bit < 32, "right {name} is outside wiz_rights_t");
            assert!(right_bits.insert(bit), "two rights use bit {bit}");
            expect_macro!(&format!("WIZ_RIGHT_{name}"), 1u64 << bit);
        }
        assert!(
            !right_bits.is_empty(),
            "the kernel defines no individual rights"
        );

        let ring = rust_uint(&user, "pub(crate) const RING: u64");
        let entry_version = rust_uint(&user, "pub(crate) const ENTRY_VERSION: u64");
        let entry_bytes = rust_uint(&user, "pub(crate) const ENTRY_BYTES: usize");
        let entry_layout = rust_module_offsets(&user, "entry");
        expect_macro!("WIZ_ENTRY_VERSION", entry_version);
        expect_macro!("WIZ_ENTRY_VERSION_AT", entry_layout["VERSION"] as u64);
        expect_macro!("WIZ_ENTRY_ARGUMENT_AT", entry_layout["ARGUMENT"] as u64);
        expect_macro!(
            "WIZ_ENTRY_BOOTSTRAP_HANDLE_AT",
            entry_layout["BOOTSTRAP_HANDLE"] as u64
        );
        expect_macro!("WIZ_ENTRY_BYTES", entry_bytes);
        expect_macro!("WIZ_RING_ADDRESS", ring);
        expect_macro!(
            "WIZ_RING_VERSION",
            rust_uint(&proto, "pub const VERSION: u32")
        );
        let page_size = rust_uint(&x86_paging, "pub const PAGE_SIZE: u64");
        assert_eq!(
            rust_expression(&queue, "const ENTRIES_AT: usize"),
            "PAGE_SIZE as usize"
        );
        assert_eq!(
            rust_expression(&queue, "const DATA_AT: usize"),
            "2 * PAGE_SIZE as usize"
        );
        assert_eq!(
            rust_expression(&queue, "const DATA_BYTES: usize"),
            "PAGE_SIZE as usize"
        );
        expect_macro!("WIZ_RING_ENTRIES_AT", page_size);
        expect_macro!(
            "WIZ_RING_DATA_AT",
            page_size.checked_mul(2).expect("two pages fit in u64")
        );
        expect_macro!("WIZ_RING_DATA_BYTES", page_size);

        let decoded_opcode_arms = simple_decoder_arms(
            &queue,
            "Opcode",
            "const fn decode(value: u32) -> Self {",
            "other=>Self::Unknown(other)",
        );
        for (number, variant) in &decoded_opcode_arms {
            expect_macro!(&format!("WIZ_OP_{}", variant.to_ascii_uppercase()), *number);
        }
        let decoded_call_arms = simple_decoder_arms(
            &user,
            "Call",
            "const fn decode(value: u64) -> Self {",
            "other=>Self::Unknown(other)",
        );

        for (name, number) in rust_status_constants(&queue) {
            expect_macro!(&format!("WIZ_STATUS_{name}"), number);
        }

        expect_macro!(
            "WIZ_SUBMISSION_BYTES",
            rust_uint(&queue, "const SUBMISSION_BYTES: usize")
        );
        expect_macro!(
            "WIZ_COMPLETION_BYTES",
            rust_uint(&queue, "const COMPLETION_BYTES: usize")
        );
        expect_macro!(
            "WIZ_TIME_ADDRESS",
            rust_uint(&timepage_source, "pub(crate) const TIME: u64")
        );
        expect_macro!(
            "WIZ_TIME_VERSION",
            rust_uint(&timepage_source, "pub(crate) const VERSION: u32")
        );
        expect_macro!(
            "WIZ_TIME_CLOCK_AT",
            rust_uint(&timepage_source, "const CLOCK_AT: usize")
        );
        expect_macro!(
            "WIZ_TIME_MODE_NONE",
            rust_uint(&timepage_source, "pub(crate) const MODE_NONE: u32")
        );
        expect_macro!(
            "WIZ_TIME_MODE_COUNTER",
            rust_uint(&timepage_source, "pub(crate) const MODE_COUNTER: u32")
        );
        let syscall_number = rust_interrupt_vector(&vectors_source, "SYSCALL");
        expect_macro!("WIZ_SYSCALL_VECTOR", syscall_number);
        let user_end = rust_uint(&paging_source, "pub const USER_END: u64");
        expect_macro!("WIZ_USER_END", user_end);
        expect_macro!("WIZ_ENOSYS", rust_negated_errno(&user, "ENOSYS"));
        expect_macro!("WIZ_EINVAL", rust_negated_errno(&user, "EINVAL"));
        assert_eq!(
            declared_macros, expected_macros,
            "the complete C macro inventory differs from the kernel ABI"
        );

        // Complete domains keep new members from hiding outside a
        // hand-maintained list.
        let declared_opcodes: std::collections::BTreeMap<_, _> = declared_macros
            .iter()
            .filter(|(name, _)| name.starts_with("WIZ_OP_"))
            .map(|(name, number)| (name.clone(), *number))
            .collect();
        let decoded_opcodes: std::collections::BTreeMap<_, _> = decoded_opcode_arms
            .iter()
            .map(|(number, variant)| (format!("WIZ_OP_{}", variant.to_ascii_uppercase()), *number))
            .collect();
        assert_eq!(
            declared_opcodes, decoded_opcodes,
            "the C opcode domain differs from Opcode::decode"
        );

        // The complete entry layouts; the C declarations and emitted
        // instructions are independent transcriptions of them.
        let process_entry_fields = c_struct_fields(&header, "wiz_entry_block");
        assert_eq!(
            c_layout(&process_entry_fields),
            entry_layout,
            "the C process-entry block differs from the kernel's field offsets"
        );
        assert!(
            c_widths(&process_entry_fields)
                .values()
                .all(|width| *width == 8),
            "every process-entry field is one u64 word"
        );
        assert_eq!(
            c_size(&process_entry_fields) as u64,
            entry_bytes,
            "the C process-entry block differs from the kernel's byte extent"
        );
        assert_eq!(
            rust_expression(&user, "const ENTRY_STACK_BYTES: u64"),
            "(ENTRY_BYTES as u64 + 15) & !15",
            "the initial stack pointer is not aligned beyond the entry block"
        );
        let compact_cross = compact_cpp(&rust_function_body(&user, "fn cross() {"));
        assert!(
            compact_cross.contains("lettop=launch.stack+USER_STACK_BYTES-ENTRY_STACK_BYTES;"),
            "the kernel does not place the entry block at the initial stack pointer"
        );
        assert_eq!(
            compact_cross
                .matches("usercopy::copy_to_user(top+entry::")
                .count(),
            3,
            "the kernel does not write exactly three process-entry words"
        );
        for write in [
            "usercopy::copy_to_user(top+entry::VERSIONasu64,&ENTRY_VERSION.to_le_bytes()",
            "usercopy::copy_to_user(top+entry::ARGUMENTasu64,&launch.argument.to_le_bytes()",
            concat!(
                "usercopy::copy_to_user(top+entry::BOOTSTRAP_HANDLEasu64,",
                "&u64::from(launch.bootstrap_handle).to_le_bytes()"
            ),
        ] {
            assert!(
                compact_cross.contains(write),
                "the kernel does not write one u64 entry word as `{write}`"
            );
        }
        assert!(
            compact_cross.contains("task::enter_user(launch.entry,top)"),
            "the kernel enters ring 3 at a pointer other than the checked entry block"
        );

        let ring_header_fields = c_struct_fields(&header, "wiz_ring_header");
        let submission_fields = c_struct_fields(&header, "wiz_submission");
        let completion_fields = c_struct_fields(&header, "wiz_completion");
        let ring_header_layout = rust_module_offsets(&queue, "header");
        let mut c_ring_header_layout = c_layout(&ring_header_fields);
        assert_eq!(
            c_ring_header_layout.remove("RESERVED0"),
            Some(12),
            "the C ring header's reserved word moved"
        );
        assert_eq!(
            c_ring_header_layout, ring_header_layout,
            "the C ring header differs from the kernel's field offsets"
        );
        let mut c_ring_header_widths = c_widths(&ring_header_fields);
        assert_eq!(
            c_ring_header_widths.remove("RESERVED0"),
            Some(4),
            "the C ring header's reserved word changed width"
        );
        assert_eq!(
            c_ring_header_widths,
            rust_field_widths(&queue, "header"),
            "the C ring header differs from the kernel's field widths"
        );
        assert_eq!(c_size(&ring_header_fields), 32, "C ring header size");
        let submission_layout = rust_module_offsets(&queue, "submission");
        let completion_layout = rust_module_offsets(&queue, "completion");
        assert_eq!(
            c_layout(&submission_fields),
            submission_layout,
            "the C submission differs from the kernel's field offsets"
        );
        assert_eq!(
            c_widths(&submission_fields),
            rust_field_widths(&queue, "submission"),
            "the C submission differs from the kernel's field widths"
        );
        assert_eq!(
            c_layout(&completion_fields),
            completion_layout,
            "the C completion differs from the kernel's field offsets"
        );
        assert_eq!(
            c_widths(&completion_fields),
            rust_field_widths(&queue, "completion"),
            "the C completion differs from the kernel's field widths"
        );
        let submission_bytes = u32::try_from(rust_uint(&queue, "const SUBMISSION_BYTES: usize"))
            .expect("SUBMISSION_BYTES fits in u32");
        let completion_bytes = u32::try_from(rust_uint(&queue, "const COMPLETION_BYTES: usize"))
            .expect("COMPLETION_BYTES fits in u32");
        assert_eq!(c_size(&submission_fields) as u32, submission_bytes);
        assert_eq!(c_size(&completion_fields) as u32, completion_bytes);
        assert_eq!(submission_bytes, 32, "submission wire size");
        assert_eq!(completion_bytes, 16, "completion wire size");
        assert!(
            queue.contains(
                "const COMPLETIONS_AT: usize = ENTRIES_AT + SUBMISSIONS as usize * SUBMISSION_BYTES;"
            ),
            "the kernel's completion base is not derived from its submission layout"
        );

        let entries_at = u32::try_from(page_size).expect("the entry base fits in disp32");
        assert!(
            queue.contains("const ENTRIES_AT: usize = PAGE_SIZE as usize;"),
            "the kernel's entry base is not one page"
        );
        let submissions = u32::try_from(rust_uint(&queue, "const SUBMISSIONS: u32"))
            .expect("SUBMISSIONS fits in u32");
        assert_eq!(
            submissions, 8,
            "the emitted program assumes eight submissions"
        );

        let image = crate::image::program::build().expect("building the emitted program");
        let elf = elf64::Elf::parse(&image).expect("the emitted program is ELF64");
        let mut segments = elf.segments();
        let segment =
            core::iter::Iterator::next(&mut segments).expect("the emitted program has a segment");
        let start = segment.offset() as usize;
        let end = start + segment.file_size() as usize;
        let code = &image[start..end];
        let operands = decode_program_operands(code);
        assert_eq!(
            operands.pops,
            ["rcx", "rax", "rsi"],
            "the emitted program does not consume version, argument and bootstrap handle in order"
        );
        let entry_version_immediate = sign_extended_imm8(entry_version)
            .expect("the entry version fits the sign-extended cmp rcx, imm8");
        let [(compared_version, rejected_at)] = operands.rcx_imm8_rejections.as_slice() else {
            panic!("the emitted program does not compare and reject one entry-block version");
        };
        assert_eq!(
            *compared_version, entry_version_immediate,
            "the emitted program accepts a different entry-block version"
        );
        assert_eq!(
            code.get(*rejected_at..*rejected_at + 2),
            Some(&[0x0F, 0x0B][..]),
            "the emitted program does not reject an unknown entry-block version with ud2"
        );
        assert_eq!(
            operands.ring_bases,
            [ring],
            "the emitted mov rbx, imm64 does not load the kernel ring base"
        );
        assert_eq!(
            operands.stack_immediates,
            [user_end],
            "the hostile emitted mov rsp, imm64 does not load the kernel user boundary"
        );
        let ring_enter = decoded_call_arms
            .iter()
            .find_map(|(number, variant)| (variant == "RingEnter").then_some(*number))
            .and_then(|number| u32::try_from(number).ok())
            .expect("Call::RingEnter fits in an emitted imm32");
        let ring_wait = decoded_call_arms
            .iter()
            .find_map(|(number, variant)| (variant == "RingWait").then_some(*number))
            .and_then(|number| u32::try_from(number).ok())
            .expect("Call::RingWait fits in an emitted imm32");
        assert_eq!(
            operands.eax_immediates,
            [0x0BAD_0BAD, 0x0BAD_0BAD, 0x0BAD_0BAD, ring_enter, ring_wait,],
            "the emitted mov eax immediates are not refusal, RingEnter and RingWait"
        );
        let syscall_immediate =
            u8::try_from(syscall_number).expect("the syscall vector fits in imm8");
        assert_eq!(
            operands.interrupts,
            vec![syscall_immediate; 7],
            "an emitted int imm8 uses a vector other than the kernel syscall gate"
        );
        let completion_at = entries_at + submissions * submission_bytes;
        let echo = decoded_opcode_arms
            .iter()
            .find_map(|(number, variant)| (variant == "Echo").then_some(*number))
            .and_then(|number| u32::try_from(number).ok())
            .expect("Opcode::Echo fits in an emitted imm32");
        let duplicate = decoded_opcode_arms
            .iter()
            .find_map(|(number, variant)| (variant == "Duplicate").then_some(*number))
            .and_then(|number| u32::try_from(number).ok())
            .expect("Opcode::Duplicate fits in an emitted imm32");
        let timer = decoded_opcode_arms
            .iter()
            .find_map(|(number, variant)| (variant == "Timer").then_some(*number))
            .and_then(|number| u32::try_from(number).ok())
            .expect("Opcode::Timer fits in an emitted imm32");
        assert_eq!(
            operands.writes32,
            [
                (entries_at + submission_layout["TOKEN"] as u32, 0x1234),
                (entries_at + submission_layout["OPCODE"] as u32, echo),
                (entries_at + submission_layout["OPCODE"] as u32, duplicate,),
                (entries_at + submission_layout["OPCODE"] as u32, timer),
            ],
            "the emitted program's submission imm32 writes differ from the kernel layout"
        );
        assert_eq!(
            operands.writes32_from_esi,
            [entries_at + submission_layout["HANDLE"] as u32],
            "the emitted program does not submit the bootstrap handle from its entry block"
        );
        assert_eq!(
            operands.r8d_zeroes, 1,
            "the emitted program does not default to ordinary ring service"
        );
        assert_eq!(
            operands.r8d_immediates,
            [1],
            "the measured behaviour does not arm its timestamp path"
        );
        assert_eq!(
            operands.r8d_zero_tests, 2,
            "the measurement selector does not guard both timestamp reads"
        );
        assert_eq!(
            operands.fenced_timestamp_reads, 2,
            "the real RingEnter is not bracketed by two fenced timestamp reads"
        );
        assert_eq!(
            operands.r10d_timestamp_captures, 1,
            "the first timestamp is not retained across RingEnter"
        );
        assert_eq!(
            operands.timestamp_subtractions, 1,
            "the second timestamp is not reduced to the round-trip delta"
        );
        assert_eq!(
            operands.writes32_from_eax,
            [entries_at + submission_layout["OFFSET"] as u32],
            "the measured delta does not use the consumed submission's offset field"
        );
        let duplicate_rights = u32::try_from(declared_macros["WIZ_RIGHT_INSPECT"])
            .expect("INSPECT fits in the emitted imm32");
        assert_eq!(
            operands.writes64_from_imm32,
            [
                (
                    entries_at + submission_layout["ARGUMENT"] as u32,
                    0x00C0_FFEE,
                ),
                (
                    entries_at + submission_layout["ARGUMENT"] as u32,
                    duplicate_rights,
                ),
                (entries_at + submission_layout["ARGUMENT"] as u32, 2_000_000,),
            ],
            "the emitted program's submission imm64 write differs from the kernel layout"
        );
        assert_eq!(
            operands.short_writes32,
            [
                (ring_header_layout["SUBMISSION_TAIL"] as u8, 1),
                (ring_header_layout["COMPLETION_HEAD"] as u8, 1),
            ],
            "the emitted program publishes through different header fields"
        );
        assert_eq!(
            operands.short_reads32,
            [ring_header_layout["COMPLETION_TAIL"] as u8],
            "the emitted program acquires a different header field"
        );
        assert_eq!(
            operands.eax_imm8_comparisons,
            [1],
            "the emitted program does not require one published completion"
        );
        assert_eq!(
            operands.reads32,
            [completion_at + completion_layout["STATUS"] as u32],
            "the emitted program reads completion.status at a different offset"
        );
        assert_eq!(
            operands.reads64,
            [completion_at + completion_layout["VALUE"] as u32],
            "the emitted program reads completion.value at a different offset"
        );
        assert_eq!(
            operands.ecx_zero_tests, 1,
            "the emitted program does not require a successful completion status"
        );
        assert_eq!(
            operands.rcx_zero_tests, 2,
            "the emitted program does not reject zero duplicate and Timer answers"
        );
        assert_eq!(
            operands.ecx_esi_comparisons, 1,
            "the emitted program does not distinguish the duplicate from its source handle"
        );

        let timepage = &timepage_source;

        // Compare complete layouts: address drift can unmap the page, and field
        // drift changes the wire ABI (D-166).
        let constants_fields = c_struct_fields(&header, "wiz_time_constants");
        let clock_fields = c_struct_fields(&header, "wiz_time_clock");
        assert_eq!(
            c_layout(&constants_fields),
            rust_module_offsets(timepage, "constants"),
            "the C constants struct differs from the kernel's offsets"
        );
        assert_eq!(
            c_widths(&constants_fields),
            rust_field_widths(timepage, "constants"),
            "the C constants struct differs from the kernel's field widths"
        );
        assert_eq!(c_size(&constants_fields), 16, "C constants struct size");
        assert_eq!(
            c_layout(&clock_fields),
            rust_module_offsets(timepage, "clock"),
            "the C clock struct differs from the kernel's offsets"
        );
        assert_eq!(
            c_widths(&clock_fields),
            rust_field_widths(timepage, "clock"),
            "the C clock struct differs from the kernel's field widths"
        );
        assert_eq!(c_size(&clock_fields), 48, "C clock struct size");

        let checked_declared_calls = c_enum_values(&active_header, "wiz_call");
        let checked_expected_calls: std::collections::BTreeMap<_, _> = decoded_call_arms
            .iter()
            .map(|(number, variant)| {
                let name = match variant.as_str() {
                    "Exit" => "WIZ_CALL_EXIT",
                    "RingEnter" => "WIZ_CALL_RING_ENTER",
                    "ThreadBase" => "WIZ_CALL_TCB_SET",
                    "RingWait" => "WIZ_CALL_RING_WAIT",
                    other => panic!("no C spelling registered for Call::{other}"),
                };
                (name.to_string(), *number)
            })
            .collect();
        assert_eq!(
            checked_declared_calls, checked_expected_calls,
            "the exact C call-number domain differs from Call::decode"
        );

        // The boundary has one definition; usercopy only re-exports it.
        let usercopy = rust_without_comments(
            &std::fs::read_to_string(root.join("wost/src/arch/x86_64/usercopy.rs"))
                .expect("usercopy.rs"),
        );
        assert!(
            usercopy.contains("pub use super::paging::USER_END;"),
            "usercopy states the boundary again instead of re-exporting it"
        );
    }

    #[test]
    fn the_expected_count_is_the_number_of_checks_registered() {
        let selftest = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("xtask sits below the workspace root")
                .join("kernel")
                .join("src")
                .join("selftest.rs"),
        )
        .expect("selftest.rs");

        let registered = selftest.matches("report.check(").count();
        assert_eq!(
            registered, EXPECTED_SELFTEST_CHECKS,
            "the kernel registers {registered} checks and declares {EXPECTED_SELFTEST_CHECKS}"
        );
    }
}
