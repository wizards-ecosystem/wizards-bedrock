//! Generating a VMware VM directory: a flat VMDK plus a VMX.
//!
//! The raw image is copied in alongside the descriptors so the folder is
//! self-contained and can be moved or archived on its own.

mod config;

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::image::Kernel;
use crate::paths::{self, Paths};
use crate::qemu::canonical_decimal;
use crate::qualification;
use crate::sha256;

const VMDK_TEMPLATE: &str = include_str!("../templates/bedrock.vmdk.in");
const VMX_TEMPLATE: &str = include_str!("../templates/bedrock.vmx.in");

const FLAT_NAME: &str = "bedrock-flat.img";
const VMDK_NAME: &str = "bedrock.vmdk";

/// The NVMe namespace's descriptor, and the flat file behind it.
const NVME_VMDK_NAME: &str = "wizard-nvm.vmdk";
/// See [`NVME_VMDK_NAME`].
const NVME_FLAT_NAME: &str = "wizard-nvm-flat.img";
const VMX_NAME: &str = "bedrock.vmx";
const DISPLAY_NAME: &str = "The Wizard's Bedrock";
const VMWARE_ISOLATION_PREFIX: &str = "cpu: user isolation";
const VMWARE_PAIRED_ISOLATION_MARKER: &str =
    "cpu: user isolation paired roots: selected; shared mappings refused";
const VMWARE_SELFTEST_SHARED_ISOLATION_MARKER: &str =
    "cpu: user isolation paired roots: forced for self-test; shared proof retained: rdcl_no";
const VMWARE_SHIPPED_SHARED_ISOLATION_MARKER: &str =
    "cpu: user isolation shared kernel mappings: rdcl_no";
const SELFTEST_ISOLATION_MARKERS: &[&str] = &[
    VMWARE_PAIRED_ISOLATION_MARKER,
    VMWARE_SELFTEST_SHARED_ISOLATION_MARKER,
];
const SHIPPED_ISOLATION_MARKERS: &[&str] = &[
    VMWARE_PAIRED_ISOLATION_MARKER,
    VMWARE_SHIPPED_SHARED_ISOLATION_MARKER,
];
const VMWARE_DMA_HANDOFF_MARKER: &str =
    "iommu: entry DMA fence retained VMware 440BX 00:00.0 as the qualified non-requester";
const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;

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
    let vm_dir = vm_directory(paths, kernel);
    let receipt = vm_dir.join(qualification::VMWARE_GENERATION_RECEIPT);
    qualification::invalidate_vmware(paths, kernel)?;
    invalidate_generation(paths, kernel)?;
    let vmrun = qualification::vmrun(paths.root())?;
    reset_vm_directory(paths, kernel)?;

    let flat = vm_dir.join(FLAT_NAME);
    let bytes = copy_new(image, &flat)?;
    let (digested_bytes, digest) = sha256::digest_reader(
        File::open(&flat).with_context(|| format!("opening {}", flat.display()))?,
    )
    .with_context(|| format!("hashing {}", flat.display()))?;
    ensure!(
        digested_bytes == bytes,
        "{} changed while its digest was recorded",
        flat.display()
    );
    let digest = sha256::hex(&digest);
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
    write_new(&vm_dir.join(VMDK_NAME), vmdk.as_bytes())?;

    // Share the builder so both machine gates boot identical bytes (D-176).
    let namespace = crate::qemu::nvme_image(paths.root())?;
    let nvme_flat = vm_dir.join(NVME_FLAT_NAME);
    let nvme_bytes = copy_new(&namespace, &nvme_flat)?;
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
    write_new(&vm_dir.join(NVME_VMDK_NAME), nvme_vmdk.as_bytes())?;

    let handoff = vm_dir.join(crate::platform_acpi::DMA_HANDOFF_FILE);
    write_new(&handoff, &crate::platform_acpi::dma_handoff_table())?;

    let serial_name = qualification::fresh_vmware_transcript_name()?;
    let vmx = generation_vmx(kernel, &serial_name)?;
    let vmx_path = vm_dir.join(VMX_NAME);
    write_new(&vmx_path, vmx.as_bytes())?;
    let serial_log = vm_dir.join(&serial_name);
    write_new(&serial_log, &[])?;
    qualification::commit_vmware_generation(
        paths,
        kernel,
        &qualification::VmwareArtifacts {
            image: &flat,
            namespace: &nvme_flat,
            vmdk: &vm_dir.join(VMDK_NAME),
            namespace_vmdk: &vm_dir.join(NVME_VMDK_NAME),
            vmx: &vmx_path,
            handoff: &handoff,
            transcript: &serial_log,
            receipt: &receipt,
        },
    )?;

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
            println!("[xtask] this is the self-test kernel, four things to do:");
            println!("        1. press `c` at `selftest: awaiting storage-delay keystroke`");
            println!(
                "        2. press `a`, Backspace, Enter at `selftest: awaiting injected keystroke`"
            );
            println!("        3. press `b` at `selftest: awaiting translated keystroke`");
            println!("        4. look for `selftest: verdict` in the fresh serial log");
        }
        Kernel::Shipped => {
            println!("[xtask] this is the shipped kernel, verify its fresh serial log:");
            println!("        1. storage reaches `nvme: up` and `fat32: mounted`");
            println!("        2. tmpfs and devfs complete their namespace survey");
            println!("        3. ring 3 launches and `kernel: reading the keyboard` settles");
            println!("        4. no `KERNEL PANIC` appears");
        }
    }
    Ok(())
}

pub(crate) fn invalidate_generation(paths: &Paths, kernel: Kernel) -> Result<()> {
    require_safe_vm_root(paths)?;
    require_safe_existing_vm_directory(paths, kernel)?;
    qualification::invalidate_vmware_generation(
        &vm_directory(paths, kernel).join(qualification::VMWARE_GENERATION_RECEIPT),
    )
}

/// Render the source-bound machine configuration for one serial generation.
pub(crate) fn generation_vmx(kernel: Kernel, serial: &str) -> Result<String> {
    render(
        VMX_TEMPLATE,
        &[
            ("DISPLAY_NAME", display_name(kernel)),
            ("VMDK_NAME", VMDK_NAME.to_string()),
            ("NVME_VMDK_NAME", NVME_VMDK_NAME.to_string()),
            (
                "DMA_HANDOFF_FILE",
                crate::platform_acpi::DMA_HANDOFF_FILE.to_string(),
            ),
            ("SERIAL_LOG", serial.to_string()),
        ],
    )
}

/// Verify source-generated settings and constrained runtime additions.
pub(crate) fn verify_runtime_vmx(generated: &[u8], runtime: &[u8]) -> Result<()> {
    config::verify(generated, runtime)
}

/// Validate a fresh VMware transcript and preserve the exact machine it came from.
pub(crate) fn qualify(paths: &Paths, kernel: Kernel) -> Result<()> {
    let attempt = qualification::begin_vmware(paths, kernel)?;
    let vm_dir = vm_directory(paths, kernel);
    let image = vm_dir.join(FLAT_NAME);
    let namespace = vm_dir.join(NVME_FLAT_NAME);
    let receipt = vm_dir.join(qualification::VMWARE_GENERATION_RECEIPT);
    let transcript = qualification::generated_transcript_path(&receipt, kernel)?;
    attempt.pass(&qualification::VmwareArtifacts {
        image: &image,
        namespace: &namespace,
        vmdk: &vm_dir.join(VMDK_NAME),
        namespace_vmdk: &vm_dir.join(NVME_VMDK_NAME),
        vmx: &vm_dir.join(VMX_NAME),
        handoff: &vm_dir.join(crate::platform_acpi::DMA_HANDOFF_FILE),
        transcript: &transcript,
        receipt: &receipt,
    })?;
    println!(
        "[xtask] VMware {} execution qualified and copied into immutable evidence",
        image_label(kernel)
    );
    Ok(())
}

