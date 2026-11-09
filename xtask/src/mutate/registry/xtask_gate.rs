use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 23,
        file: "xtask/src/cli.rs",
        find: "    qualify-advisory  as above (needs cargo-audit and fresh RustSec data)",
        replace: "                      advisory qualification omitted",
        breaks: "the exceptional-task contract omits its networked RustSec qualification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::every_exceptional_task_is_named_in_the_not_in_ci_block",
        },
    },
    Mutation {
        id: 25,
        file: "xtask/src/lib.rs",
        find: "        Task::QualifyAdvisory => qualification::qualify_advisory(&paths),",
        replace: "        Task::QualifyAdvisory => Ok(()),",
        breaks: "the supported advisory qualification command is a successful no-op",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        id: 105,
        file: "xtask/src/model.rs",
        find: "    let qualification_attempt = qualification::begin_model(paths)?;\n    let jar = paths.root().join(JAR);\n    verify_jar(&jar)?;",
        replace: "    let jar = paths.root().join(JAR);\n    verify_jar(&jar)?;\n    let qualification_attempt = qualification::begin_model(paths)?;",
        breaks: "a failed model preflight leaves an older passing qualification current",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        id: 106,
        file: "xtask/src/model.rs",
        find: "    qualification_attempt.pass(&transcripts)?;",
        replace: "    drop(qualification_attempt);",
        breaks: "a successful model check does not commit its qualification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        id: 149,
        file: "xtask/src/lib.rs",
        find: "            vmware::invalidate_generation(&paths, kernel)?;",
        replace: "            let _ = (&paths, kernel);",
        breaks: "a VMware task leaves an old generation receipt through a failed build",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::generation_invalidates_success_and_receipt_before_any_fallible_preflight",
        },
    },
    Mutation {
        id: 152,
        file: "xtask/src/lib.rs",
        find: "        Task::QualifyVmware { kernel } => vmware::qualify(&paths, kernel),",
        replace: "        Task::QualifyVmware { kernel } => { let _ = kernel; Ok(()) },",
        breaks: "VMware qualification is a successful no-op",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::generation_invalidates_success_and_receipt_before_any_fallible_preflight",
        },
    },
    Mutation {
        id: 153,
        file: "xtask/src/cli.rs",
        find: "        \"qualify-vmware\" => Ok(Task::QualifyVmware {\n            kernel: if shipped {",
        replace: "        \"qualify-vmware\" => Ok(Task::QualifyVmware {\n            kernel: if false && shipped {",
        breaks: "shipped VMware evidence is qualified as self-test evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::task_specific_flags_reach_their_task",
        },
    },
    Mutation {
        id: 393,
        file: "xtask/src/paths.rs",
        find: "    \"panic-record\",",
        replace: "",
        breaks: "panic-record host tests fall out of the supported gate",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "paths::tests::host_packages_are_exactly_the_workspace_default_members",
        },
    },
    Mutation {
        id: 442,
        file: "xtask/src/gate.rs",
        find: "    CargoPass {\n        name: \"kernel refuse-pci\",\n        packages: paths::KERNEL_PACKAGES,\n        target: Some(paths::KERNEL_TARGET),\n        features: Some(paths::REFUSE_PCI_FEATURE_SPEC),\n    },\n",
        replace: "",
        breaks: "a kernel feature graph leaves every Cargo-driven gate stage",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::the_ci_registry_drives_help_and_documentation",
        },
    },
    Mutation {
        id: 443,
        file: "xtask/src/gate.rs",
        find: "    Step::ShippedBoot,\n    Step::DegradedBoot,",
        replace: "    Step::ShippedBoot,\n    Step::ShippedBoot,\n    Step::DegradedBoot,",
        breaks: "the local gate silently gains a duplicate shipped-image boot",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::the_ci_registry_drives_help_and_documentation",
        },
    },
    Mutation {
        id: 454,
        file: "xtask/src/unsafety.rs",
        find: "        self.lines += span.end().line.saturating_sub(span.start().line) + 1;",
        replace: "        self.lines += 0;",
        breaks: "the unsafe budget counter ignores every operation's span",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::a_block_is_counted_and_a_declaration_is_not",
        },
    },
    Mutation {
        id: 526,
        file: "xtask/src/model.rs",
        find: "    ensure!(
        stderr.trim().is_empty(),
        \"TLC wrote diagnostic output beside its verdict\"
    );",
        replace: "    let _ = stderr;",
        breaks: "an exact TLC marker can be mixed with fatal diagnostics",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::a_tlc_verdict_is_complete_unambiguous_and_exact",
        },
    },
    Mutation {
        id: 527,
        file: "xtask/src/model.rs",
        find: "    ensure!(
        !lines.iter().any(|line| {
            line.starts_with(\"Error:\")
                && line != &BEHAVIOR
                && !(line.starts_with(\"Error: Invariant \") && line.ends_with(\" is violated.\"))
        }),
        \"TLC reported an unrelated error beside its verdict\"
    );",
        replace: "    let _ = &lines;",
        breaks: "the expected TLC invariant marker hides a second unrelated error",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::an_unrelated_tlc_error_is_rejected_before_verdict_shape_validation",
        },
    },
    Mutation {
        id: 528,
        file: "xtask/src/model.rs",
        find: "        !lines.iter().any(|line| is_runtime_failure(line)),",
        replace: "        true,",
        breaks: "a plausible TLC verdict can hide a Java throwable or fatal worker record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::a_tlc_runtime_failure_is_rejected_before_verdict_shape_validation",
        },
    },
    Mutation {
        id: 529,
        file: "xtask/src/model.rs",
        find: "    ensure!(
        trace_is_exact,
        \"TLC terminal verdict has no one exact matching behavior trace\"
    );",
        replace: "    let _ = trace_is_exact;",
        breaks: "a TLC terminal marker can qualify with a missing or contradictory behavior trace",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::a_tlc_verdict_is_complete_unambiguous_and_exact",
        },
    },
    Mutation {
        id: 530,
        file: "xtask/src/model.rs",
        find: "{} is missing; see tools/tla/README.md for where it comes from",
        replace: "{} is missing; see models/README.md for where it comes from",
        breaks: "a missing model checker points to a nonexistent recovery document",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::a_missing_model_checker_points_to_its_vendoring_instructions",
        },
    },
    Mutation {
        id: 534,
        file: "xtask/src/baseline.rs",
        find: "        if value >= self.minimum && value <= self.maximum {",
        replace: "        if value >= self.minimum || value <= self.maximum {",
        breaks: "a performance threshold has no lower or upper side",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "baseline::tests::baseline_math_and_bands_are_two_sided",
        },
    },
    Mutation {
        id: 535,
        file: "xtask/src/cli.rs",
        find: "        \"baseline\" => Ok(Task::Baseline),",
        replace: "        \"baseline-disabled\" => Ok(Task::Baseline),",
        breaks: "the documented foundation baseline command is not runnable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_documented_xtask_invocation_parses",
        },
    },
    Mutation {
        id: 541,
        file: "xtask/src/cli.rs",
        find: "        \"debug\" => Ok(Task::Debug(if check {",
        replace: "        \"debug-disabled\" => Ok(Task::Debug(if check {",
        breaks: "the documented native debugger command is not runnable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_documented_xtask_invocation_parses",
        },
    },
    Mutation {
        id: 547,
        file: "xtask/src/cli.rs",
        find: "        \"vmware\" => Ok(Task::Vmware {\n            profile,\n            kernel: if shipped {",
        replace: "        \"vmware\" => Ok(Task::Vmware {\n            profile,\n            kernel: if false && shipped {",
        breaks: "the shipped VMware selector silently generates another self-test image",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::task_specific_flags_reach_their_task",
        },
    },
    Mutation {
        id: 651,
        file: "xtask/src/model.rs",
        find: "    crate::bounded_process::output(command, timeout, label)",
        replace: "    crate::bounded_process::output(command, timeout.saturating_mul(2), label)",
        breaks: "the TLC process outlives its named direct-child deadline",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::a_tlc_process_has_a_direct_child_return_deadline",
        },
    },
    Mutation {
        id: 667,
        file: "xtask/src/model.rs",
        find: "    ensure!(\n        description.len() > 2 && description.starts_with('<') && description.ends_with('>'),\n        \"TLC trace state has no canonical action description\"\n    );",
        replace: "    let _unchecked_description = description;",
        breaks: "a forged TLC state description satisfies the numeric trace grammar",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::a_tlc_verdict_is_complete_unambiguous_and_exact",
        },
    },
    Mutation {
        id: 678,
        file: "xtask/src/model.rs",
        find: "fn canonical_model_scratch(scratch: &Path) -> Result<PathBuf> {\n    crate::mutate::workspace::canonical_existing_real_directory(\n        scratch,\n        \"model-check scratch directory\",\n    )\n}",
        replace: "fn canonical_model_scratch(scratch: &Path) -> Result<PathBuf> {\n    std::fs::canonicalize(scratch)\n        .with_context(|| format!(\"canonicalizing {}\", scratch.display()))\n}",
        breaks: "a redirected TLC metadir is accepted as recursive-reset authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::a_redirected_model_scratch_is_refused_without_touching_outside_bytes",
        },
    },
    Mutation {
        id: 700,
        file: "xtask/src/model.rs",
        find: "        transcripts.len() == RUNS.len(),",
        replace: "        true,",
        breaks: "model qualification can omit one registered TLC execution",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::qualified_transcripts_bind_every_registered_run_and_stream",
        },
    },
    Mutation {
        id: 701,
        file: "xtask/src/model.rs",
        find: "        ensure!(
            transcript.artifact == run.artifact,
            \"model qualification transcripts are missing or reordered\"
        );
        verify_transcript(run.artifact.name, &transcript.bytes)?;",
        replace: "        let _unchecked_registered_run = run;
        verify_transcript(transcript.artifact.name, &transcript.bytes)?;",
        breaks: "model qualification can reorder complete valid TLC executions",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::qualified_transcripts_bind_every_registered_run_and_stream",
        },
    },
    Mutation {
        id: 738,
        file: "xtask/src/boundary.rs",
        find: "        \"acpi-tables\",\n        \"eevdf\",\n        \"elf64\",",
        replace: "        \"acpi-tables\",\n        \"elf64\",",
        breaks: "every host foundation library has an explicit hostile-boundary audit policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "boundary::tests::every_foundation_library_has_an_explicit_boundary_policy",
        },
    },
    Mutation {
        id: 794,
        file: "xtask/src/tasks.rs",
        find: r#"    ensure!(
        directory == root.join("vm"),
        "VMware cleanup target is not the exact allowlisted repository child"
    );"#,
        replace: "    let _unchecked_cleanup_target = (root, directory);",
        breaks: "recursive cleanup can delete a repository sibling instead of the exact VMware root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::recursive_vm_cleanup_is_confined_to_one_real_repository_child",
        },
    },
    Mutation {
        id: 795,
        file: "xtask/src/tasks.rs",
        find: "    let mut options = fs_at::OpenOptions::default();\n    options.read(true).follow(false);\n    let mut directory_handle = match options.open_dir_at(&repository, \"vm\") {",
        replace: "    let mut options = fs_at::OpenOptions::default();\n    options.read(true).follow(true);\n    let mut directory_handle = match options.open_dir_at(&repository, \"vm\") {",
        breaks: "recursive cleanup follows a redirected VMware directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::recursive_vm_cleanup_refuses_redirected_paths_without_touching_the_target",
        },
    },
    Mutation {
        id: 1173,
        file: "xtask/src/model.rs",
        find: "            report.as_bytes(),\n            error.as_bytes(),",
        replace: "            error.as_bytes(),\n            report.as_bytes(),",
        breaks: "model transcripts swap the classified stdout and stderr streams before publication",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        id: 1211,
        file: "xtask/src/model.rs",
        find: r#"    ensure!(
        captured <= maximum,
        "TLC transcript exceeded the bounded child capture limit"
    );"#,
        replace: r#"    ensure!(
        true,
        "TLC transcript exceeded the bounded child capture limit"
    );"#,
        breaks: "model transcript construction ignores its captured-stream ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::model_transcript_construction_refuses_capture_beyond_its_ceiling",
        },
    },
    Mutation {
        id: 1212,
        file: "xtask/src/model.rs",
        find: r#"    ensure!(
        captured <= maximum,
        "model transcript payload exceeds the bounded child capture limit"
    );"#,
        replace: r#"    ensure!(
        true,
        "model transcript payload exceeds the bounded child capture limit"
    );"#,
        breaks: "model transcript verification trusts a payload beyond the child capture ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::model_transcript_verification_refuses_claimed_capture_beyond_its_ceiling",
        },
    },
    Mutation {
        id: 1317,
        file: "xtask/src/paths.rs",
        find: "fn collect_directory_entries<T>(\n    entries: impl Iterator<Item = std::io::Result<T>>,\n) -> std::io::Result<Vec<T>> {\n    entries.collect()\n}",
        replace: "fn collect_directory_entries<T>(\n    entries: impl Iterator<Item = std::io::Result<T>>,\n) -> std::io::Result<Vec<T>> {\n    Ok(entries.filter_map(|entry| entry.ok()).collect())\n}",
        breaks: "a directory enumeration error silently shrinks an all-files proof domain",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "paths::tests::directory_enumeration_errors_are_not_hidden",
        },
    },
    Mutation {
        id: 1318,
        file: "xtask/src/decisions.rs",
        find: "    std::fs::read_to_string(path)\n        .unwrap_or_else(|reason| panic!(\"reading searchable file {}: {reason}\", path.display()))",
        replace: "    std::fs::read_to_string(path).unwrap_or_default()",
        breaks: "an unreadable source silently leaves the decision-citation proof domain",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "decisions::tests::an_unreadable_searchable_file_fails_the_citation_proof",
        },
    },
    Mutation {
        id: 1319,
        file: "xtask/src/paths.rs",
        find: "fn collect_classified_entries<T>(entries: impl Iterator<Item = Result<T>>) -> Result<Vec<T>> {\n    entries.collect()\n}",
        replace: "fn collect_classified_entries<T>(entries: impl Iterator<Item = Result<T>>) -> Result<Vec<T>> {\n    Ok(entries.filter_map(|entry| entry.ok()).collect())\n}",
        breaks: "a classification error silently shrinks an all-files proof domain",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "paths::tests::directory_classification_errors_are_not_hidden",
        },
    },
    Mutation {
        id: 1320,
        file: "xtask/src/paths.rs",
        find: "    !is_symlink && !is_reparse_point && (is_dir || is_file)",
        replace: "    is_dir || is_file",
        breaks: "a redirected entry enters a repository proof walk",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "paths::tests::repository_entry_classification_rejects_redirects_and_special_files",
        },
    },
    Mutation {
        id: 1321,
        file: "xtask/src/paths.rs",
        find: "    !is_symlink && !is_reparse_point && (is_dir || is_file)",
        replace: "    !is_symlink && !is_reparse_point",
        breaks: "a special entry enters a repository proof walk",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "paths::tests::repository_entry_classification_rejects_redirects_and_special_files",
        },
    },
    Mutation {
        id: 1366,
        file: "xtask/src/prose/tests.rs",
        find: "character.is_alphanumeric() || matches!(character, '_' | '-')",
        replace: "character.is_alphanumeric() || character == '_'",
        breaks: "a then-current historical count is mistaken for a current numeric claim",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::current_numeric_history_is_distinct_from_a_current_claim",
        },
    },
    Mutation {
        id: 1367,
        file: "xtask/src/cli.rs",
        find: "    qualify-vmware    as above (validates a fresh human-boot transcript)",
        replace: "                      VMware transcript qualification omitted",
        breaks: "the exceptional-task contract omits VMware transcript qualification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::every_exceptional_task_is_named_in_the_not_in_ci_block",
        },
    },
    Mutation {
        id: 1389,
        file: "xtask/src/main.rs",
        find: r#"    let args = std::env::args_os().skip(1);
    match xtask::cli::parse(args) {
        Ok(task) => match xtask::run(task) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error:?}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("error: {error}\n\n{}", xtask::cli::help());
            ExitCode::from(2)
        }
    }"#,
        replace: "    ExitCode::default()",
        breaks: "the xtask process boundary reports success without parsing or dispatching its command",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "parse_refusals_fail_at_the_process_boundary",
        },
    },
    Mutation {
        id: 1448,
        file: "xtask/src/cli.rs",
        find: "            (true, false, None) => Ok(Task::QualifyMutantsGrade(MutantsGradeAction::Run)),",
        replace: "            (true, _, _) => Ok(Task::QualifyMutantsGrade(MutantsGradeAction::Run)),",
        breaks: "the grade task accepts a second incompatible qualification action",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::external_qualification_actions_are_typed_and_task_local",
        },
    },
    Mutation {
        id: 1449,
        file: "xtask/src/cli.rs",
        find: "            (false, true, None) => Ok(Task::QualifySecurityDeep(SecurityDeepAction::Begin)),",
        replace: "            (_, _, None) => Ok(Task::QualifySecurityDeep(SecurityDeepAction::Begin)),",
        breaks: "the Deep task begins when no action was requested",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::external_qualification_actions_are_typed_and_task_local",
        },
    },
    Mutation {
        id: 1450,
        file: "xtask/src/cli.rs",
        find: "    ) && (run || begin || verify.is_some())",
        replace: "    ) && (run && begin && verify.is_some())",
        breaks: "external qualification options leak onto other tasks",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::external_qualification_actions_are_typed_and_task_local",
        },
    },
    Mutation {
        id: 1599,
        file: "xtask/src/prose/tests.rs",
        find: "    Ok(if live { \"READY\" } else { \"CLOSED\" })",
        replace: "        let _ = live;\n        Ok(\"READY\")",
        breaks: "completed historical handoff owners can never close their disposition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::historical_handoffs_close_only_after_every_owner_finishes",
        },
    },
    Mutation {
        id: 1600,
        file: "xtask/src/prose/tests.rs",
        find: "    Ok(if live { \"READY\" } else { \"CLOSED\" })",
        replace: "        let _ = live;\n        Ok(\"CLOSED\")",
        breaks: "a historical handoff closes while one finite owner is still live",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::historical_handoffs_close_only_after_every_owner_finishes",
        },
    },
    Mutation {
        id: 1601,
        file: "xtask/src/prose/tests.rs",
        find: "            _ => anyhow::bail!(\"historical handoff owner {owner} has invalid state {state}\"),",
        replace: "            _ => live = true,",
        breaks: "a stopped or invalid historical handoff owner is treated as ordinary live work",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::historical_handoffs_close_only_after_every_owner_finishes",
        },
    },
    Mutation {
        id: 1743,
        file: "xtask/src/cli.rs",
        find: "                } else if let Some(value) = flag.strip_prefix(\"--verify=\") {",
        replace: "                } else if let Some(value) = flag.strip_prefix(\"--seal=\").or_else(|| flag.strip_prefix(\"--verify=\")) {",
        breaks: "the removed --seal spelling is silently accepted as diagnostic verification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::unsigned_external_result_directories_have_no_seal_action",
        },
    },
    Mutation {
        id: 1767,
        file: "xtask/src/gate.rs",
        find: "    let readme = crate::qualification::read_bounded_text(&readme_path, README_BYTES, \"CI README\")?;",
        replace: "    let readme = std::fs::read_to_string(&readme_path).context(\"reading README.md\")?;",
        breaks: "CI registry preflight follows and reads an unbounded README",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "gate::tests::ci_readme_refuses_a_redirected_file",
        },
    },
    Mutation {
        id: 1773,
        file: "xtask/src/tasks.rs",
        find: "    before_remove();\n    let mut remaining = MAX_VM_CLEANUP_ENTRIES;\n    empty_open_directory(&mut directory_handle, directory, 0, &mut remaining)?;",
        replace: "    drop(directory_handle);\n    drop(repository);\n    before_remove();\n    std::fs::remove_dir_all(directory)\n        .with_context(|| format!(\"removing {}\", directory.display()))?;",
        breaks: "recursive cleanup discards its directory capabilities before destructive path resolution",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::recursive_vm_cleanup_stays_bound_when_a_validated_ancestor_is_replaced",
        },
    },
    Mutation {
        id: 1774,
        file: "xtask/src/tasks.rs",
        find: "        ensure!(\n            *remaining != 0,\n            \"VMware cleanup exceeded its {MAX_VM_CLEANUP_ENTRIES}-entry limit below {}\",\n            display_path.display()\n        );\n        *remaining -= 1;",
        replace: "        let _unbounded_cleanup = *remaining;",
        breaks: "recursive cleanup has no bound on attacker-created directory entries",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::recursive_vm_cleanup_has_an_entry_budget_before_deleting",
        },
    },
    Mutation {
        id: 1775,
        file: "xtask/src/tasks.rs",
        find: "    ensure!(\n        depth <= MAX_VM_CLEANUP_DEPTH,\n        \"VMware cleanup exceeded its {MAX_VM_CLEANUP_DEPTH}-directory depth limit below {}\",\n        display_path.display()\n    );",
        replace: "    let _unbounded_depth = depth;",
        breaks: "recursive cleanup has no bound on attacker-controlled directory depth",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::recursive_vm_cleanup_has_a_depth_bound_before_deleting",
        },
    },
    Mutation {
        id: 1785,
        file: "xtask/src/test_support.rs",
        find: "    reserve_directory_with(&std::env::temp_dir(), label, &NEXT_DIRECTORY)",
        replace: "    reserve_directory_with(\n        Path::new(env!(\"CARGO_MANIFEST_DIR\"))\n            .parent()\n            .unwrap(),\n        label,\n        &NEXT_DIRECTORY,\n    )",
        breaks: "cargo-mutants command fixtures inherit Cargo configuration from the compiling checkout",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::command_fixtures_do_not_inherit_the_compiling_checkout",
        },
    },
    Mutation {
        id: 1882,
        file: "xtask/src/cli.rs",
        find: "            (true, false, None) => Ok(Task::QualifySecurityDeep(SecurityDeepAction::Run)),",
        replace: "            (true, _, None) => Ok(Task::QualifySecurityDeep(SecurityDeepAction::Run)),",
        breaks: "the owned Deep action can be combined with the diagnostic begin action",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::external_qualification_actions_are_typed_and_task_local",
        },
    },
    Mutation {
        id: 1883,
        file: "xtask/src/json.rs",
        find: "            for (index, (name, value)) in object.iter().enumerate() {",
        replace: "            for (index, (name, value)) in object.iter().rev().enumerate() {",
        breaks: "deterministic JSON object fields render in reverse rather than canonical key order",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "json::tests::rendered_json_is_compact_sorted_and_round_trips",
        },
    },
    Mutation {
        id: 1884,
        file: "xtask/src/json.rs",
        find: "            '\"' => rendered.push_str(\"\\\\\\\"\"),",
        replace: "            '\"' => rendered.push('\"'),",
        breaks: "JSON string quotes are emitted without escaping",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "json::tests::rendered_strings_escape_delimiters_controls_and_preserve_unicode",
        },
    },
    Mutation {
        id: 1885,
        file: "xtask/src/json.rs",
        find: "            '\\\\' => rendered.push_str(\"\\\\\\\\\"),",
        replace: "            '\\\\' => rendered.push('\\\\'),",
        breaks: "JSON string backslashes are emitted without escaping",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "json::tests::rendered_strings_escape_delimiters_controls_and_preserve_unicode",
        },
    },
    Mutation {
        id: 1886,
        file: "xtask/src/json.rs",
        find: "            character if character <= '\\u{001f}' => {",
        replace: "            character if character < '\\u{001f}' => {",
        breaks: "U+001F can enter rendered JSON as a raw control character",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "json::tests::rendered_strings_escape_delimiters_controls_and_preserve_unicode",
        },
    },
    Mutation {
        id: 1887,
        file: "xtask/src/json.rs",
        find: "        JsonValue::Number(number) => {\n            let parsed = parse_json(number.as_bytes(), \"rendered JSON number\")?;\n            ensure!(\n                matches!(&parsed, JsonValue::Number(parsed) if parsed == number),\n                \"rendered JSON number is not an isolated number\"\n            );\n            rendered.push_str(number);\n        }",
        replace: "        JsonValue::Number(number) => rendered.push_str(number),",
        breaks: "manually constructed invalid JSON numbers bypass renderer validation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "json::tests::invalid_number_lexemes_cannot_escape_the_renderer",
        },
    },
    Mutation {
        id: 1888,
        file: "xtask/src/json.rs",
        find: "    ensure!(depth <= 128, \"rendered JSON nesting is excessive\");",
        replace: "    ensure!(depth <= usize::MAX, \"rendered JSON nesting is excessive\");",
        breaks: "manually constructed JSON can exceed the renderer nesting limit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "json::tests::manually_constructed_json_obeys_the_nesting_limit",
        },
    },
    Mutation {
        id: 1896,
        file: "xtask/src/json.rs",
        find: "        while self\n            .peek()\n            .is_some_and(|byte| matches!(byte, b' ' | b'\\t' | b'\\r' | b'\\n'))\n        {",
        replace: "        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {",
        breaks: "vertical-tab and form-feed bytes are accepted as JSON whitespace",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "json::tests::parser_accepts_only_the_four_json_whitespace_bytes",
        },
    },
    Mutation {
        id: 1980,
        file: "xtask/src/main.rs",
        find: "    if let Some(code) = xtask::dispatch_private_codex_wrapper() {\n        std::process::exit(code);\n    }\n",
        replace: "",
        breaks: "private provider children pass through the ordinary task parser instead of the early wrapper dispatcher",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "security_deep::tests::wrapper::wrapper_dispatch_is_early_and_strips_control_environment",
        },
    },
    Mutation {
        id: 2077,
        file: "xtask/src/tasks.rs",
        find: "    qemu::observe_selftest(paths, image.path(), timeout)",
        replace: "    qemu::observe_selftest(paths, image.path(), selftest_timeout(profile))",
        breaks: "the mutation observer discards its selected deadline before starting QEMU",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::the_mutation_observer_forwards_its_caller_deadline",
        },
    },
    Mutation {
        id: 2086,
        file: "xtask/src/tasks.rs",
        find: "    if succeeded {\n        return Ok(false);\n    }",
        replace: "    if true {\n        return Ok(false);\n    }",
        breaks: "a failed Cargo clean can be reported complete without validating its residual tree",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::a_failed_clean_requires_the_exact_live_executable_residual",
        },
    },
    Mutation {
        id: 2087,
        file: "xtask/src/tasks.rs",
        find: "    ensure!(\n        target_entries.len() == 1,\n        \"failed Cargo clean left an unexpected target-root entry\"\n    );",
        replace: "    ensure!(\n        !target_entries.is_empty(),\n        \"failed Cargo clean left an unexpected target-root entry\"\n    );",
        breaks: "a failed Cargo clean can leave additional target-root artifacts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::a_failed_clean_rejects_an_extra_target_entry",
        },
    },
    Mutation {
        id: 2088,
        file: "xtask/src/tasks.rs",
        find: "    ensure!(\n        profile_entries.len() == 1,\n        \"failed Cargo clean left an unexpected xtask-profile entry\"\n    );",
        replace: "    ensure!(\n        !profile_entries.is_empty(),\n        \"failed Cargo clean left an unexpected xtask-profile entry\"\n    );",
        breaks: "a failed Cargo clean can leave additional xtask-profile artifacts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::a_failed_clean_rejects_an_extra_xtask_profile_entry",
        },
    },
    Mutation {
        id: 2089,
        file: "xtask/src/tasks.rs",
        find: "    ensure!(\n        std::fs::canonicalize(&executable)\n            .context(\"canonicalizing the retained running clean executable\")?\n            == current_executable,\n        \"failed Cargo clean retained an executable other than its running image\"\n    );",
        replace: "    ensure!(\n        true,\n        \"failed Cargo clean retained an executable other than its running image\"\n    );",
        breaks: "a foreign executable can be accepted as the sole failed-clean residual",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::a_failed_clean_rejects_a_foreign_executable_identity",
        },
    },
    Mutation {
        id: 2090,
        file: "xtask/src/tasks.rs",
        find: "    let _running =\n        crate::mutate::workspace::open_real_file(&executable, \"retained running clean executable\")?;",
        replace: "    let _running = &executable;",
        breaks: "a non-file or redirected runner can be accepted as the sole failed-clean residual",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::a_failed_clean_rejects_a_non_file_runner",
        },
    },
    Mutation {
        id: 2091,
        file: "xtask/src/tasks.rs",
        find: "    let target =\n        crate::mutate::workspace::canonical_existing_real_directory(target, \"Cargo clean target\")?;",
        replace: "    let target = std::fs::canonicalize(target)\n        .context(\"canonicalizing Cargo clean target without reparse validation\")?;",
        breaks: "a redirected target directory can become the authority for a failed cleanup",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::a_failed_clean_rejects_a_redirected_target_directory",
        },
    },
    Mutation {
        id: 2092,
        file: "xtask/src/tasks.rs",
        find: "    ensure!(\n        crate::mutate::workspace::metadata_is_real_directory(&profile_metadata)\n            && std::fs::canonicalize(&profile)\n                .context(\"canonicalizing retained xtask profile directory\")?\n                == profile,\n        \"failed Cargo clean retained a redirected xtask profile directory\"\n    );",
        replace: "    let _redirected_profile = profile_metadata;\n    ensure!(\n        std::fs::canonicalize(&profile).is_ok(),\n        \"failed Cargo clean retained a redirected xtask profile directory\"\n    );",
        breaks: "a redirected xtask profile can supply the sole apparent failed-clean residual",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::a_failed_clean_rejects_a_redirected_xtask_profile",
        },
    },
    Mutation {
        id: 2156,
        file: "xtask/src/test_support.rs",
        find: "        match std::fs::create_dir(&candidate) {",
        replace: "        match std::fs::create_dir_all(&candidate) {",
        breaks: "shared host-test fixture reservation reopens a stale process directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "test_support::tests::stale_candidates_are_skipped_without_touching_their_bytes",
        },
    },
    Mutation {
        id: 2415,
        file: "xtask/src/cli.rs",
        find: "    if profile_option\n",
        replace: "    if false && profile_option\n",
        breaks: "a profile option reaches a task that discards it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::profile_options_only_reach_tasks_with_a_build_profile",
        },
    },
    Mutation {
        id: 2416,
        file: "xtask/src/cli.rs",
        find: "                profile = Profile::Release;\n                profile_option = true;",
        replace: "                profile = Profile::Release;\n                profile_option = false;",
        breaks: "an explicit release profile disappears before task validation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::profile_options_only_reach_tasks_with_a_build_profile",
        },
    },
    Mutation {
        id: 2417,
        file: "xtask/src/cli.rs",
        find: "                profile = Profile::Dev;\n                profile_option = true;",
        replace: "                profile = Profile::Dev;\n                profile_option = false;",
        breaks: "an explicit debug profile disappears before task validation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::profile_options_only_reach_tasks_with_a_build_profile",
        },
    },
    Mutation {
        id: 2418,
        file: "xtask/src/cli.rs",
        find: "    if check && !matches!(name.as_str(), \"fmt\" | \"debug\") {",
        replace: "    if false && check && !matches!(name.as_str(), \"fmt\" | \"debug\") {",
        breaks: "a check option silently dispatches cleanup or another unrelated task",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::check_options_cannot_dispatch_cleanup_or_another_unrelated_task",
        },
    },
    Mutation {
        id: 2419,
        file: "xtask/src/cli.rs",
        find: "    if all && name != \"clean\" {",
        replace: "    if false && all && name != \"clean\" {",
        breaks: "an all option reaches a task that cannot consume it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::all_options_only_expand_the_clean_task",
        },
    },
    Mutation {
        id: 2420,
        file: "xtask/src/cli.rs",
        find: "    if shipped && !matches!(name.as_str(), \"vmware\" | \"qualify-vmware\") {",
        replace: "    if false && shipped && !matches!(name.as_str(), \"vmware\" | \"qualify-vmware\") {",
        breaks: "a shipped selector reaches a non-VMware task",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::shipped_options_only_select_vmware_images",
        },
    },
    Mutation {
        id: 2421,
        file: "xtask/src/cli.rs",
        find: "    if message_format.is_some() && name != \"check\" {",
        replace: "    if false && message_format.is_some() && name != \"check\" {",
        breaks: "a message format reaches a task that discards it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::message_format_options_only_reach_check_even_when_empty",
        },
    },
    Mutation {
        id: 2422,
        file: "xtask/src/cli.rs",
        find: "    if passthrough_option && name != \"run\" {",
        replace: "    if false && passthrough_option && name != \"run\" {",
        breaks: "passthrough arguments silently dispatch a task other than run",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::passthrough_is_run_only_even_when_the_separator_has_no_tail",
        },
    },
    Mutation {
        id: 2423,
        file: "xtask/src/cli.rs",
        find: "    if passthrough_option && name != \"run\" {",
        replace: "    if !passthrough.is_empty() && name != \"run\" {",
        breaks: "an empty passthrough separator evades task validation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::passthrough_is_run_only_even_when_the_separator_has_no_tail",
        },
    },
    Mutation {
        id: 2424,
        file: "xtask/src/cli.rs",
        find: "    if message_format.is_some() && name != \"check\" {",
        replace: "    if message_format.as_ref().is_some_and(|value| !value.is_empty()) && name != \"check\" {",
        breaks: "an empty message format evades task validation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::message_format_options_only_reach_check_even_when_empty",
        },
    },
    Mutation {
        id: 2425,
        file: "xtask/src/cli.rs",
        find: "            \"-h\" | \"--help\" => return Ok(Task::Help),",
        replace: "            \"-h\" | \"--help\" => bail!(\"help disabled\"),",
        breaks: "immediate help is replaced by unrelated option refusal",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::help_and_repeated_ordinary_options_keep_their_existing_precedence",
        },
    },
    Mutation {
        id: 2445,
        file: "xtask/src/decisions.rs",
        find: "    if directory\n        .try_exists()\n        .with_context(|| format!(\"probing {}\", directory.display()))?\n    {",
        replace: "    if false {",
        breaks: "a split decision directory is ignored in favour of the index",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "decisions::tests::the_split_layout_defines_one_decision_per_file",
        },
    },
    Mutation {
        id: 2446,
        file: "xtask/src/decisions.rs",
        find: "        listed == expected,",
        replace: "        listed.len() == expected.len(),",
        breaks: "the decision index may list its files out of order or under other titles",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "decisions::tests::an_index_that_disagrees_with_the_files_is_refused",
        },
    },
    Mutation {
        id: 2447,
        file: "xtask/src/decisions.rs",
        find: "number == filed && title",
        replace: "title",
        breaks: "a decision file may carry another decision's number in its heading",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "decisions::tests::a_decision_file_out_of_shape_is_refused",
        },
    },
    Mutation {
        id: 2448,
        file: "xtask/src/decisions.rs",
        find: "        && lines\n            .next()\n            .is_some_and(|line| line.starts_with(\"Status: \"))",
        replace: "        && lines.next().is_some()",
        breaks: "a decision file may omit its status line",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "decisions::tests::a_decision_file_out_of_shape_is_refused",
        },
    },
    Mutation {
        id: 2449,
        file: "xtask/src/decisions.rs",
        find: ".strip_suffix(\".md\")?",
        replace: "",
        breaks: "a decision file need not be Markdown",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "decisions::tests::a_decision_file_out_of_shape_is_refused",
        },
    },
    Mutation {
        id: 2450,
        file: "xtask/src/prose.rs",
        find: "                pending.push(path);\n            } else if path.extension()",
        replace: "                let _ = path;\n            } else if path.extension()",
        breaks: "documents below the top of docs escape the prose checks",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::a_broken_link_below_the_top_of_docs_is_found",
        },
    },
    Mutation {
        id: 2451,
        file: "xtask/src/test_support.rs",
        find: "        let header = function.span().byte_range().start..braces.open().byte_range().start;",
        replace: "        let header = 0..braces.open().byte_range().start;",
        breaks: "a source-contract slice ends at a nested function instead of its own closing brace",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "test_support::tests::a_function_slice_ends_at_its_own_block_whatever_the_comments_say",
        },
    },
    Mutation {
        id: 2467,
        file: "xtask/src/cli.rs",
        find: "        .filter(|(distance, _)| *distance <= limit)",
        replace: "        .filter(|(distance, _)| *distance == 0)",
        breaks: "a mistyped task is refused without naming the nearest task",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::a_mistyped_task_or_run_option_says_what_was_meant",
        },
    },
    Mutation {
        id: 2468,
        file: "xtask/src/cli.rs",
        find: "                } else if name == \"run\" {\n                    bail!(\"unknown option `{flag}`; pass QEMU arguments after `--`\");",
        replace: "                } else if name.is_empty() {\n                    bail!(\"unknown option `{flag}`; pass QEMU arguments after `--`\");",
        breaks: "a QEMU argument given to run before the separator is refused without naming it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::a_mistyped_task_or_run_option_says_what_was_meant",
        },
    },
    Mutation {
        id: 2469,
        file: "xtask/src/cli.rs",
        find: "    \"qualify-vmware\",\n    \"clean\",\n];",
        replace: "    \"qualify-vmware\",\n];",
        breaks: "the suggested task names silently drift from the tasks the parser accepts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cli::tests::every_task_the_parser_accepts_is_named_in_the_help",
        },
    },
];
