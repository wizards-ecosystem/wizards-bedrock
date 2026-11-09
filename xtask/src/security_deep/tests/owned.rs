use super::*;

#[test]
fn plugin_manifest_paths_are_normalized_relative_files() {
    assert_eq!(
        plugin_relative_path("scripts/workbench.py").unwrap(),
        PathBuf::from("scripts").join("workbench.py")
    );
    for path in [
        "",
        "/absolute",
        "C:/absolute",
        "scripts\\redirect.py",
        "scripts/../redirect.py",
        "scripts//redirect.py",
        "scripts/",
    ] {
        assert!(
            plugin_relative_path(path).is_err(),
            "accepted unsafe package path {path}"
        );
    }
}

#[cfg(windows)]
#[test]
fn an_ordinary_windows_codex_home_matches_its_canonical_verbatim_spelling() {
    let ordinary = Path::new(r"C:\Users\wizard\.codex");
    let canonical = Path::new(r"\\?\C:\Users\wizard\.codex");
    assert!(canonical_spelling_matches(canonical, ordinary));
    assert!(canonical_spelling_matches(canonical, canonical));
    assert!(!canonical_spelling_matches(
        canonical,
        Path::new(r"C:\Users\other\.codex")
    ));
}

#[test]
fn private_codex_home_projects_only_locked_auth_and_scrubs_it() {
    let _scratch_guard = SCRATCH_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let scratch = reserve_scratch().unwrap();
    let live = scratch.root.join("hostile-live-home");
    std::fs::create_dir(&live).unwrap();
    std::fs::write(live.join("auth.json"), br#"{"tokens":"sensitive"}"#).unwrap();
    std::fs::write(live.join("config.toml"), b"model_provider='hostile'").unwrap();
    for name in ["skills", "rules", "plugins", "memories"] {
        std::fs::create_dir(live.join(name)).unwrap();
        std::fs::write(live.join(name).join("hostile"), b"untrusted").unwrap();
    }
    let live = std::fs::canonicalize(live).unwrap();
    let mut credential = prepare_private_codex_home_from(&live, &scratch).unwrap();
    require_private_codex_home_seed(&scratch.codex_home(), &credential).unwrap();
    assert_eq!(file_identity(&live.join("auth.json")).unwrap().links, 2);
    assert_eq!(
        file_identity(&scratch.codex_home().join("auth.json"))
            .unwrap()
            .links,
        2
    );
    let late_alias = live.join("late-auth-alias.json");
    std::fs::hard_link(live.join("auth.json"), &late_alias).unwrap();
    assert!(
        require_exact_credential_projection(
            &live.join("auth.json"),
            &scratch.codex_home().join("auth.json"),
        )
        .is_err()
    );
    std::fs::remove_file(late_alias).unwrap();
    require_private_codex_home_seed(&scratch.codex_home(), &credential).unwrap();
    assert_eq!(
        std::fs::read(scratch.codex_home().join("config.toml")).unwrap(),
        b""
    );
    for name in ["skills", "rules", "plugins", "memories"] {
        assert!(!scratch.codex_home().join(name).exists());
    }
    credential.scrub().unwrap();
    assert!(!scratch.codex_home().join("auth.json").exists());
    remove_scratch(&scratch).unwrap();
}

#[test]
fn private_codex_home_refuses_a_preexisting_credential_hard_link() {
    let _scratch_guard = SCRATCH_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let scratch = reserve_scratch().unwrap();
    let live = scratch.root.join("hostile-live-home-with-alias");
    std::fs::create_dir(&live).unwrap();
    let auth = live.join("auth.json");
    std::fs::write(&auth, br#"{"tokens":"non-secret-fixture"}"#).unwrap();
    std::fs::hard_link(&auth, live.join("auth-alias.json")).unwrap();
    let live = std::fs::canonicalize(live).unwrap();
    let error = prepare_private_codex_home_from(&live, &scratch)
        .unwrap_err()
        .to_string();
    assert!(error.contains("pre-existing hard-link alias"));
    assert!(!scratch.codex_home().join("auth.json").exists());
    remove_scratch(&scratch).unwrap();
}

#[test]
fn runtime_generated_system_skills_are_exact_and_user_skills_stay_closed() {
    let (outer, root) = private_test_root();
    let home = root.join("codex-home");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("config.toml"), b"").unwrap();
    require_no_ambient_codex_inputs(&home).unwrap();

    let skill = home.join("skills").join(".system").join("test");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), b"qualified\n").unwrap();
    require_no_ambient_codex_inputs(&home).unwrap();

    std::fs::write(home.join("skills").join("user-skill"), b"hostile").unwrap();
    let error = require_no_ambient_codex_inputs(&home).unwrap_err();
    assert!(
        format!("{error:#}").contains("differ from the qualified runtime"),
        "wrong system-skills drift error: {error:#}"
    );
    let metadata = std::fs::symlink_metadata(&outer).unwrap();
    remove_entry_no_follow(&outer, &metadata).unwrap();
}

