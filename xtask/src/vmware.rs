use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use boot_record::{Exit, Fact, Record};
use native_abi::witness;
use panic_record::OPEN_PREFIX;

use crate::cargo::Profile;
use crate::image::{self, Kernel};
use crate::paths::{self, Paths, RunDirectory};
use crate::qemu::read_record;
use crate::qemu::storage::{NAMESPACE_BYTES, write_namespace};
use crate::sha256;

const VMDK_TEMPLATE: &str = include_str!("../templates/bedrock.vmdk.in");
const VMX_TEMPLATE: &str = include_str!("../templates/bedrock.vmx.in");
const FLAT_NAME: &str = "bedrock-flat.img";
const VMDK_NAME: &str = "bedrock.vmdk";
const NVME_VMDK_NAME: &str = "wizard-nvm.vmdk";
const NVME_FLAT_NAME: &str = "wizard-nvm-flat.img";
const VMX_NAME: &str = "bedrock.vmx";
const DISPLAY_NAME: &str = "The Wizard's Bedrock";
const SERIAL_PREFIX: &str = "serial-";
const SERIAL_SUFFIX: &str = ".log";
const HEADS: u64 = 255;
const SECTORS_PER_TRACK: u64 = 63;

pub(crate) fn generate(paths: &Paths, kernel: Kernel) -> Result<()> {
    let directory = vm_directory(paths, kernel);
    if directory.exists() {
        std::fs::remove_dir_all(&directory)
            .with_context(|| format!("resetting {}", directory.display()))?;
    }
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("creating {}", directory.display()))?;

    let flat = directory.join(FLAT_NAME);
    image::build(paths, Profile::Release, kernel, &flat)?;
    let (bytes, digest) = digest(&flat)?;
    write_descriptor(&directory.join(VMDK_NAME), FLAT_NAME, bytes)?;
    write_namespace(&directory.join(NVME_FLAT_NAME))?;
    write_descriptor(
        &directory.join(NVME_VMDK_NAME),
        NVME_FLAT_NAME,
        NAMESPACE_BYTES,
    )?;
    write(
        &directory.join(crate::platform_acpi::DMA_HANDOFF_FILE),
        &crate::platform_acpi::dma_handoff_table(),
    )?;

    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("the host clock predates the Unix epoch")?;
    let serial = format!(
        "{SERIAL_PREFIX}{:x}-{}{SERIAL_SUFFIX}",
        elapsed.as_nanos(),
        std::process::id()
    );
    let display_name = match kernel {
        Kernel::SelfTest => DISPLAY_NAME.to_string(),
        Kernel::Shipped => format!("{DISPLAY_NAME} (shipped)"),
    };
    let vmx = render(
        VMX_TEMPLATE,
        &[
            ("DISPLAY_NAME", display_name),
            ("VMDK_NAME", VMDK_NAME.to_string()),
            ("NVME_VMDK_NAME", NVME_VMDK_NAME.to_string()),
            (
                "DMA_HANDOFF_FILE",
                crate::platform_acpi::DMA_HANDOFF_FILE.to_string(),
            ),
            ("SERIAL_LOG", serial.clone()),
        ],
    )?;
    let vmx_path = directory.join(VMX_NAME);
    write(&vmx_path, vmx.as_bytes())?;
    let serial_log = directory.join(&serial);
    write(&serial_log, &[])?;

    println!("[xtask] VMware VM ready: {}", vmx_path.display());
    println!(
        "[xtask] VMware {} image: {bytes} bytes, sha256 {}",
        kernel.label(),
        sha256::hex(&digest)
    );
    println!("[xtask] open it in VMware Workstation");
    println!(
        "[xtask] serial output will appear in {}",
        serial_log.display()
    );
    print_checklist(kernel);
    Ok(())
}

