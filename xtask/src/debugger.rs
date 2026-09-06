//! The pinned native-Windows kernel debugger workflow.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};

use crate::cargo::Profile;
use crate::cli::DebugMode;
use crate::image::Kernel;
use crate::paths::Paths;
use crate::{image, qemu};

const GDB_IDENTITY: &str = "GNU gdb (GDB for MinGW-W64 x86_64, built by Brecht Sanders, r4) 17.2";
const WINGET_PACKAGE_PREFIX: &str = "BrechtSanders.WinLibs.POSIX.UCRT_";
const ENTRY_SYMBOL: &str = "kmain";
const PROOF_MARKER: &str = "wizard-debug: kernel-entry-hit";
const IDENTITY_TIMEOUT: Duration = Duration::from_secs(5);
const PROOF_TIMEOUT: Duration = Duration::from_secs(45);

pub(crate) fn run(paths: &Paths, mode: DebugMode) -> Result<()> {
    let gdb = Debugger::locate()?;
    println!("[xtask] debugger: {}", gdb.identity);
    println!("[xtask] executable: {}", gdb.executable.display());

    let image = image::build_for_run(paths, Profile::Dev, Kernel::Shipped)?;
    let elf = paths.kernel_elf(Profile::Dev);
    let mut machine = qemu::DebugMachine::start(paths, image.path())?;
    println!(
        "[xtask] paused private QEMU machine; GDB endpoint {}",
        machine.endpoint()
    );

    let result = match mode {
        DebugMode::Interactive => gdb.interactive(&elf, machine.endpoint()),
        DebugMode::Check => gdb.check(&elf, machine.endpoint()),
    };
    let cleanup = machine.stop();
    combine_with_cleanup(result, cleanup)
}

fn combine_with_cleanup(result: Result<()>, cleanup: Result<()>) -> Result<()> {
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(cleanup)) => Err(cleanup.context("stopping the debugger's QEMU machine")),
        (Err(error), Err(cleanup)) => {
            Err(anyhow!("{error:#}; QEMU cleanup also failed: {cleanup:#}"))
        }
    }
}

struct Debugger {
    executable: PathBuf,
    identity: String,
}

impl Debugger {
    fn locate() -> Result<Self> {
        if let Some(executable) = std::env::var_os("GDB").map(PathBuf::from) {
            return Self::at(executable).context("validating GDB");
        }

        let executable_name = qemu::exe_name("gdb");
        let path = std::env::var_os("PATH").unwrap_or_default();
        if let Some(executable) = qemu::which_in(std::env::split_paths(&path), &executable_name) {
            return Self::at(executable).context("validating GDB found on PATH");
        }

        for executable in winget_candidates()? {
            if let Ok(debugger) = Self::at(executable) {
                return Ok(debugger);
            }
        }

        bail!(
            "the pinned native GDB is unavailable; install it with `winget install --id BrechtSanders.WinLibs.POSIX.UCRT --version 16.1.0-14.0.0-r4 --exact`, open a fresh terminal, or set GDB"
        )
    }

    fn at(executable: PathBuf) -> Result<Self> {
        if !executable.is_file() {
            bail!("{} is not a file", executable.display());
        }
        let mut command = Command::new(&executable);
        command.arg("--version");
        let output = bounded_identity_output(command, IDENTITY_TIMEOUT)
            .with_context(|| format!("running {} --version", executable.display()))?;
        if !output.status.success() {
            bail!(
                "{} --version failed ({})",
                executable.display(),
                output.status
            );
        }
        let stdout = String::from_utf8(output.stdout).context("reading GDB's version as UTF-8")?;
        let identity = require_identity(&stdout)?.to_string();
        Ok(Self {
            executable,
            identity,
        })
    }

    fn interactive(&self, elf: &Path, endpoint: &str) -> Result<()> {
        println!("[xtask] GDB will stop at {ENTRY_SYMBOL}; use `quit` to end the VM");
        let status = Command::new(&self.executable)
            .args(gdb_arguments(elf, endpoint, DebugMode::Interactive))
            .status()
            .with_context(|| format!("launching {}", self.executable.display()))?;
        if !status.success() {
            bail!("GDB exited unsuccessfully ({status})");
        }
        Ok(())
    }

    fn check(&self, elf: &Path, endpoint: &str) -> Result<()> {
        let mut command = Command::new(&self.executable);
        command.args(gdb_arguments(elf, endpoint, DebugMode::Check));
        let output = bounded_output(command, PROOF_TIMEOUT)?;
        std::io::stdout()
            .write_all(&output.stdout)
            .context("forwarding GDB's proof output")?;
        std::io::stderr()
            .write_all(&output.stderr)
            .context("forwarding GDB's proof diagnostics")?;
        if !output.status.success() {
            bail!("GDB proof failed ({})", output.status);
        }

        let mut transcript =
            String::from_utf8(output.stdout).context("reading GDB's proof output as UTF-8")?;
        transcript.push_str(
            &String::from_utf8(output.stderr)
                .context("reading GDB's proof diagnostics as UTF-8")?,
        );
        require_proof(&transcript)?;
        println!("[xtask] debugger proof passed");
        Ok(())
    }
}

fn require_identity(stdout: &str) -> Result<&str> {
    let identity =
        core::iter::Iterator::next(&mut stdout.lines()).context("GDB printed no identity")?;
    if identity != GDB_IDENTITY {
        bail!("unsupported GDB identity `{identity}`; expected `{GDB_IDENTITY}`");
    }
    Ok(identity)
}

