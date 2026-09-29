//! The one Git executable a run trusts with source identity (D-344).
//!
//! Git answers whether the checkout is clean and which commit and tree it is.
//! A run resolves Git once, spawns that exact file for every query, and each
//! record names the bytes that answered, so a substituted executable is
//! visible in the evidence and cannot change during a run.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};

const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_VERSION_BYTES: usize = 4096;
const VERSION_TIMEOUT: Duration = Duration::from_secs(30);

/// The record keys that name the Git executable behind a source identity.
pub(crate) const FIELDS: [&str; 4] = [
    "tool.git.bytes",
    "tool.git.path",
    "tool.git.sha256",
    "tool.git.version",
];

static RESOLVED: OnceLock<Result<Git, String>> = OnceLock::new();

/// One resolved Git executable and the identity it had when resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    command.arg("--version");
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
    /// A Git query in `root` spawned from this exact executable.
    pub(crate) fn command(&self, root: &Path) -> Command {
        let mut command = Command::new(&self.path);
        crate::bounded_process::isolate_git_environment(&mut command);
        command.current_dir(root);
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
mod tests {
    use super::*;

    #[test]
    fn a_run_spawns_the_git_it_resolved_whatever_path_later_says() {
        let git = resolved().expect("Git resolves on the development host");
        assert!(git.path.is_absolute(), "{}", git.path.display());
        assert_eq!(
            std::fs::canonicalize(&git.path).unwrap(),
            git.path,
            "resolved Git is not canonical"
        );
        let command = git.command(Path::new("."));
        assert_eq!(command.get_program(), git.path.as_os_str());
        assert_eq!(
            resolved().unwrap() as *const Git,
            git as *const Git,
            "a second query resolved Git again"
        );
    }

    #[test]
    fn the_first_git_on_the_search_path_is_the_one_recorded() {
        let real = resolved().expect("Git resolves on the development host");
        let root = std::env::temp_dir().join(format!("wizard-git-decoy-{}", std::process::id()));
        let decoy = root.join("decoy");
        std::fs::create_dir_all(&decoy).unwrap();
        let name = format!("git{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(decoy.join(&name), b"not git").unwrap();
        let real_directory = real.path.parent().unwrap().to_path_buf();
        let search = std::env::join_paths([decoy.clone(), real_directory.clone()]).unwrap();
        let error = resolve(&search).unwrap_err();
        assert!(
            format!("{error:#}").contains("Git version probe"),
            "a non-Git decoy first on the search path was accepted: {error:#}"
        );
        let search = std::env::join_paths([real_directory]).unwrap();
        assert_eq!(resolve(&search).unwrap(), *real);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_executable_that_is_not_git_is_refused_by_its_version_identity() {
        let cargo = std::fs::canonicalize(env!("CARGO")).unwrap();
        let error = identify(cargo).unwrap_err().to_string();
        assert!(error.contains("no single-line version identity"), "{error}");
    }

    #[test]
    fn a_changed_executable_blocks_publication() {
        let real = resolved().expect("Git resolves on the development host");
        let root = std::env::temp_dir().join(format!("wizard-git-change-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let copy = std::fs::canonicalize(&root).unwrap().join("git-copy");
        std::fs::copy(&real.path, &copy).unwrap();
        let (bytes, sha256) = measure(&copy).unwrap();
        let recorded = Git {
            path: copy.clone(),
            version: real.version.clone(),
            bytes,
            sha256,
        };
        recorded.require_unchanged().unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&copy)
            .and_then(|mut file| std::io::Write::write_all(&mut file, b"\0"))
            .unwrap();
        let error = recorded.require_unchanged().unwrap_err().to_string();
        assert!(error.contains("changed during this run"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recorded_git_identity_is_validated_field_by_field() {
        let git = resolved().expect("Git resolves on the development host");
        let fields = git.fields();
        let check = |fields: &[(&'static str, String); 4]| {
            validate_fields(|key| {
                fields
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value.as_str())
                    .context("missing")
            })
        };
        check(&fields).unwrap();
        let uppercase = "A".repeat(64);
        for (index, value) in [
            (0, "0"),
            (0, "012"),
            (1, "relative/git.exe"),
            (2, uppercase.as_str()),
            (3, "2.44.0"),
        ] {
            let mut broken = fields.clone();
            broken[index].1 = value.to_string();
            assert!(
                check(&broken).is_err(),
                "{} = {value} was accepted",
                FIELDS[index]
            );
        }
    }
}
