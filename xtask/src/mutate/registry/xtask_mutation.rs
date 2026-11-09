use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 1,
        file: "xtask/src/mutants_grade.rs",
        find: "const COMMAND: &str = \"cargo mutants -p xtask --output <xtask-owned-directory> --jobs",
        replace: "const COMMAND: &str = \"cargo mutants -p xtask --output <xtask-owned-directory>/mutants.out --jobs",
        breaks: "the owned cargo-mutants command writes one directory deeper than the recorder reads",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::the_owned_output_directory_is_the_one_the_recorder_reads",
        },
    },
    Mutation {
        id: 2,
        file: "xtask/src/mutants_grade.rs",
        find: "        run == independent,",
        replace: "        !run.is_empty() && !independent.is_empty(),",
        breaks: "a subset cargo-mutants run can pose as the complete independently inventoried grade",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::independent_inventory_must_match_run_identities_and_diffs",
        },
    },
    Mutation {
        id: 3,
        file: "xtask/src/mutants_grade.rs",
        find: "        require_process_status(\n            json_field(phase, \"process_status\", \"cargo-mutants phase\")?,\n            expected_status,\n        )?;",
        replace: "        let _unchecked_process_status = expected_status;",
        breaks: "a cargo-mutants summary can contradict the recorded nextest phase result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::phase_summary_requires_exact_order_status_and_nextest_argv",
        },
    },
    Mutation {
        id: 4,
        file: "xtask/src/mutants_grade.rs",
        find: "            !argv[0].is_empty() && arguments == expected_arguments,",
        replace: "            !argv[0].is_empty(),",
        breaks: "cargo-mutants phase evidence can name another runner command or package",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::phase_summary_requires_exact_order_status_and_nextest_argv",
        },
    },
    Mutation {
        id: 5,
        file: "xtask/src/mutants_grade.rs",
        find: "        ensure!(\n            fields.remove(key) == Some(value),\n            \"cargo-mutants request `{key}` differs from D-324\"\n        );",
        replace: "        ensure!(fields.remove(key).is_some(), \"cargo-mutants request `{key}` is absent\");",
        breaks: "the owned grade request can weaken its exact command, environment, or pinned tools",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::request_binds_source_command_concurrency_and_runner_provenance",
        },
    },
    Mutation {
        id: 6,
        file: "xtask/src/mutants_grade.rs",
        find: "            line.starts_with(\"error:\") || line.contains(\"FAILED\")",
        replace: "            line.starts_with(\"not-an-error:\") || line.contains(\"FAILED\")",
        breaks: "a failed doctest command can retain a forged zero-test terminal summary",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::doctest_transcript_requires_only_successful_zero_test_summaries",
        },
    },
    Mutation {
        id: 14,
        file: "xtask/src/mutate/workspace/journal.rs",
        find: "        Ok(reports\n            .into_iter()\n            .max_by_key(|(_, report)| report.sequence))",
        replace: "        Ok(reports\n            .into_iter()\n            .min_by_key(|(_, report)| report.sequence))",
        breaks: "a historical passing mutation report can outrank a newer failed parent sweep",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_recorded_parent_pass_selects_the_newest_report_and_retains_the_sweep_lock",
        },
    },
    Mutation {
        id: 80,
        file: "xtask/src/mutate/workspace.rs",
        find: "        ensure!(\n            digest_tracked(&self.snapshot, &self.baseline.tracked)? == self.baseline.source_digest,\n            \"the immutable mutation source snapshot changed during the sweep\"\n        );",
        replace: "        let _snapshot_was_not_rechecked = &self.snapshot;",
        breaks: "a changed mutation snapshot can authorize a report for pristine source",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_changed_snapshot_cannot_authorize_a_report_for_pristine_source",
        },
    },
    Mutation {
        id: 103,
        file: "xtask/src/mutate/tests.rs",
        find: "    impossible.file = \"xtask/src/image/program.rs\";\n    impossible.reddens = Caught::Check(\"unused\");",
        replace: "    impossible.file = \"xtask/src/image/program.rs\";\n    let _ = &mut impossible.reddens;",
        breaks: "the mutation-driver safety test keeps a witness the running driver could rebuild",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_mutation_cannot_target_the_running_driver",
        },
    },
    Mutation {
        id: 107,
        file: "xtask/src/mutate.rs",
        find: "    let qualification_attempt = qualification::begin_mutation(paths, &registry, mutations.len())?;\n    let mut session = Session::open(paths, &registry, mutations.len())?;",
        replace: "    let mut session = Session::open(paths, &registry, mutations.len())?;\n    let qualification_attempt = qualification::begin_mutation(paths, &registry, mutations.len())?;",
        breaks: "a failed mutation preflight leaves an older passing qualification current",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        id: 108,
        file: "xtask/src/mutate.rs",
        find: "        let pass = workspace::current_pass(paths, &registry, mutations.len())?;\n        qualification_attempt.pass(&pass)?;",
        replace: "        let pass = workspace::current_pass(paths, &registry, mutations.len())?;\n        drop(qualification_attempt);",
        breaks: "a complete caught mutation sweep does not commit its qualification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        id: 371,
        file: "xtask/src/mutate/workspace.rs",
        find: "            workspace: Paths::new(workspace_root, Some(&build)),",
        replace: "            workspace: Paths::new(paths.root().to_path_buf(), Some(&build)),",
        breaks: "mutation builds run from the user checkout instead of the tracked copy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_session_builds_only_from_its_tracked_copy",
        },
    },
    Mutation {
        id: 372,
        file: "xtask/src/mutate/workspace/tree.rs",
        find: "    file.try_lock().with_context(|| {",
        replace: "    std::io::Result::<()>::Ok(()).with_context(|| {",
        breaks: "two mutation sweeps can use one sandbox concurrently",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_killed_build_or_boot_resumes_outside_the_user_tree",
        },
    },
    Mutation {
        id: 373,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "    write_source(&destination, &bytes)?;\n    ensure!(\n        read_bounded_bytes(\n            &destination,\n            MAX_TRACKED_FILE_BYTES,\n            \"restored mutation source\",\n        )? == bytes,",
        replace: "    ensure!(\n        read_bounded_bytes(\n            &destination,\n            MAX_TRACKED_FILE_BYTES,\n            \"restored mutation source\",\n        )? == bytes,",
        breaks: "resume leaves the interrupted mutation applied in its sandbox",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_killed_build_or_boot_resumes_outside_the_user_tree",
        },
    },
    Mutation {
        id: 374,
        file: "xtask/src/mutate/workspace/journal.rs",
        find: ".max_by_key(|journal| journal.generation)",
        replace: ".min_by_key(|journal| journal.generation)",
        breaks: "resume chooses stale progress over the newest complete journal generation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::journal_uses_the_newest_complete_slot_and_falls_back_from_a_torn_write",
        },
    },
    Mutation {
        id: 375,
        file: "xtask/src/mutate/workspace.rs",
        find: "        restore_workspace(\n            &self.snapshot,\n            self.workspace.root(),\n            &self.baseline.tracked,\n        )?;\n        self.journal.completed.push(outcome);",
        replace: "        self.journal.completed.push(outcome);",
        breaks: "a completed mutation leaves collateral tracked or generated files in its sandbox",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_finished_mutation_restores_the_whole_sandbox",
        },
    },
    Mutation {
        id: 376,
        file: "xtask/src/mutate/workspace/journal.rs",
        find: "            verdict: if survived == 0 && inconclusive == 0 {",
        replace: "            verdict: if true || survived == 0 && inconclusive == 0 {",
        breaks: "a failed mutation sweep is serialized as a passing report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_completed_report_binds_totals_and_verdict_to_every_outcome",
        },
    },
    Mutation {
        id: 377,
        file: "xtask/src/mutate/workspace/journal.rs",
        find: "            .max_by_key(|(_, report)| report.sequence))",
        replace: "            .min_by_key(|(_, report)| report.sequence))",
        breaks: "an older passing sweep hides the newest failed mutation report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_new_failed_sweep_supersedes_but_does_not_erase_an_older_pass",
        },
    },
    Mutation {
        id: 378,
        file: "xtask/src/mutate/workspace.rs",
        find: "    let pass_lock = acquire_lock(&state_root.join(LOCK_NAME))?;\n    let baseline = Baseline::discover(paths.root())?;",
        replace: "    let pass_lock = acquire_lock(&state_root.join(\"unrelated-lock\"))?;\n    let baseline = Baseline::discover(paths.root())?;",
        breaks: "a newer mutation sweep can supersede a pass while its consumer publishes it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_current_pass_blocks_a_newer_sweep_until_consumed",
        },
    },
    Mutation {
        id: 379,
        file: "xtask/src/mutate/workspace/journal.rs",
        find: "            self.commit == baseline.commit\n                && self.registry == registry",
        replace: "            true\n                && self.registry == registry",
        breaks: "a passing mutation report is accepted for another commit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_completed_report_binds_totals_and_verdict_to_every_outcome",
        },
    },
    Mutation {
        id: 380,
        file: "xtask/src/mutate/workspace/journal.rs",
        find: "        ensure!(\n            report.encode_with_checksum() == text,\n            \"mutation report is not in its canonical encoding\"\n        );",
        replace: "        let _ = text;",
        breaks: "a checksummed mutation report has multiple accepted byte encodings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_completed_report_binds_totals_and_verdict_to_every_outcome",
        },
    },
    Mutation {
        id: 382,
        file: "xtask/src/mutate/registry/kernel_console.rs",
        find: "            marker: \"faulting address 0x0000600000000000\",",
        replace: "            marker: \"\",",
        breaks: "a terminal mutation can name no exact panic cause",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_panicking_mutation_names_an_exact_marker_and_why_it_is_terminal",
        },
    },
    Mutation {
        id: 383,
        file: "xtask/src/mutate.rs",
        find: "    result.with_context(|| format!(\"pristine {label} mutation baseline is not green\"))",
        replace: "    let _ = (label, result);\n    Ok(())",
        breaks: "a red pristine baseline is allowed to precede mutation credit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_red_baseline_stops_before_any_mutation",
        },
    },
    Mutation {
        id: 384,
        file: "xtask/src/mutate.rs",
        find: "        (Caught::Check(name), qemu::SelftestOutcome::Failed(failed))\n            if failed.iter().any(|actual| actual == name) =>\n        {\n            Ok(true)\n        }",
        replace: "        (Caught::Check(_), qemu::SelftestOutcome::Failed(failed))\n            if !failed.is_empty() =>\n        {\n            Ok(true)\n        }",
        breaks: "an unrelated failed check is credited to the intended mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_wrong_machine_failure_is_inconclusive_not_caught",
        },
    },
    Mutation {
        id: 385,
        file: "xtask/src/mutate.rs",
        find: "        (Caught::Panic { marker, .. }, qemu::SelftestOutcome::Panicked { transcript, .. })\n            if transcript.contains(marker) =>\n        {\n            Ok(true)\n        }",
        replace: "        (Caught::Panic { .. }, qemu::SelftestOutcome::Panicked { transcript, .. })\n            if !transcript.is_empty() =>\n        {\n            Ok(true)\n        }",
        breaks: "an unrelated kernel panic is credited to a terminal mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_panic_requires_its_exact_marker",
        },
    },
    Mutation {
        id: 386,
        file: "xtask/src/mutate.rs",
        find: "        qemu::DegradedOutcome::Panicked {\n            no_x2apic_diagnosed: false,\n            transcript,\n        } => bail!(\n            \"mutation expected a prior no-x2APIC diagnosis before panic {marker:?}, but observed: {transcript}\"\n        ),\n",
        replace: "",
        breaks: "an undiagnosed degraded panic is credited to the timer mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_degraded_panic_requires_its_diagnosis_and_exact_marker",
        },
    },
    Mutation {
        id: 387,
        file: "xtask/src/mutate.rs",
        find: "        qemu::DegradedOutcome::Panicked { transcript, .. } if transcript.contains(marker) => {\n            Ok(true)\n        }",
        replace: "        qemu::DegradedOutcome::Panicked { transcript, .. } if !transcript.is_empty() => {\n            Ok(true)\n        }",
        breaks: "an unrelated degraded panic is credited to the timer mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_degraded_panic_requires_its_diagnosis_and_exact_marker",
        },
    },
    Mutation {
        id: 464,
        file: "xtask/src/mutate.rs",
        find: "    if success {\n        let passed = format!(\"test {name} ... ok\");\n        let passed = stdout\n            .lines()\n            .chain(stderr.lines())\n            .filter(|line| *line == passed)\n            .count();\n        if passed == 1 {\n            return Ok(false);\n        }\n        bail!(\"host mutation selected no exact test `{name}`\");\n    }",
        replace: "    if success {\n        return Ok(false);\n    }",
        breaks: "a successful zero-test command is accepted as an exact host witness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::an_exact_host_witness_must_run",
        },
    },
    Mutation {
        id: 465,
        file: "xtask/src/mutate/tests.rs",
        find: "                mutation.reddens,\n                Caught::HostTest {\n                    package: actual,\n                    ..\n                } if actual == package",
        replace: "                mutation.reddens,\n                Caught::HostTest { .. }",
        breaks: "a foreign package's witness counts as source-mutation coverage",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_foreign_host_witness_cannot_cover_a_librarys_source_mutation",
        },
    },
    Mutation {
        id: 542,
        file: "xtask/src/mutate.rs",
        find: "const MACHINE_OBSERVATIONS: usize = 2;\n\nconst fn machine_observation_timeout",
        replace: "const MACHINE_OBSERVATIONS: usize = 1;\n\nconst fn machine_observation_timeout",
        breaks: "one incidental machine failure can invalidate an otherwise exact mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::an_inconclusive_machine_observation_gets_one_bounded_retry",
        },
    },
    // The bytes on disk are pristine either way; only a rebuild of everything
    // that inlined them can tell the two apart.
    Mutation {
        id: 543,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "    write_source(&destination, &bytes)?;\n    ensure!(",
        replace: "    std::fs::copy(&source, &destination)\n        .map(|_| ())\n        .with_context(|| format!(\"writing {}\", destination.display()))?;\n    ensure!(",
        breaks: "a restored mutation carries its snapshot timestamp backwards",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_restored_input_is_what_the_next_build_observes",
        },
    },
    Mutation {
        id: 544,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "        let bytes = read_bounded_bytes(&source, MAX_TRACKED_FILE_BYTES, \"tracked mutation file\")?;\n        write_source(&destination, &bytes)?;",
        replace: "        std::fs::copy(&source, &destination)\n            .map(|_| ())\n            .with_context(|| format!(\"copying {}\", source.display()))?;",
        breaks: "a re-copied workspace carries its snapshot timestamps backwards",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_recopied_workspace_is_what_the_next_build_observes",
        },
    },
    Mutation {
        id: 546,
        file: "xtask/src/mutate.rs",
        find: "        (Caught::Check(name), qemu::SelftestOutcome::Panicked { failed, .. })\n            if failed.iter().any(|actual| actual == name) =>\n        {\n            Ok(true)\n        }",
        replace: "        (Caught::Check(name), qemu::SelftestOutcome::Panicked { failed, .. })\n            if false && failed.iter().any(|actual| actual == name) =>\n        {\n            Ok(true)\n        }",
        breaks: "an exact failed check loses credit when later corruption panics",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_wrong_machine_failure_is_inconclusive_not_caught",
        },
    },
    Mutation {
        id: 635,
        file: "xtask/src/mutate/workspace/tree.rs",
        find: "    let metadata = std::fs::symlink_metadata(child)\n        .with_context(|| format!(\"inspecting {}\", child.display()))?;\n    require_real_directory_metadata(child, &metadata, label)?;\n    require_canonical_child(parent, child, label)",
        replace: "    let metadata = std::fs::symlink_metadata(child)\n        .with_context(|| format!(\"inspecting {}\", child.display()))?;\n    let _unchecked_mutation_child = (metadata, label);\n    Ok(())",
        breaks: "a redirected mutation-state ancestor can reach recursive cleanup outside target",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::recursive_cleanup_revalidates_the_whole_mutation_state_ancestry",
        },
    },
    Mutation {
        id: 653,
        file: "xtask/src/mutate/workspace.rs",
        find: "        let path = require_confined_file(&self.snapshot, relative, \"mutation source file\")?;",
        replace: "        let path = self.snapshot.join(relative);",
        breaks: "a substituted snapshot parent can author the mutation input after session open",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::tracked_snapshot_and_workspace_redirects_are_refused_at_use",
        },
    },
    Mutation {
        id: 654,
        file: "xtask/src/mutate/workspace.rs",
        find: "        let path =\n            require_confined_file(self.workspace.root(), relative, \"mutation workspace file\")?;",
        replace: "        let path = self.workspace.root().join(relative);",
        breaks: "a substituted workspace parent can redirect a mutation write after session open",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::tracked_snapshot_and_workspace_redirects_are_refused_at_use",
        },
    },
    Mutation {
        id: 655,
        file: "xtask/src/mutate/workspace/tree.rs",
        find: "    require_real_file_metadata(path, &metadata, label)?;\n    require_canonical_child(parent, path, label)?;",
        replace: "    let _unchecked_direct_file = (&metadata, label);",
        breaks: "a redirected tracked file can receive an isolated mutation outside the workspace",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::tracked_snapshot_and_workspace_redirects_are_refused_at_use",
        },
    },
    Mutation {
        id: 656,
        file: "xtask/src/mutate/workspace/journal.rs",
        find: "    fn require_directory(&self) -> Result<()> {\n        ensure!(\n            canonical_existing_real_directory(&self.directory, \"mutation journal directory\")?\n                == self.directory,\n            \"mutation journal directory changed after it was opened\"\n        );\n        Ok(())\n    }",
        replace: "    fn require_directory(&self) -> Result<()> { Ok(()) }",
        breaks: "a replaced journal directory can receive durable mutation state after store open",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::journal_paths_cannot_be_substituted_after_the_store_opens",
        },
    },
    Mutation {
        id: 657,
        file: "xtask/src/mutate/workspace/journal.rs",
        find: "            if require_direct_file_if_present(&self.directory, path, label)? {\n                std::fs::remove_file(path)\n                    .with_context(|| format!(\"removing old {label} {}\", path.display()))?;\n            }\n            std::fs::rename(&temporary, path).with_context(|| {\n                format!(\n                    \"publishing {label} {} as {}\",\n                    temporary.display(),\n                    path.display()\n                )\n            })?;",
        replace: "            let _unchecked_destination =\n                require_direct_file_if_present(&self.directory, path, label)?;\n            std::fs::write(path, bytes)\n                .with_context(|| format!(\"overwriting {label} {}\", path.display()))?;",
        breaks: "a hard-linked report target can be overwritten instead of atomically replaced",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::journal_paths_cannot_be_substituted_after_the_store_opens",
        },
    },
    Mutation {
        id: 673,
        file: "xtask/src/mutate.rs",
        find: "        ensure!(\n            !success,\n            \"host mutation printed a failed verdict but Cargo exited successfully\"\n        );",
        replace: "        let _unchecked_status = success;",
        breaks: "a successful host command can forge a failed mutation witness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::an_exact_host_witness_must_run",
        },
    },
    Mutation {
        id: 674,
        file: "xtask/src/mutate.rs",
        find: "    let failed = stdout\n        .lines()\n        .chain(stderr.lines())\n        .filter(|line| *line == failed)\n        .count();",
        replace: "    let failed = stdout\n        .lines()\n        .chain(stderr.lines())\n        .filter(|line| line.contains(&failed))\n        .count();",
        breaks: "diagnostic prose containing a failed test phrase can forge a host witness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::an_exact_host_witness_must_run",
        },
    },
    Mutation {
        id: 675,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "fn write_source(destination: &Path, bytes: &[u8]) -> Result<()> {\n    write_source_with(destination, bytes, |from, to| std::fs::rename(from, to))\n}",
        replace: "fn write_source(destination: &Path, bytes: &[u8]) -> Result<()> {\n    std::fs::write(destination, bytes)\n        .with_context(|| format!(\"overwriting {}\", destination.display()))\n}",
        breaks: "a workspace mutation can overwrite an external hard-linked inode",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::workspace_replacement_never_writes_through_an_external_hardlink",
        },
    },
    Mutation {
        id: 677,
        file: "xtask/src/mutate/workspace/tree.rs",
        find: "    Ok(SweepLock { _file: file })",
        replace: "    file.set_len(0)\n        .with_context(|| format!(\"truncating mutation lock {}\", path.display()))?;\n    Ok(SweepLock { _file: file })",
        breaks: "a mutation lock can truncate an external hard-linked inode",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_mutation_lock_never_writes_through_an_external_hardlink",
        },
    },
    Mutation {
        id: 690,
        file: "xtask/src/mutate/workspace.rs",
        find: "        write_source(&path, mutated.as_bytes())\n            .with_context(|| format!(\"writing isolated mutation {}\", path.display()))",
        replace: "        std::fs::write(&path, mutated)\n            .with_context(|| format!(\"writing isolated mutation {}\", path.display()))",
        breaks: "an active mutation session can overwrite an external hard-linked inode",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::workspace_replacement_never_writes_through_an_external_hardlink",
        },
    },
    Mutation {
        id: 731,
        file: "xtask/src/mutate/workspace.rs",
        find: "    ensure!(\n        initial <= maximum,\n        \"{label} is {initial} bytes, exceeding the {maximum}-byte allocation limit\"\n    );",
        replace: "    ensure!(\n        true,\n        \"{label} is {initial} bytes, exceeding the {MAX_STATE_FILE_BYTES}-byte allocation limit\"\n    );",
        breaks: "an oversized mutation-state file can drive allocation before its cap is enforced",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::oversized_mutation_state_is_refused_before_payload_allocation",
        },
    },
    Mutation {
        id: 743,
        file: "xtask/src/mutate.rs",
        find: "    let label = format!(\"host mutation test {package}::{name}\");\n    let output = run(command, HOST_TEST_TIMEOUT, &label)?;",
        replace: "    let label = format!(\"host mutation test {package}::{name}\");\n    let output = match run(command, HOST_TEST_TIMEOUT, &label) {\n        Ok(output) => output,\n        Err(_) => return Ok(true),\n    };",
        breaks: "a timed-out host mutation is falsely reported as caught",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_parked_host_mutation_is_inconclusive_not_caught",
        },
    },
    Mutation {
        id: 796,
        file: "xtask/src/mutate/workspace/tree.rs",
        find: "    let mut options = fs_at::OpenOptions::default();\n    options.read(true).follow(false);\n    for component in components {",
        replace: "    let mut options = fs_at::OpenOptions::default();\n    options.read(true).follow(true);\n    for component in components {",
        breaks: "recursive cleanup accepts a real final directory behind a redirected repository ancestor",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::recursive_vm_cleanup_refuses_redirected_paths_without_touching_the_target",
        },
    },
    Mutation {
        id: 806,
        file: "xtask/src/mutate/workspace/source.rs",
        find: r#"        let (_, destination) =
            prepare_confined_destination(&root, relative, "mutation copy destination")?;"#,
        replace: r#"        let destination = root.join(relative);
        std::fs::create_dir_all(
            destination.parent().context("mutation copy destination has no parent")?,
        )?;"#,
        breaks: "initial mutation copy creates nested parents before proving confinement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::initial_copy_refuses_a_nested_redirect_before_creating_outside_paths",
        },
    },
    Mutation {
        id: 807,
        file: "xtask/src/mutate/workspace/source.rs",
        find: r#"    let (parent, destination) =
        prepare_confined_destination(&workspace, relative, "mutation restore destination")?;"#,
        replace: r#"    let destination = workspace.join(relative);
    let parent = destination
        .parent()
        .context("mutation restore destination has no parent")?
        .to_path_buf();"#,
        breaks: "mutation restore accepts same-byte external state before confinement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::restore_refuses_same_byte_external_state_before_its_fast_path",
        },
    },
    Mutation {
        id: 808,
        file: "xtask/src/mutate/workspace/source.rs",
        find: r#"    if let Err(error) = result {
        restore_previous_file(parent, destination, previous.as_deref(), label)
            .with_context(|| format!("{error:#}; rolling back failed {label} publication"))?;
        return Err(error);
    }"#,
        replace: r#"    if let Err(error) = result {
        return Err(error);
    }"#,
        breaks: "a failed staged source publication strands the previous workspace authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_failed_source_publication_restores_the_previous_authority",
        },
    },
    Mutation {
        id: 809,
        file: "xtask/src/mutate/workspace/source.rs",
        find: r#"            if require_direct_file_if_present(parent, staging, label).with_context(|| {
                format!(
                    "after {operation_error:#}; validating failed {label} {}",
                    staging.display()
                )
            })? {"#,
        replace: r#"            if require_direct_file_if_present(parent, staging, label)
                .with_context(|| {
                    format!(
                        "after {operation_error:#}; validating failed {label} {}",
                        staging.display()
                    )
                })
                .unwrap_or(false)
            {"#,
        breaks: "a failed stage cleanup trusts a hostile replacement as absence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::failed_stage_cleanup_rejects_a_hostile_stage_replacement",
        },
    },
    Mutation {
        id: 810,
        file: "xtask/src/mutate/workspace/source.rs",
        find: r#"                remove(staging).with_context(|| {
                    format!(
                        "after {operation_error:#}; removing failed {label} {}",
                        staging.display()
                    )
                })?;"#,
        replace: "                let _ignored_cleanup_failure = remove(staging);",
        breaks: "a failed stage silently discards its cleanup refusal",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::failed_stage_cleanup_reports_both_operation_and_removal_failures",
        },
    },
    Mutation {
        id: 816,
        file: "xtask/src/mutate/tests.rs",
        find: "        if node.method == \"check\" && report_receiver {",
        replace: "        if node.method == \"check\" {",
        breaks: "a decoy method call can forge a current machine-check identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::machine_check_inventory_rejects_decoys_and_nonliteral_names",
        },
    },
    Mutation {
        id: 916,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "            length <= MAX_TRACKED_FILE_BYTES,",
        replace: "            true,",
        breaks: "one oversized tracked mutation source reaches the hashing stream",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::oversized_tracked_sources_are_refused_before_streaming",
        },
    },
    Mutation {
        id: 917,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "            total <= MAX_TRACKED_TREE_BYTES,",
        replace: "            true,",
        breaks: "the tracked mutation source tree has no aggregate byte ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::tracked_source_trees_have_an_independent_total_size_bound",
        },
    },
    Mutation {
        id: 918,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "            .update(&(name.len() as u64).to_le_bytes())",
        replace: "            .update(&0u64.to_le_bytes())",
        breaks: "the tracked-source digest omits its path-length frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::streamed_tracked_source_digest_preserves_the_framed_schema",
        },
    },
    Mutation {
        id: 1172,
        file: "xtask/src/mutate/workspace.rs",
        find: "    ensure!(\n        Report::parse(text)? == *selected,\n        \"mutation report changed while selecting the current pass\"\n    );",
        replace: "    let _unchecked_selected_report = (selected, text);",
        breaks: "the current mutation pass digest can bind bytes from a substituted report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_current_pass_cannot_bind_bytes_from_a_substituted_report",
        },
    },
    Mutation {
        id: 1205,
        file: "xtask/src/mutate/workspace.rs",
        find: r#"    ensure!(
        bytes.len() as u64 <= maximum,
        "{label} grew beyond the {maximum}-byte allocation limit while being read"
    );"#,
        replace: r#"    ensure!(
        true,
        "{label} grew beyond the {maximum}-byte allocation limit while being read"
    );"#,
        breaks: "a mutation-state file can grow beyond its allocation ceiling after measurement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::mutation_state_growth_is_refused_after_measurement",
        },
    },
    Mutation {
        id: 1217,
        file: "xtask/src/mutate/workspace/source.rs",
        find: r#"    ensure!(
        initial <= maximum,
        "{label} is {initial} bytes, exceeding its {maximum}-byte limit"
    );"#,
        replace: r#"    ensure!(
        true,
        "{label} is {initial} bytes, exceeding its {maximum}-byte limit"
    );"#,
        breaks: "a mutation authority digest streams an initially oversized file",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::mutation_digest_refuses_an_initial_file_beyond_its_ceiling",
        },
    },
    Mutation {
        id: 1218,
        file: "xtask/src/mutate/workspace/source.rs",
        find: r#"    ensure!(
        observed <= maximum,
        "{label} grew beyond its {maximum}-byte limit while being hashed"
    );"#,
        replace: r#"    ensure!(
        true,
        "{label} grew beyond its {maximum}-byte limit while being hashed"
    );"#,
        breaks: "a mutation authority digest can grow beyond its streaming ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::mutation_digest_refuses_growth_after_measurement",
        },
    },
    Mutation {
        id: 1219,
        file: "xtask/src/mutate/workspace/source.rs",
        find: r#"    ensure!(
        observed == initial
            && file
                .metadata()
                .with_context(|| format!("remeasuring {label} {}", path.display()))?
                .len()
                == initial,
        "{label} changed length while being hashed"
    );"#,
        replace: r#"    ensure!(true, "{label} changed length while being hashed");"#,
        breaks: "a mutation authority digest accepts truncation after its preflight measurement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::mutation_digest_refuses_truncation_after_measurement",
        },
    },
    Mutation {
        id: 1380,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "            ensure!(\n                previous < raw,\n                \"Git tracked paths are not unique and sorted\"\n            );",
        replace: "            ensure!(\n                previous != raw,\n                \"Git tracked paths are not unique and sorted\"\n            );",
        breaks: "a reversed Git tracked-path inventory is accepted as sorted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::tracked_path_order_refuses_duplicates_and_reversals",
        },
    },
    Mutation {
        id: 1382,
        file: "xtask/src/mutate.rs",
        find: "    let expected = format!(\"self-test did not finish within {seconds} seconds\");\n    if error.to_string() == expected {\n        Ok(true)",
        replace: "    let expected = format!(\"self-test did not finish within {seconds} seconds\");\n    if error.to_string() != expected {\n        Ok(true)",
        breaks: "a foreign machine error can masquerade as the registered terminal timeout",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_timeout_requires_its_exact_deadline_and_terminal_rationale",
        },
    },
    Mutation {
        id: 1413,
        file: "xtask/src/mutants_grade.rs",
        find: "    command\n        .current_dir(paths.root())\n        .args([\"mutants\", \"--version\"]);",
        replace: "    command.current_dir(paths.root()).arg(\"--version\");",
        breaks: "the direct cargo-mutants version probe omits its subcommand entry point",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::direct_cargo_mutants_probe_names_its_subcommand",
        },
    },
    Mutation {
        id: 1414,
        file: "xtask/src/mutate/registry/repository.rs",
        find: "        id: 18,\n",
        replace: "        id: 17,\n",
        breaks: "two registered mutations share one permanent identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::every_mutation_identity_is_permanent_and_accounted_for",
        },
    },
    Mutation {
        id: 1415,
        file: "xtask/src/mutate/registry/xtask_mutation.rs",
        find: "    Mutation {\n        id: 1,\n",
        replace: "    Mutation {\n        id: 60000,\n",
        breaks: "a registered mutation carries an identity the counter never issued",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::every_mutation_identity_is_permanent_and_accounted_for",
        },
    },
    Mutation {
        id: 1416,
        file: "xtask/src/mutate/workspace/journal.rs",
        find: "            body.push_str(&format!(\"id {id} \"));",
        replace: "            let _ = id;",
        breaks: "the durable mutation report omits which mutation each outcome names",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_report_names_each_outcome_by_its_permanent_mutation_identity",
        },
    },
    Mutation {
        id: 1422,
        file: "xtask/src/mutants_grade.rs",
        find: "        (\"tool.cargo-mutants.sha256\", CARGO_MUTANTS_EXECUTABLE_SHA256),",
        replace: "        (\n            \"tool.cargo-mutants.sha256\",\n            fields.get(\"tool.cargo-mutants.sha256\").copied().unwrap_or_default(),\n        ),",
        breaks: "the external grade request accepts any grader digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::request_binds_source_command_concurrency_and_runner_provenance",
        },
    },
    Mutation {
        id: 1423,
        file: "xtask/src/mutants_grade.rs",
        find: "        packages == [\"xtask\"],",
        replace: "        !packages.is_empty(),",
        breaks: "a doctest transcript from another package satisfies the xtask grade",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::doctest_transcript_belongs_to_exactly_the_xtask_package",
        },
    },
    Mutation {
        id: 1424,
        file: "xtask/src/mutants_grade.rs",
        find: "const CAMPAIGN_ROOT: &str = \"qualification-runs/cargo-mutants\";",
        replace: "const CAMPAIGN_ROOT: &str = \"qualification/cargo-mutants-grade\";",
        breaks: "the owned campaign writes raw output inside authoritative qualification records",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::campaign_scratch_is_exclusively_reserved_under_target_and_removed",
        },
    },
    Mutation {
        id: 1425,
        file: "xtask/src/mutants_grade.rs",
        find: "        !bytes.starts_with(&[0xFF, 0xFE]) && !bytes.starts_with(&[0xFE, 0xFF]),",
        replace: "        true,",
        breaks: "UTF-16 source-controlled policy reaches the grade parser undiagnosed",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::source_controlled_policy_is_utf8_without_a_byte_order_mark",
        },
    },
    Mutation {
        id: 1426,
        file: "xtask/src/mutants_grade.rs",
        find: "    grade.join(CARGO_MUTANTS_OUTPUT)",
        replace: "    grade.to_path_buf()",
        breaks: "the recorder reads the campaign directory instead of the cargo-mutants output inside it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::the_owned_output_directory_is_the_one_the_recorder_reads",
        },
    },
    Mutation {
        id: 1427,
        file: "xtask/src/mutants_grade.rs",
        find: "    ensure!(start < end, \"cargo-mutants completion precedes its start\");",
        replace: "    ensure!(start <= end, \"cargo-mutants completion precedes its start\");",
        breaks: "a zero-length cargo-mutants run qualifies",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::a_grade_ends_after_it_starts_and_recounts_every_outcome",
        },
    },
    Mutation {
        id: 1428,
        file: "xtask/src/mutants_grade.rs",
        find: "            \"cargo-mutants success count\"\n        )? == 0,",
        replace: "            \"cargo-mutants success count\"\n        )? <= 1,",
        breaks: "a cargo-mutants run may report unclassified successful mutants",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::a_grade_ends_after_it_starts_and_recounts_every_outcome",
        },
    },
    Mutation {
        id: 1429,
        file: "xtask/src/mutants_grade.rs",
        find: "        (\"caught\", caught),\n",
        replace: "        (\"total_mutants\", total),\n",
        breaks: "the caught counter is not recomputed from the outcomes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::a_grade_ends_after_it_starts_and_recounts_every_outcome",
        },
    },
    Mutation {
        id: 1430,
        file: "xtask/src/mutants_grade.rs",
        find: "        baseline == 1,",
        replace: "        baseline >= 1,",
        breaks: "a grade with two pristine baselines qualifies",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::a_grade_has_one_baseline_and_an_outcome_for_every_candidate",
        },
    },
    Mutation {
        id: 1431,
        file: "xtask/src/mutants_grade.rs",
        find: "        observed == candidates.keys().cloned().collect(),",
        replace: "        observed.len() <= candidates.len(),",
        breaks: "a candidate with no outcome passes the grade",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::a_grade_has_one_baseline_and_an_outcome_for_every_candidate",
        },
    },
    Mutation {
        id: 1432,
        file: "xtask/src/mutants_grade.rs",
        find: "            \"MissedMutant\" => disposition == \"equivalent\",",
        replace: "            \"MissedMutant\" => !disposition.is_empty(),",
        breaks: "a missed mutant is adjudicated by a reason other than equivalence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::adjudications_name_exactly_the_noncaught_mutants_with_allowed_reasons",
        },
    },
    Mutation {
        id: 1433,
        file: "xtask/src/mutants_grade.rs",
        find: "            .trim()\n            .is_empty(),\n            \"cargo-mutants adjudication has no rationale\"",
        replace: "            .is_empty(),\n            \"cargo-mutants adjudication has no rationale\"",
        breaks: "a whitespace rationale satisfies an adjudication",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::adjudications_name_exactly_the_noncaught_mutants_with_allowed_reasons",
        },
    },
    Mutation {
        id: 1434,
        file: "xtask/src/mutants_grade.rs",
        find: "                .is_none(),\n            \"cargo-mutants adjudication is duplicated\"",
        replace: "                .is_none_or(|_| true),\n            \"cargo-mutants adjudication is duplicated\"",
        breaks: "one mutant may be adjudicated twice",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::adjudications_name_exactly_the_noncaught_mutants_with_allowed_reasons",
        },
    },
    Mutation {
        id: 1435,
        file: "xtask/src/mutants_grade.rs",
        find: "        observed == *expected,",
        replace: "        expected.keys().all(|name| observed.contains_key(name)),",
        breaks: "an adjudication may name a mutant that was caught",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::adjudications_name_exactly_the_noncaught_mutants_with_allowed_reasons",
        },
    },
    Mutation {
        id: 1494,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "    let mut command = crate::git::resolved()?.command(root);\n    command.args(args);\n    let output = crate::bounded_process::output_with_limit(command, GIT_TIMEOUT, label, maximum)",
        replace: "    let mut command = std::process::Command::new(\"git\");\n    crate::bounded_process::isolate_git_environment(&mut command);\n    command.current_dir(root).args(args);\n    let output = crate::bounded_process::output_with_limit(command, GIT_TIMEOUT, label, maximum)",
        breaks: "the mutation sweep asks whatever `git` PATH names about its source",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::sources::every_source_identity_names_the_git_that_established_it",
        },
    },
    Mutation {
        id: 1497,
        file: "xtask/src/mutate/workspace/tree.rs",
        find: "        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)\n",
        replace: "",
        breaks: "a checked artifact is opened through a link swapped in after its check",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::an_artifact_swapped_for_a_link_after_its_check_is_never_read",
        },
    },
    Mutation {
        id: 1498,
        file: "xtask/src/mutate/workspace/tree.rs",
        find: "    require_real_file_metadata(path, &metadata, label)?;\n    require_opened_entry(path, &metadata, label)?;",
        replace: "    require_opened_entry(path, &metadata, label)?;",
        breaks: "an opened link is read without proving the handle names a real file",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::an_artifact_swapped_for_a_link_after_its_check_is_never_read",
        },
    },
    Mutation {
        id: 1745,
        file: "xtask/src/mutants_grade.rs",
        find: "            || normalized.starts_with(\"RUST\")",
        replace: "            || false",
        breaks: "caller-controlled Rust compiler wrappers and flags reach the owned campaign",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::hostile_compiler_cargo_mutants_and_nextest_environment_is_removed",
        },
    },
    Mutation {
        id: 1746,
        file: "xtask/src/mutants_grade.rs",
        find: "        .env(\n            \"CARGO_ENCODED_RUSTFLAGS\",\n            format!(\n                \"-C\\u{1f}dlltool={}\",\n                crate::cargo::child_process_path(Path::new(&tools.dlltool.path)).display()\n            ),\n        )",
        replace: "        .env(\"CARGO_ENCODED_RUSTFLAGS\", \"\")",
        breaks: "the campaign loses its exact dlltool compiler contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::owned_commands_are_exactly_the_documented_ones",
        },
    },
    Mutation {
        id: 1747,
        file: "xtask/src/mutants_grade.rs",
        find: "fn execute_campaign(command: Command) -> Result<std::process::Output> {\n    crate::bounded_process::output_with_limit(\n        command,\n        CAMPAIGN_TIMEOUT,\n        \"complete cargo-mutants campaign\",\n        CAMPAIGN_CAPTURE_BYTES,\n    )\n}",
        replace: "fn execute_campaign(mut command: Command) -> Result<std::process::Output> {\n    command.output().context(\"running uncontained cargo-mutants campaign\")\n}",
        breaks: "the multi-day cargo-mutants process tree escapes xtask's bounded job owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::owned_commands_are_exactly_the_documented_ones",
        },
    },
    Mutation {
        id: 1748,
        file: "xtask/src/mutants_grade.rs",
        find: "        status.code() == Some(expected_code),",
        replace: "        true,",
        breaks: "a cargo-mutants campaign exit can contradict its adjudicated outcome counters",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::campaign_exit_codes_match_adjudicated_outcomes",
        },
    },
    Mutation {
        id: 1749,
        file: "xtask/src/mutants_grade.rs",
        find: "        expected_source == observed_source,\n        \"the qualification source changed during the cargo-mutants campaign\"",
        replace: "        true,\n        \"the qualification source changed during the cargo-mutants campaign\"",
        breaks: "a cargo-mutants result is recorded after the qualification source changes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::only_a_successful_unchanged_campaign_and_tracked_policy_can_pass",
        },
    },
    Mutation {
        id: 1750,
        file: "xtask/src/mutants_grade.rs",
        find: "        expected_tools == observed_tools,\n        \"cargo-mutants or cargo-nextest changed during the campaign\"",
        replace: "        true,\n        \"cargo-mutants or cargo-nextest changed during the campaign\"",
        breaks: "a cargo-mutants result is recorded after a campaign tool changes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::only_a_successful_unchanged_campaign_and_tracked_policy_can_pass",
        },
    },
    Mutation {
        id: 1751,
        file: "xtask/src/mutants_grade.rs",
        find: "        sealed == source,\n        \"sealed cargo-mutants adjudications differ from the source-controlled policy\"",
        replace: "        true,\n        \"sealed cargo-mutants adjudications differ from the source-controlled policy\"",
        breaks: "caller-supplied cargo-mutants adjudications replace the tracked policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::only_a_successful_unchanged_campaign_and_tracked_policy_can_pass",
        },
    },
    Mutation {
        id: 1752,
        file: "xtask/src/mutants_grade.rs",
        find: "                        diff: diff.to_string(),",
        replace: "                        diff: expected.get(name).map_or_else(|| diff.to_string(), |row| row.diff.clone()),",
        breaks: "an adjudication can name the right mutant while silently replacing its exact diff",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::adjudications_name_exactly_the_noncaught_mutants_with_allowed_reasons",
        },
    },
    Mutation {
        id: 1753,
        file: "xtask/src/mutants_grade.rs",
        find: "        directory.parent() == Some(parent.as_path()) && directory.starts_with(&parent),",
        replace: "        directory.starts_with(&parent),",
        breaks: "successful campaign cleanup can recursively delete a nested caller-selected directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::campaign_scratch_is_exclusively_reserved_under_target_and_removed",
        },
    },
    Mutation {
        id: 1754,
        file: "xtask/src/mutants_grade.rs",
        find: "        fields.remove(\"adjudication.policy.sha256\")\n            == Some(sha256::hex(&sha256::digest(adjudications)).as_str()),",
        replace: "        fields.remove(\"adjudication.policy.sha256\").is_some(),",
        breaks: "the campaign request no longer binds the source-controlled adjudication policy bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::request_binds_source_command_concurrency_and_runner_provenance",
        },
    },
    Mutation {
        id: 1755,
        file: "xtask/src/mutants_grade.rs",
        find: "    require_campaign_identity(\n        attempt.source(),\n        &qualification::source_identity(paths.root())?,\n        &tools,\n        &discover_tools(paths, &private_temp)?,\n    )?;",
        replace: "",
        breaks: "the final campaign tool use is not followed by source and executable remeasurement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::final_identity_check_follows_every_authoritative_tool_use_and_precedes_pass",
        },
    },
    Mutation {
        id: 1756,
        file: "xtask/src/mutants_grade.rs",
        find: "        semantic == expected,\n        \"repository-parent Cargo configuration contains semantics outside the exact pinned dlltool compatibility setting: observed {semantic:?}, expected {expected:?}\"",
        replace: "        true,\n        \"repository-parent Cargo configuration contains semantics outside the exact pinned dlltool compatibility setting: observed {semantic:?}, expected {expected:?}\"",
        breaks: "repository-parent Cargo configuration can inject compiler or test semantics",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::parent_cargo_configuration_is_semantically_allowlisted",
        },
    },
    Mutation {
        id: 1757,
        file: "xtask/src/mutants_grade.rs",
        find: "    for name in [\"TEMP\", \"TMP\", \"TMPDIR\"] {\n        command.env(name, &private_temp);\n    }",
        replace: "    for name in std::iter::empty::<&str>() {\n        command.env(name, &private_temp);\n    }",
        breaks: "cargo-mutants can select an attacker-controlled OS temp tree and its parent Cargo configuration",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::owned_temp_root_controls_cargo_configuration_discovery",
        },
    },
    Mutation {
        id: 1758,
        file: "xtask/src/mutants_grade.rs",
        find: "        require_no_cargo_config(\n            &ancestor.join(\".cargo\"),\n            \"generated cargo-mutants temp discovery chain\",\n        )?;",
        replace: "        let _ = ancestor;",
        breaks: "a Cargo configuration above the owned scratch workspace can alter mutation results",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::owned_temp_root_controls_cargo_configuration_discovery",
        },
    },
    Mutation {
        id: 2045,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "            [\"rev-parse\", \"--path-format=absolute\", \"--git-path\", \"index\"],",
        replace: "            [\"rev-parse\", \"--path-format=relative\", \"--git-path\", \"index\"],",
        breaks: "the hardened Git index query can return a process-directory-relative name to an absolute-only confinement boundary",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::a_session_builds_only_from_its_tracked_copy",
        },
    },
    Mutation {
        id: 2046,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "    ensure!(\n        path.is_absolute(),\n        \"{label} is not an absolute path: {}\",\n        path.display()\n    );",
        replace: "    let _unchecked_absolute_path = path;",
        breaks: "a process-directory-relative Git control-file name can enter workspace confinement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::an_absolute_git_path_must_resolve_to_one_confined_direct_file",
        },
    },
    Mutation {
        id: 2047,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "    let relative_parent = parent\n        .strip_prefix(&root)\n        .with_context(|| format!(\"{label} is outside {}\", root.display()))?;",
        replace: "    let relative_parent = parent\n        .strip_prefix(&parent)\n        .with_context(|| format!(\"{label} is outside {}\", root.display()))?;",
        breaks: "an absolute Git control-file path outside the workspace can be rebound to a same-named file inside it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::an_absolute_git_path_must_resolve_to_one_confined_direct_file",
        },
    },
    Mutation {
        id: 2076,
        file: "xtask/src/mutate.rs",
        find: "const fn machine_observation_timeout(wanted: Caught) -> Duration {\n    match wanted {\n        Caught::Timeout { seconds, .. } => Duration::from_secs(seconds),\n        _ => tasks::MUTATION_SELFTEST_TIMEOUT,\n    }\n}",
        replace: "const fn machine_observation_timeout(wanted: Caught) -> Duration {\n    match wanted {\n        Caught::Timeout { seconds, .. } => Duration::from_secs(seconds),\n        _ => Duration::from_secs(60),\n    }\n}",
        breaks: "ordinary machine mutations lose the extended observation budget needed to reach late exact witnesses",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_timeout_requires_its_exact_deadline_and_terminal_rationale",
        },
    },
    Mutation {
        id: 2261,
        file: "xtask/src/mutate.rs",
        find: "        .env(\"TEMP\", &temporary)\n        .env(\"TMP\", &temporary)\n        .env(\"TMPDIR\", &temporary)\n",
        replace: "",
        breaks: "host mutation witnesses share the inherited temporary directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::each_host_mutation_witness_gets_a_new_empty_temporary_directory",
        },
    },
    Mutation {
        id: 2322,
        file: "xtask/src/mutants_grade.rs",
        find: "        grade.caught.checked_add(grade.adjudicated) == Some(grade.total),",
        replace: "        true,",
        breaks: "the campaign exit accepts missing or surplus non-caught adjudications",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::campaign_exit_does_not_substitute_for_complete_adjudication",
        },
    },
    Mutation {
        id: 2323,
        file: "xtask/src/mutants_grade.rs",
        find: "    let expected_code = if grade.timeout > 0 {\n        3\n    } else if grade.missed > 0 {\n        2",
        replace: "    let expected_code = if grade.missed > 0 {\n        2\n    } else if grade.timeout > 0 {\n        3",
        breaks: "missed mutants take exit precedence over the pinned grader's timeouts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::campaign_exit_codes_match_adjudicated_outcomes",
        },
    },
    Mutation {
        id: 2324,
        file: "xtask/src/mutants_grade.rs",
        find: "    } else if grade.missed > 0 {\n        2",
        replace: "    } else if grade.missed > 0 {\n        0",
        breaks: "a missed-mutant campaign is accepted with the all-caught exit code",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::campaign_exit_codes_match_adjudicated_outcomes",
        },
    },
    Mutation {
        id: 2325,
        file: "xtask/src/mutants_grade.rs",
        find: "    } else if grade.missed > 0 {",
        replace: "    } else if grade.missed > 0 || grade.unviable > 0 {",
        breaks: "unviable mutants incorrectly require the missed-mutant exit code",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::campaign_exit_codes_match_adjudicated_outcomes",
        },
    },
    Mutation {
        id: 2326,
        file: "xtask/src/mutants_grade.rs",
        find: "    require_campaign_success(result.status, &grade, &root)?;",
        replace: "    let _ = result.status;",
        breaks: "the owned campaign can reach final identity verification without its exit boundary",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutants_grade::tests::final_identity_check_follows_every_authoritative_tool_use_and_precedes_pass",
        },
    },
    Mutation {
        id: 2410,
        file: "xtask/src/mutate/workspace.rs",
        find: "fn read_bounded_bytes_with<F>(\n    path: &Path,\n    maximum: u64,\n    label: &str,\n    after_measurement: F,\n) -> Result<Vec<u8>>\nwhere\n    F: FnOnce() -> Result<()>,\n{\n    let mut file = open_real_file(path, label)?;\n",
        replace: "fn read_bounded_bytes_with<F>(\n    path: &Path,\n    maximum: u64,\n    label: &str,\n    after_measurement: F,\n) -> Result<Vec<u8>>\nwhere\n    F: FnOnce() -> Result<()>,\n{\n    let mut file = File::open(path)?;\n",
        breaks: "read_bounded_bytes_with admits a final-component link instead of a checked real-file handle",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::bounded_mutation_reader_refuses_a_final_component_link",
        },
    },
    Mutation {
        id: 2411,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "fn digest_file_with<F>(\n    path: &Path,\n    maximum: u64,\n    label: &str,\n    after_measurement: F,\n) -> Result<String>\nwhere\n    F: FnOnce() -> Result<()>,\n{\n    let mut file = open_real_file(path, label)?;\n",
        replace: "fn digest_file_with<F>(\n    path: &Path,\n    maximum: u64,\n    label: &str,\n    after_measurement: F,\n) -> Result<String>\nwhere\n    F: FnOnce() -> Result<()>,\n{\n    let mut file = File::open(path)?;\n",
        breaks: "digest_file_with admits a final-component link instead of a checked real-file handle",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::workspace::tests::mutation_file_digest_refuses_a_final_component_link",
        },
    },
    Mutation {
        id: 2412,
        file: "xtask/src/mutate/workspace/source.rs",
        find: "        let mut file = open_real_file(&path, \"tracked mutation source\")?;",
        replace: "        let mut file = File::open(&path)?;",
        breaks: "tracked source hashing follows a path after confinement validation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::sources::a_checked_path_is_opened_only_without_following_links",
        },
    },
];