#[test]
fn system_skill_tree_fold_uses_ordinal_case_sensitive_path_order() {
    let (outer, root) = private_test_root();
    let skills = root.join("system-skills");
    std::fs::create_dir(&skills).unwrap();
    std::fs::write(skills.join("Zeta"), b"z").unwrap();
    std::fs::write(skills.join("alpha"), b"a").unwrap();

    let identity = measure_qualified_system_skills(&skills).unwrap();
    assert_eq!(identity.entries, 2);
    assert_eq!(identity.bytes, 2);
    assert_eq!(
        identity.sha256,
        "5b60bb4b4bbbf8e58be96bf961a4a12a660e04b36b7352acfe1b0808ee026be5"
    );

    let metadata = std::fs::symlink_metadata(&outer).unwrap();
    remove_entry_no_follow(&outer, &metadata).unwrap();
}

#[test]
fn legacy_culture_sorted_system_skill_digest_is_refused() {
    assert_eq!(
        CODEX_SYSTEM_SKILLS_SHA256,
        "54c2abe5f6017b67270f4bbb993b3b1e3a808da31575a1d24174cb4b2edd99c8"
    );
    assert!(codex_system_skills_identity_is_allowed(&TreeIdentity {
        entries: CODEX_SYSTEM_SKILLS_ENTRIES,
        bytes: CODEX_SYSTEM_SKILLS_BYTES,
        sha256: CODEX_SYSTEM_SKILLS_SHA256.to_string(),
    }));
    assert!(!codex_system_skills_identity_is_allowed(&TreeIdentity {
        entries: CODEX_SYSTEM_SKILLS_ENTRIES,
        bytes: CODEX_SYSTEM_SKILLS_BYTES,
        sha256: "1715d9d4d5d41e12e37d33e46d44d970ca64f298846430a20c6af04c8f1caf7c".to_string(),
    }));
}

#[test]
fn stale_credential_cleanup_unlinks_current_and_rotated_regular_names() {
    let (outer, root) = private_test_root();
    let home = root.join("codex-home");
    std::fs::create_dir(&home).unwrap();
    let live = outer.join("live-auth.json");
    std::fs::write(&live, b"current").unwrap();
    let retained = home.join("auth.json");
    std::fs::hard_link(&live, &retained).unwrap();
    scrub_stale_credential_at_owned_root(&root).unwrap();
    assert!(!retained.exists());
    assert_eq!(std::fs::read(&live).unwrap(), b"current");

    std::fs::write(&retained, b"rotated-stale-secret").unwrap();
    scrub_stale_credential_at_owned_root(&root).unwrap();
    assert!(!retained.exists());
    assert_eq!(std::fs::read(&live).unwrap(), b"current");

    let metadata = std::fs::symlink_metadata(&outer).unwrap();
    remove_entry_no_follow(&outer, &metadata).unwrap();
}

#[test]
fn stale_credential_cleanup_refuses_a_non_file_auth_name() {
    let (outer, root) = private_test_root();
    let retained = root.join("codex-home").join("auth.json");
    std::fs::create_dir_all(&retained).unwrap();

    let error = scrub_stale_credential_at_owned_root(&root)
        .unwrap_err()
        .to_string();
    assert!(error.contains("redirected or special"), "{error}");
    assert!(retained.is_dir());

    let metadata = std::fs::symlink_metadata(&outer).unwrap();
    remove_entry_no_follow(&outer, &metadata).unwrap();
}

