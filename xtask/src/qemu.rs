mod boot;
mod selftest;
mod serial;
pub(crate) mod storage;
#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};

use crate::paths::{self, RunDirectory};
use crate::qmp::Qmp;

pub(crate) use self::boot::{boot_check, degraded_check};
pub(crate) use self::selftest::run_selftest;
pub(crate) use self::serial::Verdict;

const DEFAULT_INSTALLATION: Option<&str> = if cfg!(windows) {
    Some(r"C:\Program Files\qemu")
} else {
    None
};
const MACHINE_TYPE: &str = "pc-q35-9.2";
const FIRMWARE_DIRECTORIES: &[&str] = &["share", "share/qemu", "share/OVMF", "."];
const CODE_FIRMWARE: &str = "edk2-x86_64-code.fd";
// QEMU packages the x86_64 variable store template under its i386 name.
const VARS_FIRMWARE: &str = "edk2-i386-vars.fd";
const VARS_NAME: &str = "efi-vars.fd";
const DMA_COVERAGE_NAME: &str = "dma-coverage.bin";
const DMA_COVERAGE_BYTES: usize = 48;
const QMP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_LINE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Remapping {
    Present,
    Absent,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Control {
    Qmp,
    None,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct Installation {
    emulator: PathBuf,
    code_firmware: PathBuf,
    vars_firmware: PathBuf,
}

impl Installation {
    fn locate() -> Result<Self> {
        let emulator = std::path::absolute(emulator()?).context("resolving the QEMU path")?;
        let directory = emulator
            .parent()
            .with_context(|| format!("{} has no parent directory", emulator.display()))?;
        Ok(Self {
            code_firmware: firmware("QEMU_FIRMWARE_CODE", directory, CODE_FIRMWARE)?,
            vars_firmware: firmware("QEMU_FIRMWARE_VARS", directory, VARS_FIRMWARE)?,
            emulator,
        })
    }

    // The run directory is the working directory because `-acpitable file=` cannot escape a comma.
    fn command(&self, run: &RunDirectory, remapping: Remapping) -> Result<Command> {
        std::fs::copy(&self.vars_firmware, run.path().join(VARS_NAME))
            .with_context(|| format!("copying {}", self.vars_firmware.display()))?;
        std::fs::write(
            run.path().join(crate::platform_acpi::DMA_HANDOFF_FILE),
            crate::platform_acpi::dma_handoff_table(),
        )
        .context("writing the DMA handoff table")?;
        storage::write_namespace(&run.path().join(paths::NVME_IMAGE_NAME))?;

        let code = self.code_firmware.to_string_lossy().replace(',', ",,");
        let mut command = Command::new(&self.emulator);
        command
            .current_dir(run.path())
            .args(["-no-user-config", "-accel", "tcg", "-machine", MACHINE_TYPE])
            // The hypervisor leaf authenticates the otherwise writable ACPI handoff bytes.
            .args(["-cpu", "max,hypervisor=on,tcg-cpuid=on", "-m", "512M"]);
        if remapping == Remapping::Present {
            // Caching mode caches not-present DMA entries, so every mapping change is invalidated.
            command.args(["-device", "intel-iommu,intremap=on,eim=on,caching-mode=on"]);
            std::fs::write(run.path().join(DMA_COVERAGE_NAME), dma_coverage_table())
                .context("writing the DMA coverage table")?;
            // QEMU 9.2 emits a scoped DRHD though its unit sees every request; this names it.
            command.args(["-acpitable", &format!("file={DMA_COVERAGE_NAME}")]);
        }
        // The IOMMU precedes the bridge and storage; the bridge fails a bus-zero-only scan.
        command
            .args(["-device", "pci-bridge,id=behind,chassis_nr=1"])
            .args([
                "-acpitable",
                &format!("file={}", crate::platform_acpi::DMA_HANDOFF_FILE),
            ])
            .args(["-drive", &format!("if=pflash,format=raw,readonly=on,file={code}")])
            .args(["-drive", &format!("if=pflash,format=raw,file={VARS_NAME}")])
            .args(["-drive", &format!("format=raw,file={}", paths::IMAGE_NAME)])
            // The first controller has no namespace, so the kernel must retire it for its sibling.
            .args(["-device", "nvme,serial=wizard-empty"])
            .args([
                "-drive",
                &format!("if=none,id=wizardnvm,format=raw,file={}", paths::NVME_IMAGE_NAME),
            ])
            .args(["-device", "nvme,serial=wizard0,drive=wizardnvm"]);
        Ok(command)
    }
}

fn emulator() -> Result<PathBuf> {
    let name = format!("qemu-system-x86_64{}", std::env::consts::EXE_SUFFIX);
    if let Some(path) = std::env::var_os("QEMU").map(PathBuf::from) {
        ensure!(
            path.is_file(),
            "QEMU names {}, which is not a file",
            path.display()
        );
        return Ok(path);
    }
    if let Some(directory) = std::env::var_os("QEMU_DIR").map(PathBuf::from) {
        let path = directory.join(&name);
        ensure!(path.is_file(), "QEMU_DIR holds no {}", path.display());
        return Ok(path);
    }
    let search = std::env::var_os("PATH").unwrap_or_default();
    DEFAULT_INSTALLATION
        .map(PathBuf::from)
        .into_iter()
        .chain(std::env::split_paths(&search))
        .map(|directory| directory.join(&name))
        .find(|candidate| candidate.is_file())
        .with_context(|| {
            format!("{name} is not on PATH; set QEMU to it or QEMU_DIR to its directory")
        })
}

// An FHS install keeps the emulator in `<prefix>/bin` and its firmware under `<prefix>/share`.
fn firmware(variable: &str, emulator_directory: &Path, name: &str) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(variable).map(PathBuf::from) {
        ensure!(
            path.is_file(),
            "{variable} names {}, which is not a file",
            path.display()
        );
        return Ok(path);
    }
    let prefix = emulator_directory
        .parent()
        .filter(|_| emulator_directory.ends_with("bin"));
    std::iter::once(emulator_directory)
        .chain(prefix)
        .flat_map(|root| {
            FIRMWARE_DIRECTORIES
                .iter()
                .map(move |directory| root.join(directory))
        })
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .with_context(|| {
            format!(
                "UEFI firmware {name} is not beside {}; set {variable} to it",
                emulator_directory.display()
            )
        })
}

fn dma_coverage_table() -> [u8; DMA_COVERAGE_BYTES] {
    let mut table = [0u8; DMA_COVERAGE_BYTES];
    table[0..4].copy_from_slice(b"WZDM");
    table[4..8].copy_from_slice(&(DMA_COVERAGE_BYTES as u32).to_le_bytes());
    table[8] = 1;
    table[10..16].copy_from_slice(b"WIZARD");
    table[16..24].copy_from_slice(b"BEDROCK ");
    table[24..28].copy_from_slice(&1u32.to_le_bytes());
    table[28..32].copy_from_slice(b"WZRD");
    table[32..36].copy_from_slice(&1u32.to_le_bytes());
    table[40..48].copy_from_slice(&0xfed9_0000u64.to_le_bytes());
    table[9] = 0u8.wrapping_sub(table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
    table
}

// QEMU's status is printed only: a non-zero one usually means the user closed the window.
pub(crate) fn run(run: &RunDirectory, extra: &[OsString]) -> Result<()> {
    let installation = Installation::locate()?;
    println!("[xtask] launching QEMU (serial on stdio, close the window to exit)");
    let status = installation
        .command(run, Remapping::Present)?
        .args(["-serial", "stdio"])
        .args(extra)
        .status()
        .with_context(|| format!("could not launch {}", installation.emulator.display()))?;
    if !status.success() {
        println!("[xtask] QEMU exited with {status}");
    }
    Ok(())
}

pub(crate) fn debug(run: &RunDirectory, port: u16) -> Result<Child> {
    let installation = Installation::locate()?;
    installation
        .command(run, Remapping::Present)?
        .args(["-S", "-gdb", &format!("tcp:127.0.0.1:{port}")])
        .args(["-display", "none", "-serial", "null", "-no-reboot"])
        .spawn()
        .with_context(|| format!("could not launch {}", installation.emulator.display()))
}

#[derive(Debug)]
enum Event {
    Line(String),
    Exited(ExitStatus),
    TimedOut,
}

#[derive(Debug)]
struct Machine {
    child: Child,
    lines: Receiver<Result<String>>,
    qmp: Option<Qmp>,
}

impl Machine {
    fn launch(mut command: Command, control: Control) -> Result<Self> {
        let listener = match control {
            Control::Qmp => Some(TcpListener::bind("127.0.0.1:0").context("binding QMP")?),
            Control::None => None,
        };
        if let Some(listener) = &listener {
            let port = listener
                .local_addr()
                .context("reading the QMP port")?
                .port();
            command.args(["-qmp", &format!("tcp:127.0.0.1:{port}")]);
        }
        let mut child = command
            .args(["-display", "none", "-no-reboot", "-serial", "stdio"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .context("could not launch QEMU")?;
        let stdout = child.stdout.take().context("taking QEMU's stdout")?;
        let mut machine = Self {
            child,
            lines: read_lines(stdout),
            qmp: None,
        };
        if let Some(listener) = listener {
            let deadline = Instant::now() + QMP_TIMEOUT;
            machine.qmp = Some(Qmp::accept(&listener, &mut machine.child, deadline)?);
        }
        Ok(machine)
    }

    fn next(&mut self, deadline: Instant) -> Result<Event> {
        let wait = deadline.saturating_duration_since(Instant::now());
        match self.lines.recv_timeout(wait) {
            Ok(line) => line.map(Event::Line),
            Err(RecvTimeoutError::Timeout) => Ok(Event::TimedOut),
            Err(RecvTimeoutError::Disconnected) => {
                let status = self.child.wait().context("waiting for QEMU")?;
                Ok(Event::Exited(status))
            }
        }
    }

    fn qmp(&mut self) -> Result<&mut Qmp> {
        self.qmp.as_mut().context("this machine has no QMP session")
    }

    // Registers are the only hang evidence when the serial log is empty.
    fn report_hang(&mut self) {
        let Ok(qmp) = self.qmp() else {
            return;
        };
        let Ok(state) = qmp.human("info registers") else {
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
        if let Ok(code) = qmp.human(&format!("x/8i 0x{rip}")) {
            for line in code
                .lines()
                .filter(|line| line.trim_start().starts_with("0x"))
            {
                println!("[xtask] hung in:  {}", line.trim());
            }
        }
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_lines(output: impl Read + Send + 'static) -> Receiver<Result<String>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(output);
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = (&mut reader)
                .take(MAX_LINE_BYTES)
                .read_until(b'\n', &mut line);
            match read {
                Ok(0) => return,
                Ok(_) if line.ends_with(b"\n") => {
                    println!("{}", serial::escaped(&line));
                    let text = String::from_utf8_lossy(&line);
                    let text = text.trim_end_matches(['\r', '\n']).to_string();
                    if sender.send(Ok(text)).is_err() {
                        return;
                    }
                }
                Ok(_) if line.len() as u64 == MAX_LINE_BYTES => {
                    let _ = sender.send(Err(anyhow::anyhow!("a serial line exceeded its limit")));
                    return;
                }
                Ok(_) => println!("{}", serial::escaped(&line)),
                Err(error) => {
                    let _ = sender.send(Err(error).context("reading QEMU's serial output"));
                    return;
                }
            }
        }
    });
    receiver
}
