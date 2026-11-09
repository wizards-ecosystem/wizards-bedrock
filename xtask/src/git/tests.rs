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
        parse_filter_drivers(b"filter.alpha.clean\0filter.beta.process\0filter.alpha.required\0")
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

fn filter_fixture(git: &Git, name: &str, attributes: AttributeSource, config: FilterConfigSource) {
    let root = crate::test_support::reserve_temp_directory(&format!("wizard-git-filter-{name}"));
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
