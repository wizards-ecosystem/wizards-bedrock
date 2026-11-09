use super::*;

#[cfg(unix)]
fn create_directory_symlink(target: &std::path::Path, link: &std::path::Path) {
    std::os::unix::fs::symlink(target, link).expect("creating cleanup directory symlink");
}

#[cfg(windows)]
fn create_directory_symlink(target: &std::path::Path, link: &std::path::Path) {
    std::os::windows::fs::symlink_dir(target, link).expect("creating cleanup directory symlink");
}

#[cfg(unix)]
fn create_file_symlink(target: &std::path::Path, link: &std::path::Path) {
    std::os::unix::fs::symlink(target, link).expect("creating cleanup file symlink");
}

#[cfg(windows)]
fn create_file_symlink(target: &std::path::Path, link: &std::path::Path) {
    std::os::windows::fs::symlink_file(target, link).expect("creating cleanup file symlink");
}

fn clean_fixture(label: &str) -> std::path::PathBuf {
    crate::test_support::reserve_temp_directory(&format!("wizard-clean-{label}"))
}

fn retained_runner_fixture(
    label: &str,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let fixture = clean_fixture(label);
    let target = fixture.join("target");
    let runner = target
        .join("xtask")
        .join(format!("xtask{}", std::env::consts::EXE_SUFFIX));
    std::fs::create_dir_all(runner.parent().unwrap()).unwrap();
    std::fs::write(&runner, b"running clean image").unwrap();
    (fixture, target, runner)
}

const PIXEL: &[&str] = &["pixel"];

fn pass(target: Option<&'static str>, features: Option<&'static str>) -> gate::CargoPass {
    gate::CargoPass {
        name: "fixture",
        packages: PIXEL,
        target,
        features,
    }
}

fn args(target: Option<&'static str>) -> Vec<String> {
    pass_args(
        "clippy",
        pass(target, None),
        Scope::AllTargets,
        &[],
        &["-D".to_string()],
    )
}

#[test]
fn the_host_half_lints_its_tests() {
    assert_eq!(
        args(None),
        ["clippy", "--package", "pixel", "--all-targets", "--", "-D"]
    );
}

#[test]
fn the_mutation_observer_forwards_its_caller_deadline() {
    let source = include_str!("../tasks.rs");
    let start = source
        .find("pub fn observe_test_boot(")
        .expect("observe_test_boot definition");
    let body = &source[start..];
    let end = body.find("\n}\n").expect("observe_test_boot end");
    let body = &body[..end];
    let expected = ["qemu::observe_selftest(paths, image.path(), ", "timeout)"].concat();

    assert!(body.contains(&expected), "caller deadline is not forwarded");
}

#[test]
fn the_kernel_half_is_cross_compiled_and_has_no_extra_targets() {
    let args = args(Some(paths::KERNEL_TARGET));
    assert!(args.contains(&"--target".to_string()));
    assert!(
        !args.contains(&"--all-targets".to_string()),
        "the kernel crates have no host test targets"
    );
}

#[test]
fn a_pass_with_features_names_them_and_one_without_names_none() {
    let with = pass_args(
        "clippy",
        pass(
            Some(paths::KERNEL_TARGET),
            Some(paths::SELFTEST_FEATURE_SPEC),
        ),
        Scope::AllTargets,
        &[],
        &[],
    );
    let position = with.iter().position(|arg| arg == "--features");
    assert_eq!(
        position.map(|index| with[index + 1].as_str()),
        Some(paths::SELFTEST_FEATURE_SPEC)
    );

    assert!(!args(Some(paths::KERNEL_TARGET)).contains(&"--features".to_string()));
}

#[test]
fn the_host_half_has_no_kernel_features() {
    assert!(!args(None).contains(&"--features".to_string()));
}

#[test]
fn nothing_follows_a_bare_double_dash() {
    let args = pass_args("check", pass(None, None), Scope::AllTargets, &[], &[]);
    assert!(!args.contains(&"--".to_string()));
}

