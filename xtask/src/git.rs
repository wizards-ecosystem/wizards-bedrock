//! The one Git executable a run trusts with source identity (D-344).
//!
//! Git answers whether the checkout is clean and which commit and tree it is.
//! A run resolves Git once, spawns that exact file for every query, and each
//! record names the bytes that answered, so a substituted executable is
//! visible in the evidence and cannot change during a run.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};

const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_VERSION_BYTES: usize = 4096;
const VERSION_TIMEOUT: Duration = Duration::from_secs(30);
const CONFIG_QUERY_BYTES: usize = 256 * 1024;
const CONFIG_QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_FILTER_DRIVERS: usize = 128;
const MAX_FILTER_OVERRIDE_BYTES: usize = 16 * 1024;
const REFUSED_CONFIGURATION_OPTION: &str = "--wizard-refused-unsafe-repository-configuration";
#[cfg(windows)]
const DISABLED_GIT_PATH: &str = "NUL";
#[cfg(not(windows))]
const DISABLED_GIT_PATH: &str = "/dev/null";

/// The record keys that name the Git executable behind a source identity.
pub(crate) const FIELDS: [&str; 4] = [
    "tool.git.bytes",
    "tool.git.path",
    "tool.git.sha256",
    "tool.git.version",
];

static RESOLVED: OnceLock<Result<Git, String>> = OnceLock::new();

/// One resolved Git executable and the identity it had when resolved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Git {
    path: PathBuf,
    version: String,
    bytes: u64,
    sha256: String,
}

/// The Git this run resolved from `PATH` at its first source query.
pub(crate) fn resolved() -> Result<&'static Git> {
    RESOLVED
        .get_or_init(|| {
            resolve(&std::env::var_os("PATH").unwrap_or_default())
                .map_err(|error| format!("{error:#}"))
        })
        .as_ref()
        .map_err(|error| anyhow!("resolving Git: {error}"))
}

fn resolve(search: &OsStr) -> Result<Git> {
    let name = format!("git{}", std::env::consts::EXE_SUFFIX);
    let found = crate::qemu::which_in(std::env::split_paths(search), &name)
        .context("no Git executable is on PATH")?;
    let path = std::fs::canonicalize(&found)
        .with_context(|| format!("canonicalizing Git {}", found.display()))?;
    identify(path)
}

fn identify(path: PathBuf) -> Result<Git> {
    let (bytes, sha256) = measure(&path)?;
    let mut command = Command::new(&path);
    crate::bounded_process::isolate_git_environment(&mut command);
    command
        .current_dir(
            path.parent()
                .context("resolved Git executable has no parent directory")?,
        )
        .arg("--version");
    let output = crate::bounded_process::output_with_limit(
        command,
        VERSION_TIMEOUT,
        "Git version probe",
        MAX_VERSION_BYTES,
    )?;
    ensure!(
        output.status.success(),
        "Git version probe failed ({})",
        output.status
    );
    let version = String::from_utf8(output.stdout)
        .context("Git version is not UTF-8")?
        .trim()
        .to_string();
    ensure!(
        version.starts_with("git version ") && !version.contains(['\r', '\n', '\0']),
        "Git printed no single-line version identity"
    );
    Ok(Git {
        path,
        version,
        bytes,
        sha256,
    })
}

fn measure(path: &Path) -> Result<(u64, String)> {
    let bytes =
        crate::qualification::read_bounded_file(path, MAX_EXECUTABLE_BYTES, "Git executable")?;
    Ok((
        bytes.len() as u64,
        crate::sha256::hex(&crate::sha256::digest(&bytes)),
    ))
}

impl Git {
    /// The canonical executable path used for every query in this run.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// A Git-defined query in `root` spawned from this exact executable.
    pub(crate) fn command(&self, root: &Path) -> Command {
        let Ok(root) = absolute_root(root) else {
            return self.refused_command();
        };
        match self.filter_drivers(&root) {
            Ok(drivers) => self.command_with_filters_disabled(&root, &drivers),
            Err(_) => self.refused_command(),
        }
    }

    fn base_command(&self, root: &Path) -> Command {
        debug_assert!(root.is_absolute());
        let mut git_dir = OsString::from("--git-dir=");
        git_dir.push(root.join(".git"));
        let mut work_tree = OsString::from("--work-tree=");
        work_tree.push(root);
        let mut command = Command::new(&self.path);
        crate::bounded_process::isolate_git_environment(&mut command);
        command
            .current_dir(self.path.parent().unwrap_or(&self.path))
            .args([
                "--no-pager",
                "--literal-pathspecs",
                "--no-replace-objects",
                "--no-optional-locks",
            ])
            .arg(git_dir)
            .arg(work_tree)
            .args([
                "-c",
                "core.fsmonitor=",
                "-c",
                &format!("core.hooksPath={DISABLED_GIT_PATH}"),
                "-c",
                &format!("core.attributesFile={DISABLED_GIT_PATH}"),
                "-c",
                &format!("core.excludesFile={DISABLED_GIT_PATH}"),
                "-c",
                "core.untrackedCache=false",
                "-c",
                "core.pager=",
                "-c",
                "diff.external=",
                "-c",
                "diff.trustExitCode=false",
                "-c",
                "interactive.diffFilter=",
                "-c",
                "credential.helper=",
            ]);
        command
    }