pub(crate) fn verify_transcript(root: &Path, kernel: Kernel, bytes: &[u8]) -> Result<()> {
    verify_transcript_with(root, kernel, bytes, || Ok(()))
}

fn verify_transcript_with<F>(
    root: &Path,
    kernel: Kernel,
    bytes: &[u8],
    after_nul_check: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let text = std::str::from_utf8(bytes).context("VMware transcript is not UTF-8")?;
    ensure!(
        !text.contains('\0'),
        "VMware transcript contains a NUL byte"
    );
    after_nul_check()?;
    let normalized = text.replace("\r\n", "\n");
    let text = normalized.as_str();
    ensure!(
        text.ends_with('\n')
            && text
                .chars()
                .all(|character| matches!(character, '\n' | '\t') || !character.is_control()),
        "VMware transcript is not complete canonical text"
    );
    ensure!(
        !text.contains("KERNEL PANIC"),
        "VMware transcript contains a kernel panic"
    );
    require_translation_before_storage(text)?;
    match kernel {
        Kernel::SelfTest => verify_selftest_transcript(root, text),
        Kernel::Shipped => verify_shipped_transcript(text),
    }
}

#[derive(Debug, Clone, Copy)]
enum TranscriptMarker {
    Exact(&'static str),
    Isolation(Kernel),
    ExtendedState,
    LocalApic,
    Nvme,
    Fat32,
    Tmpfs,
    UserSlot,
    KeyboardUptime,
}

impl TranscriptMarker {
    const fn example(self) -> &'static str {
        match self {
            Self::Exact(message) => message,
            Self::Isolation(_) => VMWARE_PAIRED_ISOLATION_MARKER,
            Self::ExtendedState => "cpu: extended state 832 bytes, xcr0 0x7, compacted",
            Self::LocalApic => "apic: x2apic id 0 version 0x14 (bootstrap)",
            Self::Nvme => {
                "nvme: up \u{2014} 8 entries a queue, 4096 blocks of 512 bytes, doorbells 4 apart"
            }
            Self::Fat32 => "fat32: mounted \u{2014} 129046 clusters of 512 bytes",
            Self::Tmpfs => "tmpfs: 1 file(s) and 1 directory(ies) at the root",
            Self::UserSlot => "user: program running in slot 0",
            Self::KeyboardUptime => "kernel: reading the keyboard, uptime 1 ms",
        }
    }

    fn matches(self, message: &str) -> bool {
        match self {
            Self::Exact(expected) => message == expected,
            Self::Isolation(kernel) => isolation_markers(kernel).contains(&message),
            Self::ExtendedState => extended_state_record(message),
            Self::LocalApic => local_apic_record(message),
            Self::Nvme => nvme_record(message),
            Self::Fat32 => fat32_record(message),
            Self::Tmpfs => tmpfs_record(message),
            Self::UserSlot => message
                .strip_prefix("user: program running in slot ")
                .and_then(canonical_decimal)
                .is_some(),
            Self::KeyboardUptime => message
                .strip_prefix("kernel: reading the keyboard, uptime ")
                .and_then(|value| value.strip_suffix(" ms"))
                .and_then(canonical_decimal)
                .is_some(),
        }
    }
}

const SELFTEST_MARKERS: &[TranscriptMarker] = &[
    TranscriptMarker::Exact(VMWARE_DMA_HANDOFF_MARKER),
    TranscriptMarker::Exact("cpu: protections wp smep smap umip nx"),
    TranscriptMarker::ExtendedState,
    TranscriptMarker::LocalApic,
    TranscriptMarker::Isolation(Kernel::SelfTest),
    TranscriptMarker::Exact("selftest: a thread crosses into ring three and calls back"),
    TranscriptMarker::Exact("selftest: awaiting storage-delay keystroke"),
    TranscriptMarker::Exact(
        "selftest: delayed read evidence deferred=true device=true retained=true interrupts=true completed=true exact=true",
    ),
    TranscriptMarker::Exact("selftest: the kernel reads the file the host wrote"),
    TranscriptMarker::Exact("selftest: both calibrated rates are believable"),
    TranscriptMarker::Exact("selftest: awaiting injected keystroke"),
    TranscriptMarker::Exact("selftest: an injected keystroke arrives, and its release follows"),
    TranscriptMarker::Exact("selftest: awaiting translated keystroke"),
    TranscriptMarker::Exact("selftest: a translated keystroke decodes to the same key"),
];

const SHIPPED_MARKERS: &[TranscriptMarker] = &[
    TranscriptMarker::Exact(VMWARE_DMA_HANDOFF_MARKER),
    TranscriptMarker::Isolation(Kernel::Shipped),
    TranscriptMarker::Nvme,
    TranscriptMarker::Fat32,
    TranscriptMarker::Tmpfs,
    TranscriptMarker::Exact("devfs: storage is a block device"),
    TranscriptMarker::Exact("devfs: console is a character device"),
    TranscriptMarker::UserSlot,
    TranscriptMarker::KeyboardUptime,
];

const fn isolation_markers(kernel: Kernel) -> &'static [&'static str] {
    match kernel {
        Kernel::SelfTest => SELFTEST_ISOLATION_MARKERS,
        Kernel::Shipped => SHIPPED_ISOLATION_MARKERS,
    }
}

fn positive_decimal(text: &str) -> bool {
    canonical_decimal(text).is_some_and(|value| value != 0)
}

fn positive_hexadecimal(text: &str) -> bool {
    !text.is_empty()
        && (text.len() == 1 || !text.starts_with('0'))
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && u64::from_str_radix(text, 16).is_ok_and(|value| value != 0)
}

fn extended_state_record(message: &str) -> bool {
    let Some(body) = message.strip_prefix("cpu: extended state ") else {
        return false;
    };
    let Some((bytes, xcr0)) = body.split_once(" bytes, xcr0 0x") else {
        return false;
    };
    let Some(xcr0) = xcr0.strip_suffix(", compacted") else {
        return false;
    };
    positive_decimal(bytes) && positive_hexadecimal(xcr0)
}

fn local_apic_record(message: &str) -> bool {
    let Some(body) = message.strip_prefix("apic: x2apic id ") else {
        return false;
    };
    let Some((id, version)) = body.split_once(" version 0x") else {
        return false;
    };
    let Some(version) = version.strip_suffix(" (bootstrap)") else {
        return false;
    };
    canonical_decimal(id).is_some() && positive_hexadecimal(version)
}

fn translation_record(message: &str) -> bool {
    let Some(body) = message.strip_prefix("iommu: translation on for ") else {
        return false;
    };
    let Some((address, body)) = body.split_once(" through the unit at 0x") else {
        return false;
    };
    let Some((unit, _)) = body.split_once(": ") else {
        return false;
    };
    let Some((bus, address)) = address.split_once(':') else {
        return false;
    };
    let Some((device, function)) = address.split_once('.') else {
        return false;
    };
    let hex_byte = |text: &str| {
        text.len() == 2
            && text
                .bytes()
                .all(|digit| digit.is_ascii_digit() || (b'a'..=b'f').contains(&digit))
    };
    hex_byte(bus)
        && hex_byte(device)
        && matches!(function.as_bytes(), [b'0'..=b'7'])
        && positive_hexadecimal(unit)
}

