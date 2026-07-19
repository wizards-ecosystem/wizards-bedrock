//! Build orchestrator for The Wizard's OS.
//!
//! Everything is done in-process with pure Rust — no xorriso, mtools, or
//! other external imaging tools. The bootable artifact is a raw disk image
//! with an MBR partition table holding a single FAT32 EFI System Partition:
//!
//!   cargo xtask build    — compile the kernel
//!   cargo xtask image    — build target/wizard-os.img (bootable UEFI disk)
//!   cargo xtask run      — image + boot it in QEMU (serial -> stdio)
//!   cargo xtask vmware   — image + generate vm/ (VMDK + VMX) for VMware

use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use fscommon::StreamSlice;

const DISK_MIB: u64 = 64;
const SECTOR: u64 = 512;
/// Standard 1 MiB-aligned first partition.
const PART_START_LBA: u64 = 2048;

fn main() -> Result<()> {
    let task = std::env::args().nth(1).unwrap_or_else(|| "run".into());
    let root = workspace_root();
    match task.as_str() {
        "build" => {
            build_kernel(&root)?;
        }
        "image" => {
            build_image(&root)?;
        }
        "run" => {
            let img = build_image(&root)?;
            run_qemu(&root, &img)?;
        }
        "vmware" => {
            let img = build_image(&root)?;
            make_vmware(&root, &img)?;
        }
        other => bail!("unknown task `{other}` (expected build|image|run|vmware)"),
    }
    Ok(())
}

fn workspace_root() -> PathBuf {
    // xtask lives at <root>/xtask.
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn build_kernel(root: &Path) -> Result<PathBuf> {
    println!("[xtask] building kernel");
    let status = Command::new(env!("CARGO"))
        .current_dir(root)
        .args([
            "build",
            "--package",
            "wizard-kernel",
            "--target",
            "x86_64-unknown-none",
            "--release",
        ])
        .status()
        .context("failed to spawn cargo")?;
    if !status.success() {
        bail!("kernel build failed");
    }
    Ok(root.join("target/x86_64-unknown-none/release/wizard-kernel"))
}

fn build_image(root: &Path) -> Result<PathBuf> {
    let kernel = build_kernel(root)?;
    let img_path = root.join("target/wizard-os.img");
    println!("[xtask] creating disk image {}", img_path.display());

    let mut disk = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&img_path)?;
    disk.set_len(DISK_MIB * 1024 * 1024)?;

    let total_sectors = DISK_MIB * 1024 * 1024 / SECTOR;
    write_mbr(&mut disk, total_sectors)?;

    // Format the partition as FAT32 and populate the EFI system tree.
    let part_bytes = (total_sectors - PART_START_LBA) * SECTOR;
    let part = StreamSlice::new(&mut disk, PART_START_LBA * SECTOR, PART_START_LBA * SECTOR + part_bytes)?;
    let mut part = fscommon::BufStream::new(part);
    fatfs::format_volume(
        &mut part,
        fatfs::FormatVolumeOptions::new()
            .fat_type(fatfs::FatType::Fat32)
            .volume_label(*b"WIZARD-ESP "),
    )?;
    let fs = fatfs::FileSystem::new(part, fatfs::FsOptions::new())?;
    {
        let fs_root = fs.root_dir();
        let efi_boot = fs_root.create_dir("EFI")?.create_dir("BOOT")?;
        copy_into(&efi_boot, "BOOTX64.EFI", &root.join("boot/limine/BOOTX64.EFI"))?;

        let boot = fs_root.create_dir("boot")?;
        copy_into(&boot, "wizard-kernel.elf", &kernel)?;
        let limine_dir = boot.create_dir("limine")?;
        copy_into(&limine_dir, "limine.conf", &root.join("boot/limine.conf"))?;
    }
    fs.unmount()?;

    println!("[xtask] image ready");
    Ok(img_path)
}

/// Write a protective-style MBR with one bootable partition of type 0xEF
/// (EFI System). UEFI firmware boots this layout directly; GPT can come
/// later when the OS grows its own partitioning story.
fn write_mbr(disk: &mut File, total_sectors: u64) -> Result<()> {
    let mut mbr = [0u8; 512];
    let entry = &mut mbr[446..462];
    entry[0] = 0x80; // bootable flag (ignored by UEFI, kept for BIOS tools)
    entry[4] = 0xEF; // partition type: EFI System
    // CHS fields left as zero markers; everything modern uses LBA.
    entry[8..12].copy_from_slice(&(PART_START_LBA as u32).to_le_bytes());
    entry[12..16].copy_from_slice(&((total_sectors - PART_START_LBA) as u32).to_le_bytes());
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    disk.seek(SeekFrom::Start(0))?;
    disk.write_all(&mbr)?;
    Ok(())
}

