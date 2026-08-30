//! Generating a VMware VM directory: a flat VMDK plus a VMX.
//!
//! The raw image is copied in alongside the descriptors so the folder is
//! self-contained and can be moved or archived on its own.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::image::Kernel;
use crate::paths::{self, Paths};
use crate::qualification;
use crate::sha256;

const VMDK_TEMPLATE: &str = include_str!("../templates/wizard-os.vmdk.in");
const VMX_TEMPLATE: &str = include_str!("../templates/wizard-os.vmx.in");

const FLAT_NAME: &str = "wizard-os-flat.img";
const VMDK_NAME: &str = "wizard-os.vmdk";

/// The NVMe namespace's descriptor, and the flat file behind it.
const NVME_VMDK_NAME: &str = "wizard-nvm.vmdk";
/// See [`NVME_VMDK_NAME`].
const NVME_FLAT_NAME: &str = "wizard-nvm-flat.img";
const VMX_NAME: &str = "wizard-os.vmx";
const SERIAL_LOG: &str = "serial.log";
const DISPLAY_NAME: &str = "The Wizard's OS";

/// Heads per cylinder in the geometry VMware expects.
const HEADS: u64 = 255;

/// Sectors per track in the same.
const SECTORS_PER_TRACK: u64 = 63;

/// A disk geometry for a VMDK descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// Cylinder count.
    pub cylinders: u64,
    /// Head count.
    pub heads: u64,
    /// Sectors per track.
    pub sectors_per_track: u64,
}

/// Geometry covering `sectors`.
///
/// The cylinder count rounds up: truncating would declare a geometry smaller
/// than the extent it describes.
pub fn geometry(sectors: u64) -> Geometry {
    Geometry {
        cylinders: sectors.div_ceil(HEADS * SECTORS_PER_TRACK),
        heads: HEADS,
        sectors_per_track: SECTORS_PER_TRACK,
    }
}

