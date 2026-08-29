//! Resolving and identifying tools used only by phase-close qualification.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const WINDOWS_VMWARE: &str = r"C:\Program Files (x86)\VMware\VMware Workstation";

/// One resolved executable and the version it reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tool {
    pub(crate) path: PathBuf,
    pub(crate) version: String,
}

/// The Java runtime TLC would execute, if one is available on `PATH`.
pub(crate) fn java(root: &Path) -> Result<Option<Tool>> {
    let Some(path) = find_on_path("java") else {
        return Ok(None);
    };
    let version = output(root, &path, &["-version"], ExitPolicy::Success)?;
    ensure!(
        !version.is_empty(),
        "{} printed no Java version",
        path.display()
    );
    Ok(Some(Tool { path, version }))
}

/// The VMware Workstation control executable, if it can be resolved.
pub(crate) fn vmrun(root: &Path) -> Result<Option<Tool>> {
    let name = executable_name("vmrun");
    let path = if cfg!(windows) {
        let default = PathBuf::from(WINDOWS_VMWARE).join(&name);
        default
            .is_file()
            .then_some(default)
            .or_else(|| find_on_path("vmrun"))
    } else {
        find_on_path("vmrun")
    };
    let Some(path) = path else {
        return Ok(None);
    };

    // vmrun prints its identity before usage and exits unsuccessfully when no
    // command is supplied. No VM is opened by this probe.
    let report = output(root, &path, &[], ExitPolicy::Any)?;
    let version = vmrun_version(&report)?;
    Ok(Some(Tool { path, version }))
}

fn vmrun_version(report: &str) -> Result<String> {
    let version = report
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .context("vmrun printed no version")?;
    ensure!(
        version.starts_with("vmrun version "),
        "vmrun's first line is not a version: {version}"
    );
    Ok(version.to_string())
}

fn find_on_path(stem: &str) -> Option<PathBuf> {
    let name = executable_name(stem);
    let path = std::env::var_os("PATH").unwrap_or_default();
    crate::qemu::which_in(std::env::split_paths(&path), &name)
}

fn executable_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitPolicy {
    Success,
    Any,
}

fn output(root: &Path, program: &Path, args: &[&str], policy: ExitPolicy) -> Result<String> {
    let mut child = Command::new(program)
        .current_dir(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("launching {}", program.display()))?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        if child
            .try_wait()
            .with_context(|| format!("waiting for {}", program.display()))?
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            child
                .kill()
                .with_context(|| format!("stopping timed-out {}", program.display()))?;
            child
                .wait()
                .with_context(|| format!("reaping timed-out {}", program.display()))?;
            bail!(
                "{} did not report its version within 5 seconds",
                program.display()
            );
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    let result = child
        .wait_with_output()
        .with_context(|| format!("collecting output from {}", program.display()))?;
    ensure!(
        policy == ExitPolicy::Any || result.status.success(),
        "{} {} failed ({})",
        program.display(),
        args.join(" "),
        result.status
    );
    let stdout = String::from_utf8(result.stdout)
        .with_context(|| format!("{} output is not UTF-8", program.display()))?;
    let stderr = String::from_utf8(result.stderr)
        .with_context(|| format!("{} diagnostic output is not UTF-8", program.display()))?;
    let mut combined = normalize(&stdout);
    let diagnostic = normalize(&stderr);
    if !diagnostic.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&diagnostic);
    }
    Ok(combined)
}

fn normalize(text: &str) -> String {
    text.trim().replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vmrun_version_is_separated_from_its_usage_text() {
        let report = "\r\nvmrun version 1.17.0 build-24583834\r\n\r\nUsage: vmrun COMMAND\r\n";
        assert_eq!(
            vmrun_version(report).unwrap(),
            "vmrun version 1.17.0 build-24583834"
        );
        assert!(vmrun_version("Usage: vmrun COMMAND").is_err());
    }
}
