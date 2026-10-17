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
mod tests {
    use super::*;

    fn repository_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
    }

    fn run_fixture_git(git: &Git, root: &Path, args: &[&str]) -> std::process::Output {
        let mut command = Command::new(&git.path);
        crate::bounded_process::isolate_git_environment(&mut command);
        command.current_dir(root).args(args);
        crate::bounded_process::output_with_limit(
            command,
            Duration::from_secs(10),
            "Git isolation fixture",
            64 * 1024,
        )
        .unwrap()
    }

    fn shell_command(path: &Path, arguments: &str) -> String {
        let path = path.to_string_lossy().replace('\\', "/");
        format!("\"{path}\" {arguments}")
    }

    fn poison_marker(name: &str) -> PathBuf {
        std::env::current_exe().unwrap().with_file_name(name)
    }

    #[test]
    fn a_run_spawns_the_git_it_resolved_whatever_path_later_says() {
        let git = resolved().expect("Git resolves on the development host");
        assert!(git.path.is_absolute(), "{}", git.path.display());
        assert_eq!(
            std::fs::canonicalize(&git.path).unwrap(),
            git.path,
            "resolved Git is not canonical"
        );
        let command = git.command(repository_root());
        assert_eq!(command.get_program(), git.path.as_os_str());
        assert_eq!(command.get_current_dir(), git.path.parent());
        assert_eq!(
            resolved().unwrap() as *const Git,
            git as *const Git,
            "a second query resolved Git again"
        );
    }

    #[test]
    fn repository_queries_bind_the_checkout_and_disable_executable_helpers() {
        let git = resolved().expect("Git resolves on the development host");
        let drivers = git
            .filter_drivers(repository_root())
            .expect("repository filter configuration is safe");
        let command = git.command_with_filters_disabled(repository_root(), &drivers);
        let args: Vec<_> = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        let expected = [
            "--no-pager".to_string(),
            "--literal-pathspecs".to_string(),
            "--no-replace-objects".to_string(),
            "--no-optional-locks".to_string(),
            format!("--git-dir={}", repository_root().join(".git").display()),
            format!("--work-tree={}", repository_root().display()),
            "core.fsmonitor=".to_string(),
            format!("core.hooksPath={DISABLED_GIT_PATH}"),
            format!("core.attributesFile={DISABLED_GIT_PATH}"),
            format!("core.excludesFile={DISABLED_GIT_PATH}"),
            "core.untrackedCache=false".to_string(),
            "core.pager=".to_string(),
            "diff.external=".to_string(),
            "diff.trustExitCode=false".to_string(),
            "interactive.diffFilter=".to_string(),
            "credential.helper=".to_string(),
        ];

        for expected in expected {
            assert!(
                args.iter().any(|argument| argument == &expected),
                "missing hardened Git argument {expected}"
            );
        }
    }

    #[test]
    fn filter_driver_inventory_is_strict_and_bounded() {
        assert!(parse_filter_drivers(b"").unwrap().is_empty());
        assert_eq!(
            parse_filter_drivers(
                b"filter.alpha.clean\0filter.beta.process\0filter.alpha.required\0"
            )
            .unwrap(),
            BTreeSet::from(["alpha".to_string(), "beta".to_string()])
        );
        for malformed in [
            &b"filter.alpha.clean"[..],
            &b"filter.alpha.clean\0\0"[..],
            &b"filter..clean\0"[..],
            &b"filter.bad=name.clean\0"[..],
            &b"filter.alpha.command\0"[..],
            &b"filter.\xff.clean\0"[..],
        ] {
            assert!(
                parse_filter_drivers(malformed).is_err(),
                "accepted malformed filter inventory {malformed:?}"
            );
        }

        let mut excessive_count = Vec::new();
        for index in 0..=MAX_FILTER_DRIVERS {
            excessive_count.extend_from_slice(format!("filter.driver{index}.clean\0").as_bytes());
        }
        assert!(parse_filter_drivers(&excessive_count).is_err());

        let oversized = format!(
            "filter.{}.clean\0",
            "x".repeat(MAX_FILTER_OVERRIDE_BYTES / 4 + 1)
        );
        assert!(parse_filter_drivers(oversized.as_bytes()).is_err());
    }

    #[test]
    fn a_local_fsmonitor_command_cannot_run_during_a_hardened_query() {
        let git = resolved().expect("Git resolves on the development host");
        let root = crate::test_support::reserve_temp_directory("wizard-git-fsmonitor");
        assert!(
            run_fixture_git(git, &root, &["init", "--quiet"])
                .status
                .success()
        );
        std::fs::write(root.join("tracked"), b"tracked\n").unwrap();
        assert!(
            run_fixture_git(git, &root, &["add", "tracked"])
                .status
                .success()
        );

        let marker = poison_marker("wizard-git-fsmonitor-ran");
        if marker.exists() {
            std::fs::remove_file(&marker).unwrap();
        }
        let poison = shell_command(
            &std::env::current_exe().unwrap(),
            "--exact git::tests::fsmonitor_poison_fixture_child --ignored --nocapture #",
        );
        assert!(
            run_fixture_git(
                git,
                &root,
                &["config", "--local", "core.fsmonitor", &poison],
            )
            .status
            .success()
        );

        let _ = run_fixture_git(
            git,
            &root,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        );
        assert!(marker.is_file(), "the local fsmonitor poison did not run");
        std::fs::remove_file(&marker).unwrap();

        let mut command = git.command(&root);
        command.args(["status", "--porcelain=v1", "--untracked-files=all"]);
        let output = crate::bounded_process::output_with_limit(
            command,
            Duration::from_secs(10),
            "hardened Git fsmonitor fixture",
            64 * 1024,
        )
        .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            !marker.exists(),
            "the hardened Git query executed core.fsmonitor"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore]
    fn fsmonitor_poison_fixture_child() {
        std::fs::write(poison_marker("wizard-git-fsmonitor-ran"), b"ran").unwrap();
    }

    #[test]
    #[ignore]
    fn filter_poison_fixture_child() {
        std::fs::write(poison_marker("wizard-git-filter-ran"), b"ran").unwrap();
    }

    #[derive(Clone, Copy)]
    enum AttributeSource {
        Tracked,
        Worktree,
        Info,
    }

    #[derive(Clone, Copy)]
    enum FilterConfigSource {
        LocalClean,
        IncludedClean,
        WorktreeProcess,
    }

    fn filter_fixture(
        git: &Git,
        name: &str,
        attributes: AttributeSource,
        config: FilterConfigSource,
    ) {
        let root =
            crate::test_support::reserve_temp_directory(&format!("wizard-git-filter-{name}"));
        assert!(
            run_fixture_git(git, &root, &["init", "--quiet"])
                .status
                .success()
        );
        std::fs::write(root.join("tracked"), b"indexed\n").unwrap();
        match attributes {
            AttributeSource::Tracked => {
                std::fs::write(root.join(".gitattributes"), b"tracked filter=poison\n").unwrap();
            }
            AttributeSource::Worktree => {
                std::fs::write(root.join(".gitattributes"), b"tracked text\n").unwrap();
            }
            AttributeSource::Info => {}
        }
        assert!(run_fixture_git(git, &root, &["add", "."]).status.success());
        match attributes {
            AttributeSource::Tracked => {}
            AttributeSource::Worktree => {
                std::fs::write(root.join(".gitattributes"), b"tracked filter=poison\n").unwrap();
            }
            AttributeSource::Info => {
                std::fs::write(
                    root.join(".git/info/attributes"),
                    b"tracked filter=poison\n",
                )
                .unwrap();
            }
        }

        let executable = std::env::current_exe().unwrap();
        let poison = shell_command(
            &executable,
            "--exact git::tests::filter_poison_fixture_child --ignored --nocapture",
        );
        match config {
            FilterConfigSource::LocalClean => assert!(
                run_fixture_git(
                    git,
                    &root,
                    &["config", "--local", "filter.poison.clean", &poison],
                )
                .status
                .success()
            ),
            FilterConfigSource::IncludedClean => {
                std::fs::write(
                    root.join(".git/filter.config"),
                    format!("[filter \"poison\"]\n\tclean = {poison}\n"),
                )
                .unwrap();
                assert!(
                    run_fixture_git(
                        git,
                        &root,
                        &["config", "--local", "include.path", "filter.config"],
                    )
                    .status
                    .success()
                );
            }
            FilterConfigSource::WorktreeProcess => {
                assert!(
                    run_fixture_git(
                        git,
                        &root,
                        &["config", "--local", "extensions.worktreeConfig", "true"],
                    )
                    .status
                    .success()
                );
                assert!(
                    run_fixture_git(
                        git,
                        &root,
                        &["config", "--worktree", "filter.poison.process", &poison],
                    )
                    .status
                    .success()
                );
            }
        }
        std::fs::write(root.join("tracked"), b"changed\n").unwrap();

        let marker = poison_marker("wizard-git-filter-ran");
        if marker.exists() {
            std::fs::remove_file(&marker).unwrap();
        }
        let _ = run_fixture_git(
            git,
            &root,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        );
        assert!(
            marker.is_file(),
            "the {name} fixture did not prove that unhardened Git ran its filter"
        );
        std::fs::remove_file(&marker).unwrap();

        let mut command = git.command(&root);
        command.args(["status", "--porcelain=v1", "--untracked-files=all"]);
        let output = crate::bounded_process::output_with_limit(
            command,
            Duration::from_secs(10),
            "hardened Git filter fixture",
            64 * 1024,
        )
        .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            !marker.exists(),
            "the hardened {name} query executed its content filter"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repository_content_filters_are_discovered_and_neutralized() {
        let git = resolved().expect("Git resolves on the development host");
        for (name, attributes, config) in [
            (
                "tracked-local-clean",
                AttributeSource::Tracked,
                FilterConfigSource::LocalClean,
            ),
            (
                "worktree-included-clean",
                AttributeSource::Worktree,
                FilterConfigSource::IncludedClean,
            ),
            (
                "info-worktree-process",
                AttributeSource::Info,
                FilterConfigSource::WorktreeProcess,
            ),
        ] {
            filter_fixture(git, name, attributes, config);
        }
    }

    #[test]
    fn malformed_repository_configuration_fails_closed_before_the_query() {
        let git = resolved().expect("Git resolves on the development host");
        let root = crate::test_support::reserve_temp_directory("wizard-git-malformed");
        assert!(
            run_fixture_git(git, &root, &["init", "--quiet"])
                .status
                .success()
        );
        std::fs::write(root.join(".git/config"), b"[unterminated\n").unwrap();

        let command = git.command(&root);
        assert!(
            command
                .get_args()
                .any(|argument| argument == REFUSED_CONFIGURATION_OPTION)
        );
        let output = crate::bounded_process::output_with_limit(
            command,
            Duration::from_secs(10),
            "refused Git configuration fixture",
            64 * 1024,
        )
        .unwrap();
        assert!(!output.status.success(), "{output:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn assert_hardened_status_succeeds(git: &Git, root: &Path, label: &str) {
        let drivers = git
            .filter_drivers(root)
            .unwrap_or_else(|error| panic!("{label} filter discovery failed: {error:#}"));
        let mut command = git.command_with_filters_disabled(root, &drivers);
        command.args(["status", "--porcelain=v1", "--untracked-files=all"]);
        let output = crate::bounded_process::output_with_limit(
            command,
            Duration::from_secs(10),
            label,
            64 * 1024,
        )
        .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[test]
    fn linked_worktree_and_submodule_style_gitfiles_remain_supported() {
        let git = resolved().expect("Git resolves on the development host");
        let root = crate::test_support::reserve_temp_directory("wizard-git-layouts");
        let main = root.join("main");
        let linked = root.join("linked");
        let nested = root.join("nested");
        let nested_git = root.join("nested-git");
        std::fs::create_dir_all(&main).unwrap();
        assert!(
            run_fixture_git(git, &main, &["init", "--quiet"])
                .status
                .success()
        );
        std::fs::write(main.join("tracked"), b"tracked\n").unwrap();
        assert!(
            run_fixture_git(git, &main, &["add", "tracked"])
                .status
                .success()
        );
        assert!(
            run_fixture_git(
                git,
                &main,
                &[
                    "-c",
                    "user.name=Wizard Test",
                    "-c",
                    "user.email=wizard@example.invalid",
                    "commit",
                    "--quiet",
                    "-m",
                    "fixture",
                ],
            )
            .status
            .success()
        );
        assert!(
            run_fixture_git(
                git,
                &main,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "--detach",
                    linked.to_str().unwrap()
                ],
            )
            .status
            .success()
        );
        assert_hardened_status_succeeds(git, &linked, "linked-worktree Git fixture");

        std::fs::create_dir_all(&nested).unwrap();
        assert!(
            run_fixture_git(git, &nested, &["init", "--quiet"])
                .status
                .success()
        );
        std::fs::write(nested.join("tracked"), b"tracked\n").unwrap();
        assert!(
            run_fixture_git(git, &nested, &["add", "tracked"])
                .status
                .success()
        );
        std::fs::rename(nested.join(".git"), &nested_git).unwrap();
        let nested_git = nested_git.to_string_lossy().replace('\\', "/");
        std::fs::write(nested.join(".git"), format!("gitdir: {nested_git}\n")).unwrap();
        assert_hardened_status_succeeds(git, &nested, "submodule-style Git fixture");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_first_git_on_the_search_path_is_the_one_recorded() {
        let real = resolved().expect("Git resolves on the development host");
        let root = crate::test_support::reserve_temp_directory("wizard-git-decoy");
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
        let root = crate::test_support::reserve_temp_directory("wizard-git-change");
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