fn nvme_record(message: &str) -> bool {
    let Some(body) = message.strip_prefix("nvme: up \u{2014} ") else {
        return false;
    };
    let Some((entries, body)) = body.split_once(" entries a queue, ") else {
        return false;
    };
    let Some((blocks, body)) = body.split_once(" blocks of ") else {
        return false;
    };
    let Some((bytes, doorbells)) = body.split_once(" bytes, doorbells ") else {
        return false;
    };
    let Some(doorbells) = doorbells.strip_suffix(" apart") else {
        return false;
    };
    [entries, blocks, bytes, doorbells]
        .into_iter()
        .all(positive_decimal)
}

fn fat32_record(message: &str) -> bool {
    let Some(body) = message.strip_prefix("fat32: mounted \u{2014} ") else {
        return false;
    };
    let Some((clusters, bytes)) = body.split_once(" clusters of ") else {
        return false;
    };
    let Some(bytes) = bytes.strip_suffix(" bytes") else {
        return false;
    };
    positive_decimal(clusters) && positive_decimal(bytes)
}

fn tmpfs_record(message: &str) -> bool {
    let Some(body) = message.strip_prefix("tmpfs: ") else {
        return false;
    };
    let Some((files, directories)) = body.split_once(" file(s) and ") else {
        return false;
    };
    let Some(directories) = directories.strip_suffix(" directory(ies) at the root") else {
        return false;
    };
    positive_decimal(files) && positive_decimal(directories)
}

fn info_records(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    text.split_inclusive('\n').filter_map(move |record| {
        let at = offset;
        offset += record.len();
        let line = record.strip_suffix('\n')?;
        let message = line.strip_prefix("[info ] ")?;
        Some((at, message))
    })
}

fn verify_selftest_transcript(root: &Path, text: &str) -> Result<()> {
    ensure!(
        !text.lines().any(|line| {
            line.contains("selftest: ")
                && (line.trim_end().ends_with(" FAILED")
                    || line.contains("selftest: verdict v1 FAIL"))
        }),
        "VMware self-test transcript contains a failed check"
    );
    let evidence_end = require_once_in_order(text, SELFTEST_MARKERS)?;

    let verdicts: Vec<(usize, &str)> = info_records(text)
        .filter(|(_, message)| message.starts_with(crate::qemu::SELFTEST_VERDICT_PREFIX))
        .collect();
    ensure!(
        verdicts.len() == 1,
        "VMware transcript has {} self-test verdicts",
        verdicts.len()
    );
    ensure!(
        verdicts[0].0 >= evidence_end,
        "VMware self-test verdict precedes required execution evidence"
    );
    let expected = expected_selftest_checks(root)?;
    let mut fields = verdicts[0].1[crate::qemu::SELFTEST_VERDICT_PREFIX.len()..].split(' ');
    ensure!(
        core::iter::Iterator::next(&mut fields) == Some("v1"),
        "unsupported self-test verdict"
    );
    ensure!(
        core::iter::Iterator::next(&mut fields) == Some("PASS"),
        "self-test verdict did not pass"
    );
    let passed = verdict_count(core::iter::Iterator::next(&mut fields), "passed=")?;
    let failed = verdict_count(core::iter::Iterator::next(&mut fields), "failed=")?;
    let reported = verdict_count(core::iter::Iterator::next(&mut fields), "expected=")?;
    ensure!(
        core::iter::Iterator::next(&mut fields).is_none(),
        "self-test verdict has trailing fields"
    );
    ensure!(
        expected != 0 && passed == expected && failed == 0 && reported == expected,
        "self-test verdict does not match the current {expected}-check image"
    );
    Ok(())
}

fn verify_shipped_transcript(text: &str) -> Result<()> {
    ensure!(
        !text.contains("selftest: verdict"),
        "the shipped VMware transcript came from a self-test image"
    );
    ensure!(
        !text.contains("user: no program"),
        "the shipped VMware image did not launch its program"
    );
    require_once_in_order(text, SHIPPED_MARKERS)?;
    Ok(())
}

// Each storage candidate logs its own translation record and a retired one's
// comes first, so the last record is the kept controller's (D-358).
fn require_translation_before_storage(text: &str) -> Result<()> {
    let translation = info_records(text)
        .filter(|(_, message)| translation_record(message))
        .map(|(at, _)| at)
        .last()
        .context("VMware transcript has no canonical `iommu: translation on` record")?;
    let storage = info_records(text)
        .filter(|(_, message)| nvme_record(message))
        .map(|(at, _)| at)
        .collect::<Vec<_>>();
    let [up] = storage.as_slice() else {
        bail!(
            "VMware transcript contains canonical `{}` {} times",
            TranscriptMarker::Nvme.example(),
            storage.len()
        );
    };
    ensure!(
        translation < *up,
        "VMware transcript turns translation on after `nvme: up`"
    );
    Ok(())
}

fn isolation_occurrence(text: &str, kernel: Kernel) -> Result<(usize, &str)> {
    let mut offset = 0;
    let records = text
        .split_inclusive('\n')
        .filter_map(|record| {
            let at = offset;
            offset += record.len();
            let line = record.strip_suffix('\n')?;
            line.contains(VMWARE_ISOLATION_PREFIX).then_some((at, line))
        })
        .collect::<Vec<_>>();
    ensure!(
        records.len() == 1,
        "VMware transcript contains {} user-isolation records",
        records.len()
    );
    let (at, record) = records[0];
    let message = record
        .strip_prefix("[info ] ")
        .context("VMware user-isolation record is not canonically info-framed")?;
    ensure!(
        isolation_markers(kernel).contains(&message),
        "VMware transcript contains a user-isolation record not allowed for the selected image"
    );
    Ok((at, message))
}

fn require_once_in_order(text: &str, markers: &[TranscriptMarker]) -> Result<usize> {
    let mut end = 0;
    for marker in markers {
        let occurrences: Vec<(usize, &str)> = match *marker {
            TranscriptMarker::Isolation(kernel) => vec![isolation_occurrence(text, kernel)?],
            _ => info_records(text)
                .filter(|(_, message)| marker.matches(message))
                .collect(),
        };
        ensure!(
            occurrences.len() == 1,
            "VMware transcript contains canonical `{}` {} times",
            marker.example(),
            occurrences.len()
        );
        let (at, message) = occurrences[0];
        ensure!(
            at >= end,
            "VMware transcript marker `{}` is out of order",
            marker.example()
        );
        end = at + "[info ] ".len() + message.len() + 1;
    }
    Ok(end)
}

fn verdict_count(field: Option<&str>, prefix: &str) -> Result<usize> {
    let value = field
        .and_then(|field| field.strip_prefix(prefix))
        .context("self-test verdict is missing a count")?;
    usize::try_from(
        canonical_decimal(value).context("self-test verdict contains a noncanonical count")?,
    )
    .context("self-test verdict count does not fit usize")
}