fn winget_candidates() -> Result<Vec<PathBuf>> {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return Ok(Vec::new());
    };
    let packages = local.join("Microsoft").join("WinGet").join("Packages");
    if !packages.is_dir() {
        return Ok(Vec::new());
    }

    let mut candidates = Vec::new();
    for entry in
        std::fs::read_dir(&packages).with_context(|| format!("reading {}", packages.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry under {}", packages.display()))?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(WINGET_PACKAGE_PREFIX)
        {
            let candidate = entry
                .path()
                .join("mingw64")
                .join("bin")
                .join(qemu::exe_name("gdb"));
            if candidate.is_file() {
                candidates.push(candidate);
            }
        }
    }
    candidates.sort();
    candidates.reverse();
    Ok(candidates)
}

fn gdb_arguments(elf: &Path, endpoint: &str, mode: DebugMode) -> Vec<OsString> {
    let mut arguments = ["--quiet", "--nx", "--nh"]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    if mode == DebugMode::Check {
        arguments.push(OsString::from("--batch"));
    }
    arguments.extend([OsString::from("--se"), elf.as_os_str().to_owned()]);
    for command in [
        "set pagination off".to_string(),
        "set confirm off".to_string(),
        format!("target remote {endpoint}"),
        format!("hbreak {ENTRY_SYMBOL}"),
        "continue".to_string(),
    ] {
        arguments.extend([OsString::from("--ex"), OsString::from(command)]);
    }
    if mode == DebugMode::Check {
        for command in [
            format!("printf \"{PROOF_MARKER}\\n\""),
            "backtrace".to_string(),
            "detach".to_string(),
            "quit".to_string(),
        ] {
            arguments.extend([OsString::from("--ex"), OsString::from(command)]);
        }
    }
    arguments
}

fn bounded_output(command: Command, timeout: Duration) -> Result<Output> {
    crate::bounded_process::output(command, timeout, "GDB proof")
}

fn bounded_identity_output(command: Command, timeout: Duration) -> Result<Output> {
    crate::bounded_process::output(command, timeout, "GDB identity probe")
}

fn require_proof(transcript: &str) -> Result<()> {
    if transcript.matches(PROOF_MARKER).count() != 1 {
        bail!("GDB did not emit exactly one kernel-entry marker");
    }
    let normalized = transcript.replace('\\', "/");
    let frame = normalized
        .lines()
        .find(|line| line.trim_start().starts_with("#0"))
        .context("GDB produced no frame zero")?;
    if !frame.contains("wizard_kernel::kmain") || !frame.contains("wost/src/boot/mod.rs") {
        bail!("GDB frame zero was not the symbolized kernel entry: {frame}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_reviewed_native_gdb_identity_is_accepted() {
        let version = format!("{GDB_IDENTITY}\nCopyright (C) 2025 Free Software Foundation\n");
        assert_eq!(require_identity(&version).unwrap(), GDB_IDENTITY);
        assert!(require_identity("").is_err());
        assert!(require_identity("GNU gdb (GDB) 17.2\n").is_err());
        assert!(require_identity(&version.replace("17.2", "17.3")).is_err());
    }

    #[test]
    fn the_debugger_commands_are_entry_first_and_the_proof_is_bounded() {
        let elf = Path::new("kernel symbols.elf");
        let interactive = gdb_arguments(elf, "127.0.0.1:1234", DebugMode::Interactive);
        let check = gdb_arguments(elf, "127.0.0.1:1234", DebugMode::Check);
        let render = |arguments: &[OsString]| {
            arguments
                .iter()
                .map(|argument| argument.to_string_lossy())
                .collect::<Vec<_>>()
                .join("|")
        };
        let interactive = render(&interactive);
        let check = render(&check);

        for commands in [&interactive, &check] {
            assert!(commands.contains("--se|kernel symbols.elf"));
            assert!(commands.contains("target remote 127.0.0.1:1234"));
            assert!(commands.contains("hbreak kmain|--ex|continue"));
        }
        assert!(!interactive.contains("--batch"));
        assert!(!interactive.contains(PROOF_MARKER));
        assert!(check.contains("--batch"));
        assert!(check.contains(PROOF_MARKER));
        assert!(check.contains("backtrace|--ex|detach|--ex|quit"));
        assert!(IDENTITY_TIMEOUT <= Duration::from_secs(10));
        assert!(PROOF_TIMEOUT >= Duration::from_secs(10));
        assert!(PROOF_TIMEOUT <= Duration::from_secs(60));
    }

    #[test]
    #[ignore]
    fn slow_identity_child() {
        std::thread::sleep(Duration::from_secs(2));
    }

    #[test]
    fn the_gdb_identity_probe_has_a_direct_child_return_deadline() {
        let mut command = Command::new(std::env::current_exe().expect("current test executable"));
        command.args([
            "--exact",
            "debugger::tests::slow_identity_child",
            "--ignored",
        ]);
        let started = Instant::now();
        let error = bounded_identity_output(command, Duration::from_millis(100)).unwrap_err();

        assert!(format!("{error:#}").contains("GDB identity probe timed out after 100 ms"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn the_debugger_proof_requires_the_entry_marker_symbol_and_source() {
        let proof = "Breakpoint 1, wizard_kernel::kmain () at wost\\src\\boot/mod.rs:526\n\
                     wizard-debug: kernel-entry-hit\n\
                     #0  wizard_kernel::kmain () at wost\\src\\boot/mod.rs:526\n";
        require_proof(proof).unwrap();

        assert!(require_proof(&proof.replace(PROOF_MARKER, "wrong marker")).is_err());
        assert!(require_proof(&proof.replace("#0", "#1")).is_err());
        assert!(require_proof(&proof.replace("wizard_kernel::kmain", "kmain")).is_err());
        assert!(require_proof(&proof.replace("wost\\src\\boot/mod.rs", "unknown.rs")).is_err());
        assert!(require_proof(&format!("{proof}{PROOF_MARKER}\n")).is_err());
    }
}