/// Substitute `{{KEY}}` placeholders in `template`.
///
/// Substituted text is never rescanned, so a value that itself contains a
/// placeholder is emitted literally. An unrecognised key is an error rather
/// than a silently empty field.
pub fn render(template: &str, values: &[(&str, String)]) -> Result<String> {
    const OPEN: &str = "{{";
    const CLOSE: &str = "}}";

    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else {
            bail!("unterminated placeholder in template");
        };
        let key = &after[..end];
        let value = values
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value)
            .with_context(|| format!("template names an unknown placeholder: {key}"))?;
        out.push_str(value);
        rest = &after[end + CLOSE.len()..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Write an isolated VMware VM directory beside the repository, wrapping `image`.
pub fn generate(paths: &Paths, image: &Path, kernel: Kernel) -> Result<()> {
    let vmrun = qualification::vmrun(paths.root())?;
    let vm_dir = vm_directory(paths, kernel);
    std::fs::create_dir_all(&vm_dir).with_context(|| format!("creating {}", vm_dir.display()))?;

    let flat = vm_dir.join(FLAT_NAME);
    std::fs::copy(image, &flat)
        .with_context(|| format!("copying {} to {}", image.display(), flat.display()))?;

    let bytes = std::fs::metadata(&flat)
        .with_context(|| format!("measuring {}", flat.display()))?
        .len();
    let digest = sha256::hex(&sha256::digest(
        &std::fs::read(&flat).with_context(|| format!("reading {}", flat.display()))?,
    ));
    let sectors = bytes / paths::SECTOR_SIZE;
    let geometry = geometry(sectors);

    let vmdk = render(
        VMDK_TEMPLATE,
        &[
            ("SECTORS", sectors.to_string()),
            ("FLAT_NAME", FLAT_NAME.to_string()),
            ("CYLINDERS", geometry.cylinders.to_string()),
            ("HEADS", geometry.heads.to_string()),
            ("SECTORS_PER_TRACK", geometry.sectors_per_track.to_string()),
        ],
    )?;
    write(&vm_dir.join(VMDK_NAME), &vmdk)?;

    // The namespace Phase 6 reads, built by the same function QEMU's runner
    // uses so both machines see the same bytes (D-176).
    let namespace = crate::qemu::nvme_image(paths.root())?;
    let nvme_flat = vm_dir.join(NVME_FLAT_NAME);
    std::fs::copy(&namespace, &nvme_flat)
        .with_context(|| format!("copying {} to {}", namespace.display(), nvme_flat.display()))?;
    let nvme_bytes = std::fs::metadata(&nvme_flat)
        .with_context(|| format!("measuring {}", nvme_flat.display()))?
        .len();
    let nvme_sectors = nvme_bytes / paths::SECTOR_SIZE;
    let nvme_geometry = crate::vmware::geometry(nvme_sectors);
    let nvme_vmdk = render(
        VMDK_TEMPLATE,
        &[
            ("SECTORS", nvme_sectors.to_string()),
            ("FLAT_NAME", NVME_FLAT_NAME.to_string()),
            ("CYLINDERS", nvme_geometry.cylinders.to_string()),
            ("HEADS", nvme_geometry.heads.to_string()),
            (
                "SECTORS_PER_TRACK",
                nvme_geometry.sectors_per_track.to_string(),
            ),
        ],
    )?;
    write(&vm_dir.join(NVME_VMDK_NAME), &nvme_vmdk)?;

    let vmx = render(
        VMX_TEMPLATE,
        &[
            ("DISPLAY_NAME", display_name(kernel)),
            ("VMDK_NAME", VMDK_NAME.to_string()),
            ("NVME_VMDK_NAME", NVME_VMDK_NAME.to_string()),
            ("SERIAL_LOG", SERIAL_LOG.to_string()),
        ],
    )?;
    let vmx_path = vm_dir.join(VMX_NAME);
    write(&vmx_path, &vmx)?;
    let serial_log = vm_dir.join(SERIAL_LOG);
    reset_serial_log(&serial_log)?;

    println!("[xtask] VMware VM ready: {}", vmx_path.display());
    println!(
        "[xtask] VMware {} image: {bytes} bytes, sha256 {digest}",
        image_label(kernel)
    );
    println!("[xtask] open it in VMware Workstation");
    if let Some(vmrun) = vmrun {
        println!("[xtask] VMware identity: {}", vmrun.version);
        println!("[xtask] or run:");
        println!(
            "        \"{}\" -T ws start \"{}\"",
            vmrun.path.display(),
            vmx_path.display()
        );
    } else {
        println!(
            "[xtask] vmrun unavailable: install VMware Workstation or open the VMX on another recorded host"
        );
    }
    println!(
        "[xtask] serial output will appear in {}",
        serial_log.display()
    );
    match kernel {
        Kernel::SelfTest => {
            println!("[xtask] this is the self-test kernel — four things to do:");
            println!("        1. press `c` at `selftest: awaiting storage-delay keystroke`");
            println!(
                "        2. press `a`, Backspace, Enter at `selftest: awaiting injected keystroke`"
            );
            println!("        3. press `b` at `selftest: awaiting translated keystroke`");
            println!("        4. look for `selftest: verdict` in the fresh serial log");
        }
        Kernel::Shipped => {
            println!("[xtask] this is the shipped kernel — verify its fresh serial log:");
            println!("        1. storage reaches `nvme: up` and `fat32: mounted`");
            println!("        2. tmpfs and devfs complete their namespace survey");
            println!("        3. ring 3 launches and `kernel: reading the keyboard` settles");
            println!("        4. no `KERNEL PANIC` appears");
        }
    }
    Ok(())
}

fn vm_directory(paths: &Paths, kernel: Kernel) -> PathBuf {
    match kernel {
        Kernel::SelfTest => paths.vm_dir(),
        Kernel::Shipped => paths.vm_dir().join("shipped"),
    }
}

fn image_label(kernel: Kernel) -> &'static str {
    match kernel {
        Kernel::SelfTest => "self-test",
        Kernel::Shipped => "shipped",
    }
}

fn display_name(kernel: Kernel) -> String {
    match kernel {
        Kernel::SelfTest => DISPLAY_NAME.to_string(),
        Kernel::Shipped => format!("{DISPLAY_NAME} (shipped)"),
    }
}

fn reset_serial_log(path: &Path) -> Result<()> {
    std::fs::write(path, []).with_context(|| format!("resetting {}", path.display()))
}