/// `cargo doc` rejects `--all-targets` rather than ignoring it, so the
/// scope has to be part of the call and not inferred from the graph.
#[test]
fn a_default_scope_asks_for_no_extra_targets() {
    let args = pass_args("doc", pass(None, None), Scope::Default, &[], &[]);

    assert_eq!(args, ["doc", "--package", "pixel"]);
}

/// Both shipped and self-test feature graphs must be linted because one
/// graph can hide dead code in the other.
#[test]
fn the_gate_covers_every_kernel_feature_graph_it_boots() {
    let kernel: Vec<Option<&str>> = gate::REGISTRY
        .cargo_passes()
        .iter()
        .filter(|pass| pass.packages == paths::KERNEL_PACKAGES)
        .map(|pass| pass.features)
        .collect();

    assert_eq!(
        kernel,
        [
            None,
            Some(paths::SELFTEST_FEATURE_SPEC),
            Some(paths::REFUSE_PCI_FEATURE_SPEC)
        ]
    );
}

#[test]
fn the_gate_covers_the_host_packages_exactly_once() {
    let host = gate::REGISTRY
        .cargo_passes()
        .iter()
        .filter(|pass| pass.packages == paths::HOST_PACKAGES)
        .count();

    assert_eq!(host, 1);
}

/// Every pass has to be distinguishable in the log, or a failure in one
/// configuration reads as a failure in the other.
#[test]
fn every_pass_is_labelled_distinctly() {
    let labels: Vec<String> = gate::REGISTRY
        .cargo_passes()
        .iter()
        .map(|pass| pass.label())
        .collect();

    for (index, label) in labels.iter().enumerate() {
        assert!(!label.is_empty());
        assert!(
            !labels[index + 1..].contains(label),
            "two passes both called {label}"
        );
    }
}

/// Kernel passes are cross-compiled and host passes are not — the property
/// that stops a kernel crate being checked for the host triple, which is
/// what produces phantom errors across every kernel file.
#[test]
fn only_the_host_pass_builds_for_the_host() {
    for pass in gate::REGISTRY.cargo_passes() {
        let cross = pass.target == Some(paths::KERNEL_TARGET);
        assert_eq!(
            cross,
            pass.packages == paths::KERNEL_PACKAGES,
            "{}",
            pass.label()
        );
    }
}