#[test]
fn owned_worker_repository_is_a_clean_detached_projection() {
    let repository = crate::cargo::child_process_path(&unique_test_directory("wdss"));
    require_fixture_git(&repository, &["init", "--quiet"]);
    std::fs::write(repository.join(".gitignore"), b"ignored/\n").unwrap();
    std::fs::write(repository.join("README.md"), b"fixture repository\n").unwrap();
    std::fs::write(repository.join("tracked.txt"), b"tracked source\n").unwrap();
    std::fs::create_dir(repository.join("ignored")).unwrap();
    std::fs::write(
        repository.join("ignored").join("host-secret"),
        b"not an input",
    )
    .unwrap();
    require_fixture_git(
        &repository,
        &["add", ".gitignore", "README.md", "tracked.txt"],
    );
    require_fixture_git(
        &repository,
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
    );
    require_fixture_git(&repository, &["tag", "fixture-tag"]);
    let source = qualification::source_identity(&repository).unwrap();
    let paths = Paths::new(repository.clone(), Some(&repository.join("target")));
    let (scratch_outer, scratch) = test_owned_scratch();
    let occupied_destination = scratch
        .repository_parent
        .join(repository.file_name().unwrap());
    std::fs::create_dir(&occupied_destination).unwrap();
    let occupied_error =
        prepare_owned_worker_repository(&paths, &scratch, crate::git::resolved().unwrap(), &source)
            .unwrap_err()
            .to_string();
    assert!(occupied_error.contains("destination already exists"));
    std::fs::remove_dir(&occupied_destination).unwrap();
    let worker =
        prepare_owned_worker_repository(&paths, &scratch, crate::git::resolved().unwrap(), &source)
            .unwrap();

    assert_eq!(worker.parent(), Some(scratch.repository_parent.as_path()));
    assert_eq!(
        qualification::source_identity(&crate::cargo::child_process_path(&worker)).unwrap(),
        source
    );
    assert_eq!(
        std::fs::read(worker.join("tracked.txt")).unwrap(),
        b"tracked source\n"
    );
    assert!(!worker.join("ignored").exists());
    assert!(require_fixture_git(&worker, &["remote"]).stdout.is_empty());
    assert!(
        require_fixture_git(&worker, &["tag", "--list"])
            .stdout
            .is_empty()
    );

    let object_relative = PathBuf::from(".git")
        .join("objects")
        .join(&source.commit[..2])
        .join(&source.commit[2..]);
    let original_object = file_identity(&repository.join(&object_relative)).unwrap();
    let worker_object = file_identity(&worker.join(&object_relative)).unwrap();
    assert_ne!(original_object.id, worker_object.id);
    assert_eq!(original_object.links, 1);
    assert_eq!(worker_object.links, 1);

    for path in [scratch_outer, repository] {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        remove_entry_no_follow(&path, &metadata).unwrap();
    }
}

#[cfg(not(windows))]
#[test]
fn the_owned_campaign_refuses_off_windows() {
    let paths = Paths::new(PathBuf::from("unused-repository"), None);
    let error = run(&paths, SecurityDeepAction::Run)
        .unwrap_err()
        .to_string();
    assert!(error.contains("requires Windows Job ownership"), "{error}");
}

#[test]
fn mirror_and_scratch_observers_reject_drift_redirects_and_growth() {
    let outer = unique_test_directory("wizard-security-deep-observers-test");
    let mirror = outer.join("mirror");
    std::fs::create_dir(&mirror).unwrap();
    let bytes = b"owned plugin bytes";
    std::fs::write(mirror.join("integrity.json"), b"synthetic integrity").unwrap();
    std::fs::write(mirror.join("entry.py"), bytes).unwrap();
    let package = PluginPackage {
        root: outer.join("source"),
        integrity: b"synthetic integrity".to_vec(),
        files: vec![PluginFile {
            relative: PathBuf::from("entry.py"),
            slash_path: "entry.py".to_string(),
            sha256: sha256::hex(&sha256::digest(bytes)),
        }],
    };
    require_mirror_exact(&mirror, &package).unwrap();
    std::fs::write(mirror.join("entry.py"), b"replacement").unwrap();
    assert!(require_mirror_exact(&mirror, &package).is_err());
    std::fs::write(mirror.join("entry.py"), bytes).unwrap();
    std::fs::write(mirror.join("unlisted"), b"extra").unwrap();
    assert!(require_mirror_exact(&mirror, &package).is_err());
    std::fs::remove_file(mirror.join("unlisted")).unwrap();

    let scratch = outer.join("scratch");
    std::fs::create_dir(&scratch).unwrap();
    std::fs::write(scratch.join("one"), b"x").unwrap();
    observe_scratch(&scratch).unwrap();
    let mut entries = MAX_SCRATCH_ENTRIES;
    let mut measured_bytes = 0;
    assert!(measure_scratch(&scratch, 0, &mut entries, &mut measured_bytes).is_err());
    let mut entries = 0;
    let mut measured_bytes = MAX_SCRATCH_BYTES;
    assert!(measure_scratch(&scratch, 0, &mut entries, &mut measured_bytes).is_err());
    let mut entries = 0;
    let mut measured_bytes = 0;
    assert!(
        measure_scratch(
            &scratch,
            MAX_SCRATCH_DEPTH + 1,
            &mut entries,
            &mut measured_bytes,
        )
        .is_err()
    );

    #[cfg(windows)]
    {
        let outside = outer.join("outside");
        std::fs::create_dir(&outside).unwrap();
        let redirected = scratch.join("redirected");
        if std::os::windows::fs::symlink_dir(&outside, &redirected).is_ok() {
            assert!(observe_scratch(&scratch).is_err());
            assert!(
                require_owned_path(
                    &redirected.join("prospective").join("result.json"),
                    &scratch,
                    "redirected prospective path",
                )
                .is_err()
            );
            std::fs::remove_dir(&redirected).unwrap();
        }
    }

    let metadata = std::fs::symlink_metadata(&outer).unwrap();
    remove_entry_no_follow(&outer, &metadata).unwrap();
}