fn write(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE_SECTORS: u64 = 64 * 1024 * 1024 / 512;

    #[test]
    fn selftest_and_shipped_vms_do_not_share_machine_state() {
        let paths = Paths::new(PathBuf::from("/repo"), None);
        assert_eq!(vm_directory(&paths, Kernel::SelfTest), paths.vm_dir());
        assert_eq!(
            vm_directory(&paths, Kernel::Shipped),
            paths.vm_dir().join("shipped")
        );
    }

    #[test]
    fn a_generated_vm_starts_with_a_fresh_serial_transcript() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("host clock")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("wizard-vmware-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).expect("VMware fixture directory");
        let log = root.join(SERIAL_LOG);
        std::fs::write(&log, "stale verdict").expect("stale transcript");

        reset_serial_log(&log).expect("fresh transcript");

        assert_eq!(std::fs::read(&log).unwrap(), b"");
        std::fs::remove_dir_all(root).expect("remove VMware fixture");
    }

    #[test]
    fn geometry_rounds_cylinders_up() {
        // The regression: truncating division gave 8, describing 1.25 MiB less
        // than the extent actually holds.
        assert_eq!(geometry(IMAGE_SECTORS).cylinders, 9);
        assert_eq!(geometry(1).cylinders, 1);
        assert_eq!(geometry(HEADS * SECTORS_PER_TRACK).cylinders, 1);
        assert_eq!(geometry(HEADS * SECTORS_PER_TRACK + 1).cylinders, 2);
    }

    #[test]
    fn geometry_always_covers_the_extent() {
        for sectors in [1, 63, 16_065, 16_066, 131_072, 2_097_152] {
            let g = geometry(sectors);
            let covered = g.cylinders * g.heads * g.sectors_per_track;
            assert!(covered >= sectors, "{sectors} sectors not covered by {g:?}");
            let previous = (g.cylinders - 1) * g.heads * g.sectors_per_track;
            assert!(previous < sectors, "{g:?} is larger than it needs to be");
        }
    }

    #[test]
    fn render_substitutes_every_placeholder() {
        let vmdk = render(
            VMDK_TEMPLATE,
            &[
                ("SECTORS", "131072".to_string()),
                ("FLAT_NAME", FLAT_NAME.to_string()),
                ("CYLINDERS", "9".to_string()),
                ("HEADS", "255".to_string()),
                ("SECTORS_PER_TRACK", "63".to_string()),
            ],
        )
        .unwrap();

        assert!(!vmdk.contains("{{"));
        assert!(vmdk.contains(r#"RW 131072 FLAT "wizard-os-flat.img" 0"#));
        assert!(vmdk.contains(r#"ddb.geometry.cylinders = "9""#));
    }

    #[test]
    fn render_rejects_a_placeholder_it_was_not_given() {
        let error = render("hello {{NAME}}", &[]).unwrap_err().to_string();
        assert!(error.contains("NAME"), "{error}");
    }

    #[test]
    fn render_does_not_rescan_substituted_text() {
        let out = render(
            "{{A}}{{B}}",
            &[("A", "{{B}}".to_string()), ("B", "value".to_string())],
        )
        .unwrap();
        assert_eq!(out, "{{B}}value");
    }

    #[test]
    fn the_vmx_boots_efi_and_logs_serial_to_a_file() {
        let vmx = render(
            VMX_TEMPLATE,
            &[
                ("DISPLAY_NAME", DISPLAY_NAME.to_string()),
                ("VMDK_NAME", VMDK_NAME.to_string()),
                ("NVME_VMDK_NAME", NVME_VMDK_NAME.to_string()),
                ("SERIAL_LOG", SERIAL_LOG.to_string()),
            ],
        )
        .unwrap();

        assert!(vmx.contains(r#"firmware = "efi""#));
        assert!(vmx.contains(r#"serial0.fileName = "serial.log""#));
        assert!(vmx.contains(r#"sata0:0.fileName = "wizard-os.vmdk""#));
        // The storage controller Phase 6 drives. A VM without one runs the
        // checks and fails ten of them, which is what the first VMware boot to
        // run any checks discovered (D-176).
        //
        // Two, and the first with no namespace attached: the discovery check
        // asks whether a useless candidate is retired in favour of its
        // sibling, and a machine with one controller cannot answer however the
        // human at it presses (D-240).
        assert!(vmx.contains(r#"nvme0.present = "TRUE""#));
        assert!(
            !vmx.contains(r#"nvme0:0.present"#),
            "the first controller must have no namespace to retire"
        );
        assert!(vmx.contains(r#"nvme1.present = "TRUE""#));
        assert!(vmx.contains(r#"nvme1:0.fileName = "wizard-nvm.vmdk""#));
        // before the guest gets a chance to look for it (D-176).
        assert!(vmx.contains(r#"pciBridge4.virtualDev = "pcieRootPort""#));
    }
}
