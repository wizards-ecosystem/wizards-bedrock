//! Locating and launching QEMU.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::paths::Paths;

/// Default install location on Windows, checked before `PATH`.
const WINDOWS_DEFAULT: &str = r"C:\Program Files\qemu";

/// Subdirectories of the QEMU installation that may hold firmware images.
const FIRMWARE_DIRS: &[&str] = &["share", "share/qemu", "share/OVMF", "."];

/// The UEFI firmware image, mapped read-only.
const CODE_FIRMWARE: &str = "edk2-x86_64-code.fd";

/// Template for the writable UEFI variable store. The i386 naming is genuine
/// QEMU packaging, not a mistake.
const VARS_FIRMWARE: &str = "edk2-i386-vars.fd";

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
        None => bail!("QEMU not found — set QEMU_DIR, or put {emulator} on PATH"),
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

/// Boot `image` in QEMU, forwarding `extra` arguments verbatim.
pub fn run(paths: &Paths, image: &Path, extra: &[OsString]) -> Result<()> {
    let qemu_dir = locate()?;
    let code = firmware(&qemu_dir, CODE_FIRMWARE)?;

    // The variable store must be writable, and starting from the pristine
    // template every run keeps boots reproducible.
    let vars_template = firmware(&qemu_dir, VARS_FIRMWARE)?;
    let vars = paths.efi_vars();
    std::fs::copy(&vars_template, &vars)
        .with_context(|| format!("copying {} to {}", vars_template.display(), vars.display()))?;

    let emulator = qemu_dir.join(exe_name("qemu-system-x86_64"));
    println!("[xtask] launching QEMU (serial on stdio, close the window to exit)");

    let status = Command::new(&emulator)
        .args(["-machine", "q35", "-cpu", "qemu64", "-m", "512M"])
        .args(["-serial", "stdio"])
        .args(drive(&format!(
            "if=pflash,format=raw,readonly=on,file={}",
            code.display()
        )))
        .args(drive(&format!(
            "if=pflash,format=raw,file={}",
            vars.display()
        )))
        .args(drive(&format!("format=raw,file={}", image.display())))
        .args(extra)
        .status()
        .with_context(|| format!("could not launch {}", emulator.display()))?;

    // A non-zero status here usually means the user closed the window, which is
    // how you quit QEMU — reporting it as a build failure would be a lie.
    if !status.success() {
        println!("[xtask] QEMU exited with {status}");
    }
    Ok(())
}

fn drive(spec: &str) -> [String; 2] {
    ["-drive".to_string(), spec.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn drive_produces_a_flag_and_its_value() {
        assert_eq!(
            drive("format=raw,file=disk.img"),
            ["-drive", "format=raw,file=disk.img"]
        );
    }
}