fn copy_into<T: fatfs::ReadWriteSeek>(
    dir: &fatfs::Dir<'_, T>,
    name: &str,
    src: &Path,
) -> Result<()> {
    let data = fs::read(src).with_context(|| format!("reading {}", src.display()))?;
    let mut file = dir.create_file(name)?;
    file.truncate()?;
    file.write_all(&data)?;
    Ok(())
}

fn run_qemu(root: &Path, img: &Path) -> Result<()> {
    let qemu_dir = find_qemu()?;
    let code_fd = qemu_dir.join("share/edk2-x86_64-code.fd");
    if !code_fd.exists() {
        bail!("UEFI firmware not found at {}", code_fd.display());
    }
    // The vars flash must be writable; keep a per-build copy in target/.
    let vars_src = qemu_dir.join("share/edk2-i386-vars.fd");
    let vars_fd = root.join("target/efi-vars.fd");
    fs::copy(&vars_src, &vars_fd).context("copying UEFI vars template")?;

    println!("[xtask] launching QEMU (serial on stdio, close window to exit)");
    let status = Command::new(qemu_dir.join("qemu-system-x86_64.exe"))
        .args(["-machine", "q35", "-cpu", "qemu64", "-m", "512M"])
        .args(["-serial", "stdio"])
        .arg("-drive")
        .arg(format!("if=pflash,format=raw,readonly=on,file={}", code_fd.display()))
        .arg("-drive")
        .arg(format!("if=pflash,format=raw,file={}", vars_fd.display()))
        .arg("-drive")
        .arg(format!("format=raw,file={}", img.display()))
        .status()
        .context("failed to launch qemu-system-x86_64")?;
    if !status.success() {
        bail!("QEMU exited with {status}");
    }
    Ok(())
}

fn find_qemu() -> Result<PathBuf> {
    let default = PathBuf::from(r"C:\Program Files\qemu");
    if default.join("qemu-system-x86_64.exe").exists() {
        return Ok(default);
    }
    // Fall back to PATH.
    if let Ok(path) = which("qemu-system-x86_64.exe") {
        return Ok(path.parent().unwrap().to_path_buf());
    }
    bail!("QEMU not found (looked in C:\\Program Files\\qemu and PATH)");
}

fn which(exe: &str) -> Result<PathBuf> {
    let paths = std::env::var_os("PATH").context("no PATH")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(exe);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!("{exe} not on PATH")
}

/// Generate a VMware VM directory: flat VMDK descriptor + VMX, with the raw
/// image copied alongside so the VM folder is self-contained.
fn make_vmware(root: &Path, img: &Path) -> Result<()> {
    let vm_dir = root.join("vm");
    fs::create_dir_all(&vm_dir)?;

    let flat = vm_dir.join("wizard-os-flat.img");
    fs::copy(img, &flat)?;
    let sectors = fs::metadata(&flat)?.len() / SECTOR;
    let cylinders = sectors / (255 * 63);

    let vmdk = format!(
        "# Disk DescriptorFile\n\
         version=1\n\
         encoding=\"windows-1252\"\n\
         CID=fffffffe\n\
         parentCID=ffffffff\n\
         createType=\"monolithicFlat\"\n\
         \n\
         # Extent description\n\
         RW {sectors} FLAT \"wizard-os-flat.img\" 0\n\
         \n\
         # The Disk Data Base\n\
         #DDB\n\
         ddb.adapterType = \"lsilogic\"\n\
         ddb.geometry.cylinders = \"{cylinders}\"\n\
         ddb.geometry.heads = \"255\"\n\
         ddb.geometry.sectors = \"63\"\n\
         ddb.virtualHWVersion = \"19\"\n"
    );
    fs::write(vm_dir.join("wizard-os.vmdk"), vmdk)?;

    let vmx = "\
.encoding = \"windows-1252\"\n\
config.version = \"8\"\n\
virtualHW.version = \"19\"\n\
displayName = \"The Wizard's OS\"\n\
guestOS = \"other-64\"\n\
firmware = \"efi\"\n\
memsize = \"512\"\n\
numvcpus = \"2\"\n\
sata0.present = \"TRUE\"\n\
sata0:0.present = \"TRUE\"\n\
sata0:0.fileName = \"wizard-os.vmdk\"\n\
serial0.present = \"TRUE\"\n\
serial0.fileType = \"file\"\n\
serial0.fileName = \"serial.log\"\n\
usb.present = \"TRUE\"\n\
svga.autodetect = \"TRUE\"\n";
    fs::write(vm_dir.join("wizard-os.vmx"), vmx)?;

    println!("[xtask] VMware VM ready: {}", vm_dir.join("wizard-os.vmx").display());
    println!("[xtask] open it in VMware Workstation, or run:");
    println!("        vmrun start \"{}\"", vm_dir.join("wizard-os.vmx").display());
    println!("[xtask] serial output will appear in vm/serial.log");
    Ok(())
}
