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
use crate::qemu::{SHIPPED_USER_EXECUTION_MARKER, canonical_decimal};
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
const VMWARE_ISA_DMA_MARKER: &str = "iommu: entry DMA fence quiesced VMware PIIX4 00:07.0 at its masked 8237 controllers; it remains a requester";
const SELFTEST_DELAYED_READ_MARKER: &str = "selftest: delayed read evidence deferred=true device=true retained=true polled=true completed=true exact=true";
const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;

/// Heads per cylinder in the geometry VMware expects.
const HEADS: u64 = 255;

/// Sectors per track in the same.
const SECTORS_PER_TRACK: u64 = 63;

/// A disk geometry for a VMDK descriptor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    verify_transcript_with(kernel, bytes, || Ok(()), || expected_selftest_checks(root))
}

// The count belongs to the transcript's own image, so retained captures outlive new checks.
fn verify_transcript_with<F, E>(
    kernel: Kernel,
    bytes: &[u8],
    after_nul_check: F,
    expected_checks: E,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
    E: FnOnce() -> Result<usize>,
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
        Kernel::SelfTest => verify_selftest_transcript(text, expected_checks),
        Kernel::Shipped => verify_shipped_transcript(text),
    }
}

#[derive(Clone, Copy, Debug)]
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
    TranscriptMarker::Exact(VMWARE_ISA_DMA_MARKER),
    TranscriptMarker::Exact("cpu: protections wp smep smap umip nx"),
    TranscriptMarker::ExtendedState,
    TranscriptMarker::Isolation(Kernel::SelfTest),
    TranscriptMarker::LocalApic,
    TranscriptMarker::Exact("selftest: a thread crosses into ring three and calls back"),
    TranscriptMarker::Exact("selftest: awaiting storage-delay keystroke"),
    TranscriptMarker::Exact(SELFTEST_DELAYED_READ_MARKER),
    TranscriptMarker::Exact("selftest: the kernel reads the file the host wrote"),
    TranscriptMarker::Exact("selftest: both calibrated rates are believable"),
    TranscriptMarker::Exact("selftest: awaiting injected keystroke"),
    TranscriptMarker::Exact("selftest: an injected keystroke arrives, and its release follows"),
    TranscriptMarker::Exact("selftest: awaiting translated keystroke"),
    TranscriptMarker::Exact("selftest: a translated keystroke decodes to the same key"),
];

const SHIPPED_MARKERS: &[TranscriptMarker] = &[
    TranscriptMarker::Exact(VMWARE_DMA_HANDOFF_MARKER),
    TranscriptMarker::Exact(VMWARE_ISA_DMA_MARKER),
    TranscriptMarker::Nvme,
    TranscriptMarker::Fat32,
    TranscriptMarker::Tmpfs,
    TranscriptMarker::Exact("devfs: storage is a block device"),
    TranscriptMarker::Exact("devfs: console is a character device"),
    TranscriptMarker::Isolation(Kernel::Shipped),
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

fn verify_selftest_transcript(
    text: &str,
    expected_checks: impl FnOnce() -> Result<usize>,
) -> Result<()> {
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
    let expected = expected_checks()?;
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
    require_once_in_order(
        text,
        &[TranscriptMarker::Exact(SHIPPED_USER_EXECUTION_MARKER)],
    )?;
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
        &module_sources(root.join("kernel/src/selftest.rs"))?,
        "const EXPECTED_CHECKS: usize = ",
    )?;
    let harness = source_usize(
        &module_sources(root.join("xtask/src/qemu.rs"))?,
        "const EXPECTED_SELFTEST_CHECKS: usize = ",
    )?;
    ensure!(
        kernel == harness,
        "kernel expects {kernel} checks but the harness expects {harness}"
    );
    Ok(kernel)
}

fn module_sources(file: PathBuf) -> Result<Vec<PathBuf>> {
    let mut pending = vec![file.with_extension("")];
    let mut sources = vec![file];
    while let Some(directory) = pending.pop() {
        for entry in paths::directory_entries(&directory)? {
            let path = entry.path();
            if entry.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }
    Ok(sources)
}

fn source_usize(sources: &[PathBuf], prefix: &str) -> Result<usize> {
    let mut values = Vec::new();
    for path in sources {
        let source =
            qualification::read_bounded_text(path, MAX_SOURCE_BYTES, "VMware source input")?;
        let found: Vec<usize> = source
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
        values.extend(found);
    }
    ensure!(
        values.len() == 1,
        "{} contain {} `{prefix}` constants",
        sources
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
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
    crate::mutate::workspace::prepare_real_directory_tree(&root, "VMware root")?;
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
        crate::mutate::workspace::canonical_existing_real_directory(parent, label)?;
    let canonical_child =
        crate::mutate::workspace::canonical_existing_real_directory(child, label)?;
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
    crate::mutate::workspace::canonical_existing_real_directory(parent, "VMware output parent")?;
    let mut options = OpenOptions::new();
    OpenOptions::create_new(&mut options, true);
    OpenOptions::write(&mut options, true);
    let file = OpenOptions::open(&options, path)
        .with_context(|| format!("creating new VMware output {}", path.display()))?;
    crate::mutate::workspace::require_direct_file(parent, path, "VMware output")?;
    Ok(file)
}

#[cfg(test)]
mod tests;