#[test]
fn a_failed_clean_requires_the_exact_live_executable_residual() {
    let missing_fixture = clean_fixture("missing-runner");
    let missing_target = missing_fixture.join("target");
    let missing_runner = missing_target
        .join("xtask")
        .join(format!("xtask{}", std::env::consts::EXE_SUFFIX));

    assert!(!finish_cargo_clean(true, &missing_target, None).unwrap());
    assert!(finish_cargo_clean(false, &missing_target, Some(&missing_runner)).is_err());
    std::fs::remove_dir_all(missing_fixture).unwrap();

    let (fixture, target, runner) = retained_runner_fixture("exact-runner");
    assert!(finish_cargo_clean(false, &target, Some(&runner)).unwrap());
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_failed_clean_rejects_an_extra_target_entry() {
    let (fixture, target, runner) = retained_runner_fixture("extra-target");
    std::fs::write(target.join("zz-residue"), b"unexpected target state").unwrap();

    assert!(finish_cargo_clean(false, &target, Some(&runner)).is_err());
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_failed_clean_rejects_an_extra_xtask_profile_entry() {
    let (fixture, target, runner) = retained_runner_fixture("extra-profile");
    std::fs::write(
        runner.parent().unwrap().join("zz-residue"),
        b"unexpected profile state",
    )
    .unwrap();

    assert!(finish_cargo_clean(false, &target, Some(&runner)).is_err());
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_failed_clean_rejects_a_foreign_executable_identity() {
    let (fixture, target, runner) = retained_runner_fixture("foreign-runner");
    let foreign = fixture.join(format!("foreign{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&foreign, b"foreign executable").unwrap();

    assert!(finish_cargo_clean(false, &target, Some(&foreign)).is_err());
    assert!(runner.is_file());
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_failed_clean_rejects_a_non_file_runner() {
    let fixture = clean_fixture("non-file-runner");
    let target = fixture.join("target");
    let runner = target
        .join("xtask")
        .join(format!("xtask{}", std::env::consts::EXE_SUFFIX));
    std::fs::create_dir_all(&runner).unwrap();

    assert!(finish_cargo_clean(false, &target, Some(&runner)).is_err());
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_failed_clean_rejects_a_redirected_target_directory() {
    let fixture = clean_fixture("redirected-target");
    let target = fixture.join("target");
    let outside_target = fixture.join("outside-target");
    let runner = outside_target
        .join("xtask")
        .join(format!("xtask{}", std::env::consts::EXE_SUFFIX));
    std::fs::create_dir_all(runner.parent().unwrap()).unwrap();
    std::fs::write(&runner, b"outside target executable").unwrap();
    create_directory_symlink(&outside_target, &target);

    assert!(finish_cargo_clean(false, &target, Some(&runner)).is_err());
    assert_eq!(
        std::fs::read(&runner).unwrap(),
        b"outside target executable"
    );
    crate::test_support::remove_directory_link(&target);
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_failed_clean_rejects_a_redirected_xtask_profile() {
    let fixture = clean_fixture("redirected-profile");
    let target = fixture.join("target");
    let profile = target.join("xtask");
    let outside_profile = fixture.join("outside-profile");
    let runner = outside_profile.join(format!("xtask{}", std::env::consts::EXE_SUFFIX));
    std::fs::create_dir_all(&target).unwrap();
    std::fs::create_dir_all(&outside_profile).unwrap();
    std::fs::write(&runner, b"outside profile executable").unwrap();
    create_directory_symlink(&outside_profile, &profile);

    assert!(finish_cargo_clean(false, &target, Some(&runner)).is_err());
    assert_eq!(
        std::fs::read(&runner).unwrap(),
        b"outside profile executable"
    );
    crate::test_support::remove_directory_link(&profile);
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_failed_clean_rejects_a_redirected_runner() {
    let fixture = clean_fixture("redirected-runner");
    let target = fixture.join("target");
    let runner = target
        .join("xtask")
        .join(format!("xtask{}", std::env::consts::EXE_SUFFIX));
    std::fs::create_dir_all(runner.parent().unwrap()).unwrap();
    let outside = fixture.join(format!("outside{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&outside, b"outside executable").unwrap();
    create_file_symlink(&outside, &runner);

    assert!(finish_cargo_clean(false, &target, Some(&outside)).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside executable");
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn recursive_vm_cleanup_is_confined_to_one_real_repository_child() {
    let fixture = clean_fixture("confined");
    let repository = fixture.join("repository");
    std::fs::create_dir_all(repository.join("vm/nested/deeper")).unwrap();
    std::fs::write(repository.join("vm/root-file"), b"generated root state").unwrap();
    std::fs::write(
        repository.join("vm/nested/deeper/generated"),
        b"generated nested state",
    )
    .unwrap();
    let outside = repository.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let sentinel = outside.join("sentinel");
    std::fs::write(&sentinel, b"outside authority").unwrap();

    assert!(remove_vm_directory(&repository, &outside).is_err());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
    assert!(remove_vm_directory(&repository, &repository.join("vm")).unwrap());
    assert!(repository.join("vm").is_dir());
    assert_eq!(std::fs::read_dir(repository.join("vm")).unwrap().count(), 0);
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn recursive_vm_cleanup_refuses_redirected_paths_without_touching_the_target() {
    let fixture = clean_fixture("redirected");
    let repository = fixture.join("repository");
    let redirected = repository.join("redirected");
    std::fs::create_dir_all(&redirected).unwrap();
    let sentinel = redirected.join("sentinel");
    std::fs::write(&sentinel, b"outside authority").unwrap();
    create_directory_symlink(&redirected, &repository.join("vm"));

    assert!(remove_vm_directory(&repository, &repository.join("vm")).is_err());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
    crate::test_support::remove_directory_link(&repository.join("vm"));
    std::fs::create_dir(repository.join("vm")).unwrap();
    create_directory_symlink(&redirected, &repository.join("vm/redirected"));

    assert!(remove_vm_directory(&repository, &repository.join("vm")).unwrap());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
    assert_eq!(std::fs::read_dir(repository.join("vm")).unwrap().count(), 0);

    let linked_repository = fixture.join("repository-link");
    create_directory_symlink(&repository, &linked_repository);
    assert!(remove_vm_directory(&linked_repository, &linked_repository.join("vm")).is_err());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
    crate::test_support::remove_directory_link(&linked_repository);

    let linked_parent = fixture.join("repository-parent-link");
    create_directory_symlink(&fixture, &linked_parent);
    let repository_below_link = linked_parent.join("repository");
    assert!(
        remove_vm_directory(&repository_below_link, &repository_below_link.join("vm")).is_err()
    );
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
    crate::test_support::remove_directory_link(&linked_parent);
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn recursive_vm_cleanup_stays_bound_when_a_validated_ancestor_is_replaced() {
    let fixture = clean_fixture("ancestor-race");
    let repository = fixture.join("repository");
    let moved_repository = fixture.join("repository-before-swap");
    let outside = fixture.join("outside");
    std::fs::create_dir_all(repository.join("vm/nested")).unwrap();
    std::fs::write(repository.join("vm/nested/generated"), b"generated state").unwrap();
    std::fs::create_dir_all(outside.join("vm")).unwrap();
    let sentinel = outside.join("vm/sentinel");
    std::fs::write(&sentinel, b"outside authority").unwrap();
    let mut swapped = false;

    let result = remove_vm_directory_with(&repository, &repository.join("vm"), || {
        if std::fs::rename(&repository, &moved_repository).is_ok() {
            std::fs::rename(&outside, &repository).unwrap();
            swapped = true;
        }
    });

    if swapped {
        std::fs::rename(&repository, &outside).unwrap();
        std::fs::rename(&moved_repository, &repository).unwrap();
    }
    let sentinel_after = std::fs::read(&sentinel);
    std::fs::remove_dir_all(&fixture).unwrap();

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(sentinel_after.unwrap(), b"outside authority");
}

#[test]
fn recursive_vm_cleanup_has_an_entry_budget_before_deleting() {
    let fixture = clean_fixture("entry-budget");
    let directory = fixture.join("vm");
    std::fs::create_dir(&directory).unwrap();
    let sentinel = directory.join("sentinel");
    std::fs::write(&sentinel, b"generated state").unwrap();
    let mut handle =
        crate::mutate::workspace::open_real_directory(&directory, "cleanup fixture").unwrap();
    let mut remaining = 0;

    let error = empty_open_directory(&mut handle, &directory, 0, &mut remaining)
        .expect_err("an exhausted cleanup budget was accepted");
    assert!(error.to_string().contains("entry limit"));
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"generated state");

    drop(handle);
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn recursive_vm_cleanup_has_a_depth_bound_before_deleting() {
    let fixture = clean_fixture("depth-bound");
    let directory = fixture.join("vm");
    std::fs::create_dir(&directory).unwrap();
    let sentinel = directory.join("sentinel");
    std::fs::write(&sentinel, b"generated state").unwrap();
    let mut handle =
        crate::mutate::workspace::open_real_directory(&directory, "cleanup fixture").unwrap();
    let mut remaining = MAX_VM_CLEANUP_ENTRIES;

    let error = empty_open_directory(
        &mut handle,
        &directory,
        MAX_VM_CLEANUP_DEPTH + 1,
        &mut remaining,
    )
    .expect_err("an over-depth cleanup was accepted");
    assert!(error.to_string().contains("depth limit"));
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"generated state");

    drop(handle);
    std::fs::remove_dir_all(fixture).unwrap();
}