fn expected_selftest_checks(root: &Path) -> Result<usize> {
    let kernel = source_usize(
        &root.join("kernel/src/selftest.rs"),
        "const EXPECTED_CHECKS: usize = ",
    )?;
    let harness = source_usize(
        &root.join("xtask/src/qemu.rs"),
        "const EXPECTED_SELFTEST_CHECKS: usize = ",
    )?;
    ensure!(
        kernel == harness,
        "kernel expects {kernel} checks but the harness expects {harness}"
    );
    Ok(kernel)
}

fn source_usize(path: &Path, prefix: &str) -> Result<usize> {
    let source = qualification::read_bounded_text(path, MAX_SOURCE_BYTES, "VMware source input")?;
    let values: Vec<usize> = source
        .lines()
        .filter_map(|line| line.trim().strip_prefix(prefix))
        .map(|value| {
            <usize as core::str::FromStr>::from_str(
                value
                    .strip_suffix(';')
                    .context("source count has no semicolon")?,
            )
            .context("source count is not a number")
        })
        .collect::<Result<_>>()?;
    ensure!(
        values.len() == 1,
        "{} contains {} `{prefix}` constants",
        path.display(),
        values.len()
    );
    Ok(values[0])
}

fn vm_directory(paths: &Paths, kernel: Kernel) -> PathBuf {
    match kernel {
        Kernel::SelfTest => paths.vm_dir().join("self-test"),
        Kernel::Shipped => paths.vm_dir().join("shipped"),
    }
}

fn reset_vm_directory(paths: &Paths, kernel: Kernel) -> Result<()> {
    let root = paths.vm_dir();
    require_safe_vm_root(paths)?;
    ensure!(
        root == paths.root().join("vm") && root.parent() == Some(paths.root()),
        "VMware root escaped the workspace"
    );
    crate::mutation_workspace::prepare_real_directory_tree(&root, "VMware root")?;
    require_canonical_child(paths.root(), &root, "VMware root")?;

    let directory = vm_directory(paths, kernel);
    ensure!(
        directory.parent() == Some(root.as_path()),
        "VMware selector directory escaped its root"
    );
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            require_canonical_child(&root, &directory, "VMware selector directory")?;
            std::fs::remove_dir_all(&directory)
                .with_context(|| format!("resetting {}", directory.display()))?;
        }
        Ok(_) => std::fs::remove_file(&directory)
            .with_context(|| format!("removing unexpected {}", directory.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", directory.display()));
        }
    }
    std::fs::create_dir(&directory)
        .with_context(|| format!("creating fresh VMware directory {}", directory.display()))
}

fn require_safe_vm_root(paths: &Paths) -> Result<()> {
    let root = paths.vm_dir();
    ensure!(
        root == paths.root().join("vm") && root.parent() == Some(paths.root()),
        "VMware root escaped the workspace"
    );
    match std::fs::symlink_metadata(&root) {
        Ok(metadata) => {
            ensure!(
                metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
                "VMware root is not a real directory"
            );
            require_canonical_child(paths.root(), &root, "VMware root")?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", root.display())),
    }
}

fn require_safe_existing_vm_directory(paths: &Paths, kernel: Kernel) -> Result<()> {
    let root = paths.vm_dir();
    let directory = vm_directory(paths, kernel);
    ensure!(
        directory.parent() == Some(root.as_path()),
        "VMware selector directory escaped its root"
    );
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            require_canonical_child(&root, &directory, "VMware selector directory")?;
            Ok(())
        }
        Ok(_) => bail!("VMware selector is not a real directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", directory.display())),
    }
}

