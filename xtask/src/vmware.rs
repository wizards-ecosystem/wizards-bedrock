//! Generating a VMware VM directory: a flat VMDK plus a VMX.
//!
//! The raw image is copied in alongside the descriptors so the folder is
//! self-contained and can be moved or archived on its own.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::paths::{self, Paths};

const VMDK_TEMPLATE: &str = include_str!("../templates/wizard-os.vmdk.in");
const VMX_TEMPLATE: &str = include_str!("../templates/wizard-os.vmx.in");

const FLAT_NAME: &str = "wizard-os-flat.img";
const VMDK_NAME: &str = "wizard-os.vmdk";
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

/// Write a VMware VM directory beside the repository, wrapping `image`.
pub fn generate(paths: &Paths, image: &Path) -> Result<()> {
    let vm_dir = paths.vm_dir();
    std::fs::create_dir_all(&vm_dir).with_context(|| format!("creating {}", vm_dir.display()))?;

    let flat = vm_dir.join(FLAT_NAME);
    std::fs::copy(image, &flat)
        .with_context(|| format!("copying {} to {}", image.display(), flat.display()))?;

    let bytes = std::fs::metadata(&flat)
        .with_context(|| format!("measuring {}", flat.display()))?
        .len();
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

    let vmx = render(
        VMX_TEMPLATE,
        &[
            ("DISPLAY_NAME", DISPLAY_NAME.to_string()),
            ("VMDK_NAME", VMDK_NAME.to_string()),
            ("SERIAL_LOG", SERIAL_LOG.to_string()),
        ],
    )?;
    let vmx_path = vm_dir.join(VMX_NAME);
    write(&vmx_path, &vmx)?;

    println!("[xtask] VMware VM ready: {}", vmx_path.display());
    println!("[xtask] open it in VMware Workstation, or run:");
    println!("        vmrun start \"{}\"", vmx_path.display());
    println!(
        "[xtask] serial output will appear in {}",
        vm_dir.join(SERIAL_LOG).display()
    );
    Ok(())
}

fn write(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE_SECTORS: u64 = 64 * 1024 * 1024 / 512;

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
                ("SERIAL_LOG", SERIAL_LOG.to_string()),
            ],
        )
        .unwrap();

        assert!(vmx.contains(r#"firmware = "efi""#));
        assert!(vmx.contains(r#"serial0.fileName = "serial.log""#));
        assert!(vmx.contains(r#"sata0:0.fileName = "wizard-os.vmdk""#));
    }
}