    fn filter_configuration(&self, root: &Path) -> Result<Vec<u8>> {
        let mut command = self.base_command(root);
        command.args([
            "config",
            "--includes",
            "--null",
            "--name-only",
            "--get-regexp",
            "^filter\\..*\\.(clean|smudge|process|required)$",
        ]);
        let output = crate::bounded_process::output_with_limit(
            command,
            CONFIG_QUERY_TIMEOUT,
            "Git filter configuration query",
            CONFIG_QUERY_BYTES,
        )?;
        if output.status.code() == Some(1) && output.stdout.is_empty() && output.stderr.is_empty() {
            return Ok(Vec::new());
        }
        ensure!(
            output.status.success() && output.stderr.is_empty(),
            "Git filter configuration query failed or wrote diagnostics ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(output.stdout)
    }

    fn filter_drivers(&self, root: &Path) -> Result<BTreeSet<String>> {
        // D-406 excludes the checkout owner's same-account rewrite between
        // this bounded discovery and the immediately returned query.
        parse_filter_drivers(&self.filter_configuration(root)?)
    }

    fn command_with_filters_disabled(&self, root: &Path, drivers: &BTreeSet<String>) -> Command {
        let mut command = self.base_command(root);
        for driver in drivers {
            for value in [
                format!("filter.{driver}.clean="),
                format!("filter.{driver}.smudge="),
                format!("filter.{driver}.process="),
                format!("filter.{driver}.required=false"),
            ] {
                command.args(["-c", &value]);
            }
        }
        command
    }

    fn refused_command(&self) -> Command {
        let mut command = Command::new(&self.path);
        crate::bounded_process::isolate_git_environment(&mut command);
        command
            .current_dir(self.path.parent().unwrap_or(&self.path))
            .args(["--no-pager", REFUSED_CONFIGURATION_OPTION]);
        command
    }

    /// Refuse to publish a record whose Git executable changed after it answered.
    pub(crate) fn require_unchanged(&self) -> Result<()> {
        ensure!(
            measure(&self.path)? == (self.bytes, self.sha256.clone()),
            "the Git executable {} changed during this run",
            self.path.display()
        );
        Ok(())
    }

    /// The identity fields, in [`FIELDS`] order.
    pub(crate) fn fields(&self) -> [(&'static str, String); 4] {
        [
            (FIELDS[0], self.bytes.to_string()),
            (FIELDS[1], self.path.to_string_lossy().into_owned()),
            (FIELDS[2], self.sha256.clone()),
            (FIELDS[3], self.version.clone()),
        ]
    }
}

fn parse_filter_drivers(bytes: &[u8]) -> Result<BTreeSet<String>> {
    let mut drivers = BTreeSet::new();
    let mut override_bytes = 0usize;
    ensure!(
        bytes.is_empty() || bytes.last() == Some(&0),
        "Git filter configuration is not NUL terminated"
    );
    if bytes.is_empty() {
        return Ok(drivers);
    }
    let entries = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    for encoded in entries.split(|byte| *byte == 0) {
        ensure!(
            !encoded.is_empty(),
            "Git filter configuration contains an empty key"
        );
        let key = std::str::from_utf8(encoded).context("Git filter name is not UTF-8")?;
        let normalized = key.to_ascii_lowercase();
        let suffix = [".clean", ".smudge", ".process", ".required"]
            .into_iter()
            .find(|suffix| normalized.ends_with(suffix))
            .context("Git returned an unexpected filter configuration key")?;
        ensure!(
            normalized.starts_with("filter.") && key.len() > "filter.".len() + suffix.len(),
            "Git returned a filter configuration without a driver name"
        );
        let driver = &key["filter.".len()..key.len() - suffix.len()];
        ensure!(
            !driver.chars().any(char::is_control) && !driver.contains('='),
            "Git returned an unsafe filter driver name"
        );
        if drivers.insert(driver.to_string()) {
            ensure!(
                drivers.len() <= MAX_FILTER_DRIVERS,
                "repository defines more than {MAX_FILTER_DRIVERS} filter drivers"
            );
            let driver_bytes = driver
                .len()
                .checked_mul(4)
                .and_then(|bytes| bytes.checked_add(80))
                .context("Git filter override length overflowed")?;
            override_bytes = override_bytes
                .checked_add(driver_bytes)
                .context("Git filter override length overflowed")?;
            ensure!(
                override_bytes <= MAX_FILTER_OVERRIDE_BYTES,
                "repository filter overrides exceed {MAX_FILTER_OVERRIDE_BYTES} bytes"
            );
        }
    }
    Ok(drivers)
}

fn absolute_root(root: &Path) -> Result<PathBuf> {
    if root.is_absolute() {
        Ok(root.to_path_buf())
    } else {
        std::env::current_dir()
            .context("reading the current directory for a relative Git root")
            .map(|current| current.join(root))
    }
}

/// Check the recorded identity of the Git that established a source.
pub(crate) fn validate_fields<'a>(get: impl Fn(&str) -> Result<&'a str>) -> Result<()> {
    let bytes = get(FIELDS[0])?;
    ensure!(
        bytes
            .parse::<u64>()
            .is_ok_and(|value| value > 0 && value.to_string() == bytes),
        "recorded Git length is not a canonical positive count"
    );
    ensure!(
        Path::new(get(FIELDS[1])?).is_absolute(),
        "recorded Git path is not absolute"
    );
    let sha256 = get(FIELDS[2])?;
    ensure!(
        sha256.len() == 64
            && sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "recorded Git digest is malformed"
    );
    ensure!(
        get(FIELDS[3])?.starts_with("git version "),
        "recorded Git version is malformed"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