fn require_canonical_child(parent: &Path, child: &Path, label: &str) -> Result<()> {
    let canonical_parent =
        crate::mutation_workspace::canonical_existing_real_directory(parent, label)?;
    let canonical_child =
        crate::mutation_workspace::canonical_existing_real_directory(child, label)?;
    ensure!(
        canonical_child.parent() == Some(canonical_parent.as_path()),
        "{label} escaped its canonical parent"
    );
    Ok(())
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

fn copy_new(source: &Path, destination: &Path) -> Result<u64> {
    let mut input = File::open(source)
        .with_context(|| format!("opening generated input {}", source.display()))?;
    let mut output = create_new_output(destination)?;
    let copied = std::io::copy(&mut input, &mut output).with_context(|| {
        format!(
            "copying {} to new VMware output {}",
            source.display(),
            destination.display()
        )
    })?;
    output
        .sync_all()
        .with_context(|| format!("syncing VMware output {}", destination.display()))?;
    Ok(copied)
}

fn write_new(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = create_new_output(path)?;
    file.write_all(contents)
        .with_context(|| format!("writing new VMware output {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing VMware output {}", path.display()))
}

fn create_new_output(path: &Path) -> Result<File> {
    let parent = path.parent().context("VMware output has no parent")?;
    crate::mutation_workspace::canonical_existing_real_directory(parent, "VMware output parent")?;
    let mut options = OpenOptions::new();
    OpenOptions::create_new(&mut options, true);
    OpenOptions::write(&mut options, true);
    let file = OpenOptions::open(&options, path)
        .with_context(|| format!("creating new VMware output {}", path.display()))?;
    crate::mutation_workspace::require_direct_file(parent, path, "VMware output")?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE_SECTORS: u64 = 64 * 1024 * 1024 / 512;

    #[cfg(unix)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating VMware directory symlink");
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_dir(target, link).expect("creating VMware directory symlink");
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating VMware file symlink");
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_file(target, link).expect("creating VMware file symlink");
    }

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace above xtask")
            .to_path_buf()
    }

    #[test]
    fn vmware_generation_never_writes_through_preexisting_output_links() {
        let root = std::env::temp_dir().join(format!(
            "wizard-vmware-output-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory = root.join("selector");
        let input = root.join("input");
        let sentinel = root.join("outside-sentinel");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&input, b"new disk bytes").unwrap();
        std::fs::write(&sentinel, b"outside authority").unwrap();

        let disk = directory.join(FLAT_NAME);
        std::fs::hard_link(&sentinel, &disk).unwrap();
        let copy = copy_new(&input, &disk);
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"outside authority",
            "a generated disk truncated an external hardlink"
        );
        assert!(copy.is_err(), "a preexisting generated disk was accepted");
        std::fs::remove_file(&disk).unwrap();

        let descriptor = directory.join(VMX_NAME);
        create_file_symlink(&sentinel, &descriptor);
        let write = write_new(&descriptor, b"new VMX bytes");
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"outside authority",
            "a generated descriptor followed an external symlink"
        );
        assert!(
            write.is_err(),
            "a preexisting generated descriptor was accepted"
        );
        std::fs::remove_file(&descriptor).unwrap();

        let copied = copy_new(&input, &disk).unwrap();
        assert_eq!(copied, b"new disk bytes".len() as u64);
        assert_eq!(std::fs::read(&disk).unwrap(), b"new disk bytes");
        write_new(&descriptor, b"new VMX bytes").unwrap();
        assert_eq!(std::fs::read(&descriptor).unwrap(), b"new VMX bytes");

        std::fs::remove_dir_all(root).unwrap();
    }

    // One translation record per storage candidate: the retired first one's,
    // then the kept controller's.
    const RETIRED_TRANSLATION: &str = "[info ] iommu: translation on for 00:04.0 through the unit at 0xfed90000: 4-level walks, flushed tables, caching mode, cap 0x80d2008c222f06c6, ecap 0xf02\n";
    const KEPT_TRANSLATION: &str = "[info ] iommu: translation on for 00:05.0 through the unit at 0xfed90000: 4-level walks, flushed tables, caching mode, cap 0x80d2008c222f06c6, ecap 0xf02\n";
    const FIXTURE_PAIRED_ISOLATION: &str =
        "cpu: user isolation paired roots: selected; shared mappings refused";
    const FIXTURE_SELFTEST_SHARED_ISOLATION: &str =
        "cpu: user isolation paired roots: forced for self-test; shared proof retained: rdcl_no";
    const FIXTURE_SHIPPED_SHARED_ISOLATION: &str =
        "cpu: user isolation shared kernel mappings: rdcl_no";
    const FIXTURE_QEMU_ISOLATION: &str = "cpu: user isolation shared kernel mappings: qemu_tcg";

    fn storage_records() -> String {
        format!(
            "{RETIRED_TRANSLATION}{KEPT_TRANSLATION}[info ] nvme: up \u{2014} 8 entries a queue, 4096 blocks of 512 bytes, doorbells 4 apart\n"
        )
    }

    fn selftest_transcript_with_isolation(isolation: &str) -> String {
        let expected = expected_selftest_checks(&repository_root()).expect("current check count");
        let mut transcript = String::new();
        for marker in [
            "iommu: entry DMA fence retained VMware 440BX 00:00.0 as the qualified non-requester",
            "cpu: protections wp smep smap umip nx",
            "cpu: extended state 832 bytes, xcr0 0x7, compacted",
            "apic: x2apic id 0 version 0x14 (bootstrap)",
        ] {
            transcript.push_str("[info ] ");
            transcript.push_str(marker);
            transcript.push('\n');
        }
        transcript.push_str("[info ] ");
        transcript.push_str(isolation);
        transcript.push('\n');
        transcript.push_str(&storage_records());
        for marker in [
            "selftest: a thread crosses into ring three and calls back",
            "selftest: awaiting storage-delay keystroke",
            "selftest: delayed read evidence deferred=true device=true retained=true interrupts=true completed=true exact=true",
            "selftest: the kernel reads the file the host wrote",
            "selftest: both calibrated rates are believable",
            "selftest: awaiting injected keystroke",
            "selftest: an injected keystroke arrives, and its release follows",
            "selftest: awaiting translated keystroke",
            "selftest: a translated keystroke decodes to the same key",
        ] {
            transcript.push_str("[info ] ");
            transcript.push_str(marker);
            transcript.push('\n');
        }
        transcript.push_str(&format!(
            "[info ] selftest: verdict v1 PASS passed={expected} failed=0 expected={expected}\n"
        ));
        transcript
    }

    fn selftest_transcript() -> String {
        selftest_transcript_with_isolation(FIXTURE_PAIRED_ISOLATION)
    }

    fn shipped_transcript_with_isolation(isolation: &str) -> String {
        let mut transcript = format!(
            "[info ] iommu: entry DMA fence retained VMware 440BX 00:00.0 as the qualified non-requester\n[info ] {isolation}\n"
        );
        transcript.push_str(&storage_records());
        for marker in [
            "fat32: mounted \u{2014} 129046 clusters of 512 bytes",
            "tmpfs: 1 file(s) and 1 directory(ies) at the root",
            "devfs: storage is a block device",
            "devfs: console is a character device",
            "user: program running in slot 0",
            "kernel: reading the keyboard, uptime 1 ms",
        ] {
            transcript.push_str("[info ] ");
            transcript.push_str(marker);
            transcript.push('\n');
        }
        transcript
    }

    fn shipped_transcript() -> String {
        shipped_transcript_with_isolation(FIXTURE_PAIRED_ISOLATION)
    }

    #[test]
    fn the_vmware_dma_handoff_marker_is_required_by_both_selectors() {
        for markers in [SELFTEST_MARKERS, SHIPPED_MARKERS] {
            assert_eq!(
                markers
                    .iter()
                    .filter(|marker| matches!(
                        marker,
                        TranscriptMarker::Exact(message)
                            if *message == VMWARE_DMA_HANDOFF_MARKER
                    ))
                    .count(),
                1,
                "each VMware selector must require exactly one consumed DMA policy witness"
            );
        }

        let kernel = std::fs::read_to_string(repository_root().join("kernel/src/main.rs"))
            .expect("reading kernel entry source");
        assert!(
            kernel.contains(VMWARE_DMA_HANDOFF_MARKER),
            "the verifier marker must remain the kernel's exact emitted record"
        );

        let root = repository_root();
        let record = format!("[info ] {VMWARE_DMA_HANDOFF_MARKER}\n");
        for (selector, transcript) in [
            (Kernel::SelfTest, selftest_transcript()),
            (Kernel::Shipped, shipped_transcript()),
        ] {
            verify_transcript(&root, selector, transcript.as_bytes())
                .expect("canonical VMware DMA witness");
            let omitted = transcript.replacen(&record, "", 1);
            assert!(
                verify_transcript(&root, selector, omitted.as_bytes()).is_err(),
                "selector accepted a transcript without its VMware DMA witness"
            );
        }
    }

    /// The transcript with two markers' records exchanged.
    fn swapped(transcript: &str, first: TranscriptMarker, second: TranscriptMarker) -> String {
        let mut lines: Vec<&str> = transcript.lines().collect();
        let at = |marker: TranscriptMarker| {
            let record = format!("[info ] {}", marker.example());
            lines
                .iter()
                .position(|line| *line == record)
                .expect("marker record")
        };
        let (first, second) = (at(first), at(second));
        lines.swap(first, second);
        format!("{}\n", lines.join("\n"))
    }

    #[test]
    fn selftest_verdict_counts_use_one_canonical_decimal_grammar() {
        let root = repository_root();
        let valid = selftest_transcript();
        let expected = expected_selftest_checks(&root).unwrap();
        verify_transcript(&root, Kernel::SelfTest, valid.as_bytes()).unwrap();

        for forged in [
            valid.replace(
                &format!("passed={expected}"),
                &format!("passed=0{expected}"),
            ),
            valid.replace("failed=0", "failed=+0"),
            valid.replace(
                &format!("expected={expected}"),
                &format!("expected=0{expected}"),
            ),
            valid.replace(&format!("passed={expected}"), "passed=18446744073709551616"),
            valid.replace(&format!("passed={expected}"), "passed=\u{0661}"),
        ] {
            assert!(verify_transcript(&root, Kernel::SelfTest, forged.as_bytes()).is_err());
        }
    }

    #[test]
    fn vmware_selftest_verdict_requires_single_ascii_spaces() {
        let root = repository_root();
        let valid = selftest_transcript();
        verify_transcript(&root, Kernel::SelfTest, valid.as_bytes()).unwrap();
        let verdict = format!("{}\n", valid.lines().last().expect("verdict record"));

        for forged in [
            valid.replace("verdict v1 PASS", "verdict v1  PASS"),
            valid.replace("verdict v1 PASS", "verdict v1\tPASS"),
            valid.replace("verdict v1 PASS", "verdict v1\u{a0}PASS"),
            valid.replace(" failed=0", "  failed=0"),
            valid.replacen(&verdict, &verdict.replace('\n', " \n"), 1),
            valid.replacen(&verdict, &verdict.replace('\n', "\t\n"), 1),
            valid.replacen(&verdict, &verdict.replace('\n', "\u{a0}\n"), 1),
            valid.trim_end_matches('\n').to_string(),
        ] {
            assert!(
                verify_transcript(&root, Kernel::SelfTest, forged.as_bytes()).is_err(),
                "accepted noncanonical verdict spacing: {forged:?}"
            );
        }
    }

    #[test]
    fn oversized_source_counts_are_refused_before_text_allocation() {
        let root =
            std::env::temp_dir().join(format!("wizard-vmware-source-bound-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir(&root).unwrap();
        let path = root.join("selftest.rs");
        File::create(&path)
            .unwrap()
            .set_len(MAX_SOURCE_BYTES + 1)
            .unwrap();

        let error = source_usize(&path, "const EXPECTED_CHECKS: usize = ")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("exceeding the 4194304-byte allocation limit"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vmware_isolation_markers_are_bound_to_kernel_producers() {
        let kernel = std::fs::read_to_string(repository_root().join("kernel/src/main.rs"))
            .expect("reading kernel entry source");
        let forced = VMWARE_SELFTEST_SHARED_ISOLATION_MARKER.replace("rdcl_no", "{reason}");
        let shared = VMWARE_SHIPPED_SHARED_ISOLATION_MARKER.replace("rdcl_no", "{reason}");

        for producer in [
            format!("\"{VMWARE_PAIRED_ISOLATION_MARKER}\""),
            format!("\"{forced}\""),
            format!("info!(\"{shared}\")"),
        ] {
            assert!(
                kernel.contains(&producer),
                "the verifier marker is not bound to kernel producer {producer}"
            );
        }
    }

    #[test]
    fn vmware_isolation_records_are_selector_specific_exact_and_unique() {
        let root = repository_root();
        for (kernel, transcript) in [
            (
                Kernel::SelfTest,
                selftest_transcript_with_isolation(FIXTURE_PAIRED_ISOLATION),
            ),
            (
                Kernel::SelfTest,
                selftest_transcript_with_isolation(FIXTURE_SELFTEST_SHARED_ISOLATION),
            ),
            (
                Kernel::Shipped,
                shipped_transcript_with_isolation(FIXTURE_PAIRED_ISOLATION),
            ),
            (
                Kernel::Shipped,
                shipped_transcript_with_isolation(FIXTURE_SHIPPED_SHARED_ISOLATION),
            ),
        ] {
            verify_transcript(&root, kernel, transcript.as_bytes())
                .expect("selector-appropriate isolation evidence");
        }

        let selftest = selftest_transcript();
        let shipped = shipped_transcript();
        let paired = format!("[info ] {FIXTURE_PAIRED_ISOLATION}\n");
        let selftest_shared = format!("[info ] {FIXTURE_SELFTEST_SHARED_ISOLATION}\n");
        let shipped_shared = format!("[info ] {FIXTURE_SHIPPED_SHARED_ISOLATION}\n");
        let qemu = format!("[info ] {FIXTURE_QEMU_ISOLATION}\n");
        let unknown = "[info ] cpu: user isolation invented native claim\n";
        let trailing = format!("[info ] {FIXTURE_PAIRED_ISOLATION} trailing\n");
        let warning = format!("[warn ] {FIXTURE_PAIRED_ISOLATION}\n");

        let rejected = [
            (
                Kernel::SelfTest,
                selftest_transcript_with_isolation(FIXTURE_SHIPPED_SHARED_ISOLATION),
            ),
            (
                Kernel::Shipped,
                shipped_transcript_with_isolation(FIXTURE_SELFTEST_SHARED_ISOLATION),
            ),
            (
                Kernel::SelfTest,
                selftest_transcript_with_isolation(FIXTURE_QEMU_ISOLATION),
            ),
            (
                Kernel::Shipped,
                shipped_transcript_with_isolation(FIXTURE_QEMU_ISOLATION),
            ),
            (Kernel::SelfTest, selftest.replacen(&paired, "", 1)),
            (Kernel::Shipped, shipped.replacen(&paired, "", 1)),
            (Kernel::SelfTest, selftest.replacen(&paired, unknown, 1)),
            (Kernel::SelfTest, selftest.replacen(&paired, &trailing, 1)),
            (Kernel::SelfTest, selftest.replacen(&paired, &warning, 1)),
            (
                Kernel::SelfTest,
                selftest.replacen(&paired, &format!("{paired}{paired}"), 1),
            ),
            (
                Kernel::SelfTest,
                selftest.replacen(&paired, &format!("{paired}{selftest_shared}"), 1),
            ),
            (
                Kernel::Shipped,
                shipped.replacen(&paired, &format!("{paired}{shipped_shared}"), 1),
            ),
            (
                Kernel::SelfTest,
                selftest.replacen(&paired, &format!("{paired}{qemu}"), 1),
            ),
        ];
        for (kernel, transcript) in rejected {
            assert!(
                verify_transcript(&root, kernel, transcript.as_bytes()).is_err(),
                "accepted missing, ambiguous, cross-selector or inexact isolation evidence"
            );
        }
    }

    #[test]
    fn uart_crlf_and_lf_are_both_valid_transcript_terminators() {
        let root = repository_root();
        for (kernel, transcript) in [
            (Kernel::SelfTest, selftest_transcript()),
            (Kernel::Shipped, shipped_transcript()),
        ] {
            verify_transcript(&root, kernel, transcript.as_bytes()).unwrap();
            verify_transcript(&root, kernel, transcript.replace('\n', "\r\n").as_bytes()).unwrap();
        }
    }

    #[test]
    fn only_a_carriage_return_paired_with_line_feed_is_allowed() {
        let root = repository_root();
        let valid = selftest_transcript();
        for poisoned in [
            valid.replacen("[info ]", "[info\r ]", 1),
            format!("\r{valid}"),
            valid.replace('\n', "\r\r\n"),
            format!("{valid}\r"),
        ] {
            assert!(verify_transcript(&root, Kernel::SelfTest, poisoned.as_bytes()).is_err());
        }
    }

    #[test]
    fn a_vmware_selftest_transcript_is_complete_current_and_unambiguous() {
        let root = repository_root();
        let valid = selftest_transcript();
        assert!(verify_transcript(&root, Kernel::SelfTest, valid.as_bytes()).is_ok());

        for marker in SELFTEST_MARKERS {
            let omitted = valid.replacen(marker.example(), "omitted", 1);
            assert!(
                verify_transcript(&root, Kernel::SelfTest, omitted.as_bytes()).is_err(),
                "omitting {} was accepted",
                marker.example()
            );
        }
        let marker_record = format!("[info ] {}\n", SELFTEST_MARKERS[0].example());
        let duplicated_marker = valid.replacen(
            &marker_record,
            &format!("{marker_record}{marker_record}"),
            1,
        );
        assert!(verify_transcript(&root, Kernel::SelfTest, duplicated_marker.as_bytes()).is_err());
        let duplicated = format!("{valid}{}", valid.lines().last().expect("verdict line"));
        assert!(verify_transcript(&root, Kernel::SelfTest, duplicated.as_bytes()).is_err());
        let reordered = swapped(&valid, SELFTEST_MARKERS[4], SELFTEST_MARKERS[5]);
        assert!(verify_transcript(&root, Kernel::SelfTest, reordered.as_bytes()).is_err());
        let split_compound = valid.replace(", compacted", "\n[info ] , compacted");
        assert!(verify_transcript(&root, Kernel::SelfTest, split_compound.as_bytes()).is_err());
        let merged_steps = valid.replace("\n[info ] apic: x2apic", " [info ] apic: x2apic");
        assert!(verify_transcript(&root, Kernel::SelfTest, merged_steps.as_bytes()).is_err());
        let expected = expected_selftest_checks(&root).unwrap();
        let stale = valid.replace(
            &format!("passed={expected}"),
            &format!("passed={}", expected - 1),
        );
        assert!(verify_transcript(&root, Kernel::SelfTest, stale.as_bytes()).is_err());
        let panicked = format!("{valid}[error] KERNEL PANIC\n");
        assert!(verify_transcript(&root, Kernel::SelfTest, panicked.as_bytes()).is_err());
        let failed = format!("{valid}[error] selftest: injected defect FAILED\n");
        assert!(verify_transcript(&root, Kernel::SelfTest, failed.as_bytes()).is_err());
        for poisoned in [
            valid.trim_end_matches('\n').to_string(),
            valid.replacen("[info ]", "[info\r ]", 1),
            valid.replacen("[info ]", "[info\u{1b} ]", 1),
            valid.replacen("[info ]", "[info\u{8} ]", 1),
        ] {
            assert!(
                verify_transcript(&root, Kernel::SelfTest, poisoned.as_bytes()).is_err(),
                "accepted a partial or terminal-active transcript: {poisoned:?}"
            );
        }
        let mut invalid_utf8 = b"[info ] invalid byte: ".to_vec();
        invalid_utf8.push(0xff);
        invalid_utf8.push(b'\n');
        invalid_utf8.extend_from_slice(valid.as_bytes());
        assert!(verify_transcript(&root, Kernel::SelfTest, &invalid_utf8).is_err());
    }

    #[test]
    fn a_nul_transcript_is_rejected_before_canonical_text_validation() {
        let root = repository_root();
        let with_nul = format!("{}\0", selftest_transcript());
        let canonical_validation_started = std::cell::Cell::new(false);

        assert!(
            verify_transcript_with(&root, Kernel::SelfTest, with_nul.as_bytes(), || {
                canonical_validation_started.set(true);
                Ok(())
            })
            .is_err()
        );
        assert!(!canonical_validation_started.get());
    }

    #[test]
    fn a_shipped_vmware_transcript_proves_liveness_without_selftest_substitution() {
        let root = repository_root();
        let valid = shipped_transcript();
        assert!(verify_transcript(&root, Kernel::Shipped, valid.as_bytes()).is_ok());

        for marker in SHIPPED_MARKERS {
            let omitted = valid.replacen(marker.example(), "omitted", 1);
            assert!(
                verify_transcript(&root, Kernel::Shipped, omitted.as_bytes()).is_err(),
                "omitting {} was accepted",
                marker.example()
            );
        }
        let reordered = swapped(&valid, SHIPPED_MARKERS[0], SHIPPED_MARKERS[1]);
        assert!(verify_transcript(&root, Kernel::Shipped, reordered.as_bytes()).is_err());
        let selftest = format!("{valid}selftest: verdict v1 PASS passed=1 failed=0 expected=1\n");
        assert!(verify_transcript(&root, Kernel::Shipped, selftest.as_bytes()).is_err());
        let refusal = format!("{valid}user: no program: refused\n");
        assert!(verify_transcript(&root, Kernel::Shipped, refusal.as_bytes()).is_err());
    }

    #[test]
    fn vmware_evidence_requires_canonical_info_records() {
        let root = repository_root();
        for (kernel, valid, markers) in [
            (Kernel::SelfTest, selftest_transcript(), SELFTEST_MARKERS),
            (Kernel::Shipped, shipped_transcript(), SHIPPED_MARKERS),
        ] {
            for marker in markers {
                let canonical = format!("[info ] {}", marker.example());
                for forged in [
                    valid.replacen(&canonical, &format!("[warn ] {}", marker.example()), 1),
                    valid.replacen(
                        &canonical,
                        &format!("[info ] forged {}", marker.example()),
                        1,
                    ),
                    valid.replacen(&canonical, &format!("{canonical} trailing"), 1),
                ] {
                    assert!(
                        verify_transcript(&root, kernel, forged.as_bytes()).is_err(),
                        "accepted a noncanonical record for {}",
                        marker.example()
                    );
                }
            }

            let first = format!("\n[info ] {}", markers[1].example());
            let merged = valid.replacen(&first, &format!(" [info ] {}", markers[1].example()), 1);
            assert!(
                verify_transcript(&root, kernel, merged.as_bytes()).is_err(),
                "accepted two execution records on one line"
            );
        }

        let valid = selftest_transcript();
        for forged in [
            valid.replacen("[info ] selftest: verdict", "[warn ] selftest: verdict", 1),
            valid.replacen(
                "[info ] selftest: verdict",
                "[info ] forged selftest: verdict",
                1,
            ),
        ] {
            assert!(
                verify_transcript(&root, Kernel::SelfTest, forged.as_bytes()).is_err(),
                "accepted a noncanonical self-test verdict"
            );
        }
    }

    #[test]
    fn a_vmware_transcript_without_translation_is_refused() {
        let root = repository_root();
        let storage = format!("[info ] {}\n", TranscriptMarker::Nvme.example());
        for (kernel, valid) in [
            (Kernel::SelfTest, selftest_transcript()),
            (Kernel::Shipped, shipped_transcript()),
        ] {
            let untranslated = valid.replacen(&storage_records(), &storage, 1);
            let error = verify_transcript(&root, kernel, untranslated.as_bytes()).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("no canonical `iommu: translation on` record"),
                "{error}"
            );
        }
    }

    #[test]
    fn a_vmware_translation_after_storage_is_refused() {
        let root = repository_root();
        let storage = format!("[info ] {}\n", TranscriptMarker::Nvme.example());
        // The retired candidate's record still precedes storage, so only the
        // last record can decide.
        let late = format!("{RETIRED_TRANSLATION}{storage}{KEPT_TRANSLATION}");
        for (kernel, valid) in [
            (Kernel::SelfTest, selftest_transcript()),
            (Kernel::Shipped, shipped_transcript()),
        ] {
            let reordered = valid.replacen(&storage_records(), &late, 1);
            let error = verify_transcript(&root, kernel, reordered.as_bytes()).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("translation on after `nvme: up`"),
                "{error}"
            );
        }
    }

    #[test]
    fn two_vmware_translations_before_storage_qualify() {
        let root = repository_root();
        let storage = format!("[info ] {}\n", TranscriptMarker::Nvme.example());
        let retirement = "[info ] nvme: candidate 1 retired cleanly; trying next\n";
        let separated = format!("{RETIRED_TRANSLATION}{retirement}{KEPT_TRANSLATION}{storage}");
        for (kernel, valid) in [
            (Kernel::SelfTest, selftest_transcript()),
            (Kernel::Shipped, shipped_transcript()),
        ] {
            assert!(valid.contains(&storage_records()));
            verify_transcript(&root, kernel, valid.as_bytes()).unwrap();
            let spaced = valid.replacen(&storage_records(), &separated, 1);
            verify_transcript(&root, kernel, spaced.as_bytes()).unwrap();
        }
    }

    #[test]
    fn a_warning_translation_record_does_not_count() {
        let root = repository_root();
        let storage = format!("[info ] {}\n", TranscriptMarker::Nvme.example());
        let retired = RETIRED_TRANSLATION.replacen("[info ]", "[warn ]", 1);
        let kept = KEPT_TRANSLATION.replacen("[info ]", "[warn ]", 1);
        let warned = format!("{retired}{kept}{storage}");
        for (kernel, valid) in [
            (Kernel::SelfTest, selftest_transcript()),
            (Kernel::Shipped, shipped_transcript()),
        ] {
            let demoted = valid.replacen(&storage_records(), &warned, 1);
            let error = verify_transcript(&root, kernel, demoted.as_bytes()).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("no canonical `iommu: translation on` record"),
                "{error}"
            );

            let late_warning = valid.replacen(&storage, &format!("{storage}{kept}"), 1);
            verify_transcript(&root, kernel, late_warning.as_bytes()).unwrap();
        }
    }

    #[test]
    fn selftest_and_shipped_vms_do_not_share_machine_state() {
        let paths = Paths::new(PathBuf::from("/repo"), None);
        assert_eq!(
            vm_directory(&paths, Kernel::SelfTest),
            paths.vm_dir().join("self-test")
        );
        assert_eq!(
            vm_directory(&paths, Kernel::Shipped),
            paths.vm_dir().join("shipped")
        );
    }

    #[test]
    fn a_generated_vm_creates_one_empty_serial_transcript() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("host clock")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("wizard-vmware-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).expect("VMware fixture directory");
        let log = root.join(format!("serial-{}.log", "1".repeat(64)));

        write_new(&log, &[]).expect("fresh transcript");

        assert_eq!(std::fs::read(&log).unwrap(), b"");
        assert!(
            write_new(&log, &[]).is_err(),
            "a second generation reused the transcript path"
        );
        std::fs::remove_dir_all(root).expect("remove VMware fixture");
    }

    #[test]
    fn generation_replaces_only_its_exact_selector_directory() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("host clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "wizard-vmware-reset-{}-{nonce}",
            std::process::id()
        ));
        let repository = root.join("repository");
        let paths = Paths::new(repository.clone(), None);
        let selftest = vm_directory(&paths, Kernel::SelfTest);
        let shipped = vm_directory(&paths, Kernel::Shipped);
        std::fs::create_dir_all(&selftest).expect("self-test selector");
        std::fs::create_dir_all(&shipped).expect("shipped selector");
        std::fs::write(selftest.join("nvram"), "stale firmware state").unwrap();
        std::fs::write(selftest.join("bedrock.vmss"), "stale suspend state").unwrap();
        std::fs::write(shipped.join("keep"), "other selector").unwrap();

        reset_vm_directory(&paths, Kernel::SelfTest).expect("reset self-test selector");

        assert!(selftest.is_dir());
        assert!(
            crate::paths::directory_entries(&selftest)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            std::fs::read_to_string(shipped.join("keep")).unwrap(),
            "other selector"
        );
        std::fs::remove_dir_all(root).expect("remove VMware reset fixture");
    }

    #[test]
    fn a_redirected_repository_ancestor_is_refused_before_vmware_root_creation() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("host clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "wizard-vmware-ancestor-{}-{nonce}",
            std::process::id()
        ));
        let outside_parent = root.join("outside-parent");
        let outside_repository = outside_parent.join("repository");
        let redirected_parent = root.join("redirected-parent");
        std::fs::create_dir_all(&outside_repository).expect("outside repository");
        create_directory_symlink(&outside_parent, &redirected_parent);
        let paths = Paths::new(redirected_parent.join("repository"), None);

        assert!(reset_vm_directory(&paths, Kernel::SelfTest).is_err());
        assert!(
            !outside_repository.join("vm").exists(),
            "VMware root was created through a redirected repository ancestor"
        );
        std::fs::remove_dir_all(root).expect("remove redirected VMware fixture");
    }

    #[test]
    fn destructive_vmware_paths_require_one_canonical_parent() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("host clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "wizard-vmware-canonical-{}-{nonce}",
            std::process::id()
        ));
        let parent = root.join("parent");
        let child = parent.join("child");
        let sibling = root.join("sibling");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::create_dir(&sibling).unwrap();

        require_canonical_child(&parent, &child, "fixture child").unwrap();
        assert!(require_canonical_child(&parent, &sibling, "fixture sibling").is_err());

        let redirected_parent = root.join("redirected-parent");
        create_directory_symlink(&parent, &redirected_parent);
        assert!(
            require_canonical_child(
                &redirected_parent,
                &redirected_parent.join("child"),
                "redirected fixture child"
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generation_invalidates_success_and_receipt_before_any_fallible_preflight() {
        let dispatch = include_str!("lib.rs");
        assert!(
            dispatch.contains("Task::QualifyVmware { kernel } => vmware::qualify(&paths, kernel),")
        );
        let vmware = dispatch.find("Task::Vmware { profile, kernel }").unwrap();
        let qualify = dispatch[vmware..].find("Task::QualifyVmware").unwrap() + vmware;
        let arm = &dispatch[vmware..qualify];
        let invalidate_pass = arm.find("qualification::invalidate_vmware").unwrap();
        let invalidate_receipt = arm.find("vmware::invalidate_generation").unwrap();
        let build = arm.find("image::build").unwrap();
        let generate = arm.find("vmware::generate").unwrap();
        assert!(
            invalidate_pass < invalidate_receipt && invalidate_receipt < build && build < generate
        );

        let source = include_str!("vmware.rs");
        let generate = source.find("pub fn generate").unwrap();
        let qualify = source[generate..].find("pub(crate) fn qualify").unwrap() + generate;
        let body = &source[generate..qualify];
        let invalidate_pass = body
            .find("qualification::invalidate_vmware(paths, kernel)")
            .unwrap();
        let invalidate_receipt = body.find("invalidate_generation(paths, kernel)").unwrap();
        let preflight = body.find("qualification::vmrun").unwrap();
        let reset = body.find("write_new(&serial_log, &[])").unwrap();
        let receipt = body
            .find("qualification::commit_vmware_generation")
            .unwrap();
        assert!(
            invalidate_pass < invalidate_receipt
                && invalidate_receipt < preflight
                && preflight < reset
                && reset < receipt
        );
    }

    #[test]
    fn geometry_rounds_cylinders_up() {
        // Ceiling division keeps the geometry large enough for the final
        // partial cylinder.
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
        assert!(vmdk.contains(r#"RW 131072 FLAT "bedrock-flat.img" 0"#));
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
        let serial = format!("serial-{}.log", "1".repeat(64));
        let vmx = render(
            VMX_TEMPLATE,
            &[
                ("DISPLAY_NAME", DISPLAY_NAME.to_string()),
                ("VMDK_NAME", VMDK_NAME.to_string()),
                ("NVME_VMDK_NAME", NVME_VMDK_NAME.to_string()),
                (
                    "DMA_HANDOFF_FILE",
                    crate::platform_acpi::DMA_HANDOFF_FILE.to_string(),
                ),
                ("SERIAL_LOG", serial.clone()),
            ],
        )
        .unwrap();

        assert!(vmx.contains(r#"firmware = "efi""#));
        // Storage fails closed on a machine with no remapping unit (D-357).
        assert!(vmx.contains(r#"vvtd.enable = "TRUE""#));
        assert!(vmx.contains(r#"acpi.addtable.filename = "dma-handoff.bin""#));
        assert!(vmx.contains(&format!(r#"serial0.fileName = "{serial}""#)));
        assert!(vmx.contains(r#"sata0:0.fileName = "bedrock.vmdk""#));
        // A namespace-less candidate before the usable controller makes the
        // retirement path observable on VMware (D-176, D-240).
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