#[test]
fn cleanup_and_pass_order_are_fail_closed() {
    let parent = canonical_directory(
        &std::env::temp_dir(),
        "test Codex Security temporary parent",
    )
    .unwrap();
    require_scratch_cleanup_target(&parent.join(OWNED_SCRATCH_NAME)).unwrap();
    assert!(require_scratch_cleanup_target(&parent.join("other")).is_err());
    assert!(
        require_scratch_cleanup_target(&parent.join("nested").join(OWNED_SCRATCH_NAME)).is_err()
    );

    let implementation = include_str!("../../security_deep.rs");
    let run = implementation.find("fn run_owned(").unwrap();
    let reserve = implementation[run..].find("fn reserve_scratch(").unwrap() + run;
    let body = &implementation[run..reserve];
    assert!(
        body.find("remove_scratch(&scratch)").unwrap()
            < body.find("pass_observed_deep(authority").unwrap()
    );
    assert!(
        body.find("require_absent(&inert_mcp").unwrap()
            < body.find("pass_observed_deep(authority").unwrap()
    );
    let preflight_sequence = [
        "require_safe_effective_config(&effective",
        "let git_identity = git_identity(git)?",
        "let request = render_owned_request",
        "require_worker_filesystem_policy",
        "let outcome = run_mcp",
    ];
    let preflight_positions = preflight_sequence
        .iter()
        .map(|needle| {
            body.find(needle)
                .unwrap_or_else(|| panic!("missing {needle}"))
        })
        .collect::<Vec<_>>();
    assert!(preflight_positions.windows(2).all(|pair| pair[0] < pair[1]));
    let final_start = body
        .find("let final_effective = read_codex_config")
        .unwrap();
    let final_body = &body[final_start..];
    let final_sequence = [
        "let final_effective = read_codex_config",
        "resolve_toolchain(paths.root(), &scratch)? == tools",
        "credential.scrub()?",
        "let manifest = read(",
        "let scan = verify_scan",
        "let transitions = aggregate_worker_transitions(",
        "qualification::source_identity(paths.root())? == source",
        "qualification::source_identity(&worker_git_repository)? == source",
        "git.require_unchanged()?",
        "require_plugin_unchanged(&package)?",
        "require_mirror_exact(&scratch.plugin, &package)?",
        "require_worker_canaries(&scratch)?",
        "require_absent(&inert_mcp",
        "observe_scratch(&scratch.root)?",
        "let artifacts = qualification::ObservedDeepArtifacts",
        "let fields = fields(",
        "finalize_owned_transaction(",
        "remove_scratch(&scratch)",
        "pass_observed_deep(authority",
    ];
    let positions = final_sequence
        .iter()
        .map(|needle| {
            final_body
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle}"))
        })
        .collect::<Vec<_>>();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));

    let events = std::cell::RefCell::new(Vec::new());
    let rejected: Result<()> = finalize_owned_transaction(
        || {
            events.borrow_mut().push("cleanup");
            bail!("synthetic cleanup failure")
        },
        || {
            events.borrow_mut().push("publish");
            Ok(())
        },
    );
    assert!(rejected.is_err());
    assert_eq!(*events.borrow(), ["cleanup"]);

    events.borrow_mut().clear();
    finalize_owned_transaction(
        || {
            events.borrow_mut().push("cleanup");
            Ok(())
        },
        || {
            events.borrow_mut().push("publish");
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(*events.borrow(), ["cleanup", "publish"]);
}