pub(crate) fn qualify(paths: &Paths, kernel: Kernel) -> Result<()> {
    let directory = vm_directory(paths, kernel);
    let run = RunDirectory::create(paths)?;
    image::build(paths, Profile::Release, kernel, &run.image())?;
    ensure!(
        digest(&directory.join(FLAT_NAME))? == digest(&run.image())?,
        "the VMware boot image differs from a fresh build"
    );
    let transcript = transcript_path(&directory)?;
    let text = std::fs::read_to_string(&transcript)
        .with_context(|| format!("reading {}", transcript.display()))?
        .replace("\r\n", "\n");
    ensure!(
        !text.contains(OPEN_PREFIX),
        "the VMware transcript contains a kernel panic"
    );
    let records = text
        .lines()
        .filter_map(|line| read_record(line).transpose())
        .collect::<Result<Vec<_>>>()?;
    match kernel {
        Kernel::SelfTest => {
            let verdicts = records
                .iter()
                .filter_map(|record| match *record {
                    Record::Verdict(verdict) => Some(verdict),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let [verdict] = verdicts.as_slice() else {
                bail!(
                    "the VMware transcript has {} self-test verdicts",
                    verdicts.len()
                );
            };
            ensure!(
                verdict.is_clean_pass(),
                "the VMware self-test verdict passed {} of {} checks with {} failures",
                verdict.passed,
                verdict.expected,
                verdict.failed
            );
        }
        Kernel::Shipped => {
            for record in [
                Record::Fact(Fact::Settled),
                Record::Exit(Exit {
                    status: witness::AGREED,
                }),
            ] {
                ensure!(
                    records.contains(&record),
                    "the VMware transcript never writes `{record}`"
                );
            }
        }
    }
    println!(
        "[xtask] VMware {} transcript {} verifies",
        kernel.label(),
        transcript.display()
    );
    Ok(())
}

// Substituted text is never rescanned, and an unknown placeholder is an error.
fn render(template: &str, values: &[(&str, String)]) -> Result<String> {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        rendered.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .context("unterminated placeholder in template")?;
        let key = &after[..end];
        let value = values
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value)
            .with_context(|| format!("template names an unknown placeholder: {key}"))?;
        rendered.push_str(value);
        rest = &after[end + 2..];
    }
    rendered.push_str(rest);
    Ok(rendered)
}

// Cylinders round up, since truncating would declare less than the extent it describes.
fn write_descriptor(path: &Path, flat_name: &str, bytes: u64) -> Result<()> {
    let sectors = bytes / paths::SECTOR_SIZE;
    let cylinders = sectors.div_ceil(HEADS * SECTORS_PER_TRACK);
    let vmdk = render(
        VMDK_TEMPLATE,
        &[
            ("SECTORS", sectors.to_string()),
            ("FLAT_NAME", flat_name.to_string()),
            ("CYLINDERS", cylinders.to_string()),
            ("HEADS", HEADS.to_string()),
            ("SECTORS_PER_TRACK", SECTORS_PER_TRACK.to_string()),
        ],
    )?;
    write(path, vmdk.as_bytes())
}

fn print_checklist(kernel: Kernel) {
    match kernel {
        Kernel::SelfTest => {
            println!("[xtask] this is the self-test kernel; it needs no input:");
            println!("        look for `record v4 verdict` in the fresh serial log");
        }
        Kernel::Shipped => {
            println!("[xtask] this is the shipped kernel, verify its fresh serial log:");
            println!("        1. storage reaches `nvme: up` and `fat32: mounted`");
            println!("        2. tmpfs and devfs complete their namespace survey");
            println!("        3. ring 3 launches and `kernel: reading the keyboard` settles");
            println!("        4. no `KERNEL PANIC` appears");
        }
    }
}

fn transcript_path(directory: &Path) -> Result<PathBuf> {
    let entries =
        std::fs::read_dir(directory).with_context(|| format!("listing {}", directory.display()))?;
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(SERIAL_PREFIX) && name.ends_with(SERIAL_SUFFIX) {
            found.push(entry.path());
        }
    }
    match found.as_slice() {
        [transcript] => Ok(transcript.clone()),
        _ => bail!(
            "{} holds {} generated serial logs, expected one",
            directory.display(),
            found.len()
        ),
    }
}

fn vm_directory(paths: &Paths, kernel: Kernel) -> PathBuf {
    match kernel {
        Kernel::SelfTest => paths.vm_dir().join("self-test"),
        Kernel::Shipped => paths.vm_dir().join("shipped"),
    }
}

fn digest(path: &Path) -> Result<(u64, [u8; 32])> {
    File::open(path)
        .and_then(sha256::digest_reader)
        .with_context(|| format!("hashing {}", path.display()))
}

fn write(path: &Path, contents: &[u8]) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}
