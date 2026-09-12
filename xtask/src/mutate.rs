//! Persistent deliberate defects and the exact checks required to catch them.
//!
//! The cheap gate requires every source pattern to occur exactly once. The
//! complete task applies each mutation in a clean, commit-pinned, journaled
//! workspace, proves the witness class green first, and then requires the
//! mutation's exact check, panic marker, boot marker or host-test verdict to go
//! red. Another failure is inconclusive, and a selected-zero-tests host command
//! proves nothing. Machine entries stay outside CI because each costs a boot.

use anyhow::{Context, Result, bail, ensure};
use std::process::{Command, Output};
use std::time::Duration;

use crate::mutation_workspace::{Outcome, Session};
use crate::paths::Paths;
use crate::{qemu, qualification, sha256, tasks};

/// One deliberate defect, and what it is a defect *in*.
#[derive(Debug, Clone, Copy)]
pub struct Mutation {
    /// Workspace-relative path.
    pub file: &'static str,
    /// The exact text to replace. Must occur exactly once.
    pub find: &'static str,
    /// What to put there.
    pub replace: &'static str,
    /// The property this removes, in the register a check name is written in.
    ///
    /// Human-facing, and deliberately *not* the check's name: only four of
    /// these are, and the paraphrases read better in a progress line than the
    /// checks they stand for. [`Mutation::reddens`] carries the name.
    pub breaks: &'static str,
    /// The check this must turn red, or why nothing names it.
    pub reddens: Caught,
}

/// What a mutation must produce, and how precisely it is required.
#[derive(Debug, Clone, Copy)]
pub enum Caught {
    /// The exact `report.check("…")` name that must appear as `FAILED`.
    Check(&'static str),
    /// An exact host test whose own FAILED verdict must name the defect.
    HostTest {
        /// Cargo package containing the test.
        package: &'static str,
        /// Fully qualified test name accepted by `--exact`.
        name: &'static str,
    },
    /// The compiler-resolved API audit must name this exact dead item.
    ApiAudit(&'static str),
    /// A shipped degraded boot must emit one exact serial marker.
    BootMarker(&'static str),
    /// A degraded boot must diagnose its absent APIC before this exact panic.
    DegradedPanic {
        /// Exact text which must occur in the serial panic transcript.
        marker: &'static str,
        /// Why a terminal outcome is the defect's expected witness.
        why: &'static str,
    },
    /// The defect kills the machine before the verdict.
    Panic {
        /// Exact text which must occur in the serial panic line.
        marker: &'static str,
        /// Why a terminal outcome is the defect's expected witness.
        why: &'static str,
    },
}

/// The defects, each of which some check must notice.
///
/// Chosen to be *load-bearing rather than merely different*: every one removes
/// a guard, a bound or a strip that something depends on, and none is a
/// cosmetic edit that would fail a build instead. A mutation that cannot
/// compile tests nothing.
pub const MUTATIONS: &[Mutation] = &[
    Mutation {
        file: "docs/017-pre-wizzy-hostile-surface-audit.md",
        find: "| `PW-006` | Re-attack the final P0/P1 source after the admitted fixes and bind every disposition, break-test and evidence artifact in the phase-close ADR. |",
        replace: "| `PW006` | Re-attack the final P0/P1 source after the admitted fixes and bind every disposition, break-test and evidence artifact in the phase-close ADR. |",
        breaks: "the hostile-surface census omits a live register obligation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_live_register_row_has_a_hostile_surface_disposition",
        },
    },
    Mutation {
        file: "docs/014-storage.md",
        find: "| Mutations | 1359 registered; 285 in the last successful complete sweep; the current complete sweep caught 1301, with 34 survivors and 24 inconclusive |",
        replace: "| Mutations | 1359 registered; 285 in a successful complete sweep; the current complete sweep caught 1301, with 34 survivors and 24 inconclusive |\n<!-- last successful complete sweep -->",
        breaks: "the phase-close table borrows its successful-sweep label from unrelated prose",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::the_phase_close_says_what_the_source_says",
        },
    },
    Mutation {
        file: "docs/014-storage.md",
        find: "the current complete sweep caught 1301",
        replace: "the current complete sweep caught 285",
        breaks: "the phase-close table hides the current failed mutation sweep behind an older pass",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::the_phase_close_says_what_the_source_says",
        },
    },
    Mutation {
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
        file: "xtask/src/qualification.rs",
        find: "        relative: \"qualification/advisory/record\",",
        replace: "        relative: \"qualification/advisory/record-omitted\",",
        breaks: "offline evidence omits the RustSec qualification record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::qualification_records_cannot_be_omitted_or_changed_offline",
        },
    },
    Mutation {
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
        file: "xtask/src/qualification.rs",
        find: "        .args([\"--deny\", \"warnings\", \"--file\"])",
        replace: "        .args([\"--deny\", \"none\", \"--file\"])",
        breaks: "the RustSec run does not deny advisory warnings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory_refresh_and_review_commands_are_direct_exact_and_bounded",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    invalidate(paths, Kind::Advisory)?;",
        replace: "    let _preserved_advisory_pass = paths;",
        breaks: "a failed advisory attempt can preserve an older green record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory_invalidation_precedes_even_failed_tool_resolution",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    verify_advisory_qualification(paths, root, &source)?;",
        replace: "    let _omitted_advisory_qualification = (&source, root);",
        breaks: "offline evidence can omit the required advisory qualification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_missing_advisory_record_is_rejected",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    require_source(&record, source)?;\n    require_advisory_inputs(paths, &record)?;",
        replace: "    let _unchecked_advisory_source = source;\n    require_advisory_inputs(paths, &record)?;",
        breaks: "an advisory result can be donated by another source commit and tree",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::an_advisory_record_bound_to_another_source_is_rejected",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        record.get(\"input.cargo-lock.bytes\")? == lock.bytes.to_string()\n            && record.get(\"input.cargo-lock.sha256\")? == lock.sha256,\n        \"advisory qualification belongs to another Cargo.lock\"\n    );",
        replace: "    let _unchecked_advisory_lock = (record, lock);",
        breaks: "a current source can reuse an advisory result for different locked bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_stale_advisory_record_cannot_describe_another_lockfile",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            ensure!(\n                bytes == ADVISORY_POLICY.as_bytes(),\n                \"advisory policy differs from the no-ignore phase-close policy\"\n            );",
        replace: "            ensure!(!bytes.is_empty(), \"advisory policy is empty\");",
        breaks: "an ignored RustSec advisory can enter a phase-close result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_advisory_policy_and_result_bytes_are_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        lines[1] == \"exit.code=0\",\n        \"cargo-audit transcript does not record a successful result\"\n    );",
        replace: "    ensure!(lines[1].starts_with(\"exit.code=\"), \"cargo-audit transcript has no result\");",
        breaks: "a failed cargo-audit execution can be reframed as a successful transcript",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_advisory_policy_and_result_bytes_are_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            ADVISORY_TRANSCRIPT => (MAX_ADVISORY_TRANSCRIPT_BYTES, false),",
        replace: "            ADVISORY_TRANSCRIPT => (u64::MAX, false),",
        breaks: "an oversized advisory transcript reaches payload allocation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            record.get(\"database.url\")? == ADVISORY_DATABASE_URL\n                && is_git_id(record.get(\"database.revision\")?)\n                && is_git_id(record.get(\"database.tree\")?),",
        replace: "            record.get(\"database.url\")? == ADVISORY_DATABASE_URL,",
        breaks: "a malformed advisory database revision can enter the qualification record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory_schema_binds_database_time_command_and_tool_identity",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            record.get(\"execution.command\")? == ADVISORY_COMMAND\n                && record.get(\"execution.exit-code\")? == \"0\"\n                && record.get(\"execution.refresh-command\")? == ADVISORY_REFRESH_COMMAND\n                && record.get(\"execution.refresh-exit-code\")? == \"0\",",
        replace: "            record.get(\"execution.exit-code\")? == \"0\"\n                && record.get(\"execution.refresh-command\")? == ADVISORY_REFRESH_COMMAND\n                && record.get(\"execution.refresh-exit-code\")? == \"0\",",
        breaks: "an advisory record can name a weaker or stale command line",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory_schema_binds_database_time_command_and_tool_identity",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            observed != 0 && observed.to_string() == record.get(\"observation.utc-unix-seconds\")?,",
        replace: "            observed.to_string() == record.get(\"observation.utc-unix-seconds\")?,",
        breaks: "an advisory record can carry no real UTC observation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory_schema_binds_database_time_command_and_tool_identity",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    let version = report\n        .strip_prefix(\"cargo-audit-audit \")\n        .or_else(|| report.strip_prefix(\"cargo-audit \"))\n        .context(\"cargo-audit printed no supported version identity\")?;",
        replace: "    let version = report;",
        breaks: "another executable can pose as the resolved RustSec scanner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::cargo_audit_version_is_one_exact_resolved_identity",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        report == ADVISORY_TOOL_VERSION,\n        \"cargo-audit version differs from the report schema qualification\"\n    );",
        replace: "    let _unqualified_advisory_tool_version = report;",
        breaks: "an unqualified cargo-audit release can emit a different report schema",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::cargo_audit_version_is_one_exact_resolved_identity",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        origin == ADVISORY_DATABASE_URL,\n        \"advisory database origin differs from the RustSec authority\"\n    );",
        replace: "    let _unchecked_advisory_origin = origin;",
        breaks: "another Git remote can pose as the RustSec advisory database",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory_database_identity_rejects_forged_origin_and_dirty_bytes",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(status.is_empty(), \"advisory database checkout is dirty\");",
        replace: "    let _unchecked_advisory_status = status;",
        breaks: "uncommitted advisory database bytes can escape the recorded Git identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory_database_identity_rejects_forged_origin_and_dirty_bytes",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        ensure!(\n            advisory_database_identity(&directory, &database)? == database_identity,\n            \"advisory database changed during the revision-pinned audit\"\n        );",
        replace: "        let _unchecked_database_after_review = &database_identity;",
        breaks: "the advisory database can change during the revision-pinned review",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        ensure!(\n            advisory_database_identity(&directory, &database)? == database_identity,\n            \"advisory database changed after cargo-audit completed\"\n        );",
        replace: "        let _unchecked_database_before_commit = &database_identity;",
        breaks: "the advisory database can change before qualification publication",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    require_json_fields(\n        report,\n        &[\n            \"database\",\n            \"lockfile\",\n            \"settings\",\n            \"vulnerabilities\",\n            \"warnings\",\n        ],\n        \"cargo-audit report\",\n    )?;",
        replace: "    let _unchecked_advisory_report_schema = report;",
        breaks: "forged top-level cargo-audit fields can enter a clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    require_json_fields(\n        database,\n        &[\"advisory-count\", \"last-commit\", \"last-updated\"],\n        \"cargo-audit database\",\n    )?;",
        replace: "    let _unchecked_advisory_database_schema = database;",
        breaks: "forged advisory database fields can enter a clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        advisory_count != 0,\n        \"cargo-audit report names an empty advisory database\"\n    );",
        replace: "    let _unchecked_advisory_count = database;",
        breaks: "an empty advisory database can qualify a locked graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            ensure!(\n                commit == database_revision,\n                \"cargo-audit report belongs to another advisory database revision\"\n            );",
        replace: "            let _unchecked_report_revision = database_revision;",
        breaks: "a cargo-audit report can name another advisory database revision",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            ensure!(\n                is_rfc3339_timestamp(updated),\n                \"cargo-audit report has no RFC3339 database update time\"\n            );",
        replace: "            let _unchecked_advisory_update_time = updated;",
        breaks: "an advisory report can omit a real database update time",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    require_json_fields(lockfile, &[\"dependency-count\"], \"cargo-audit lockfile\")?;",
        replace: "    let _unchecked_advisory_lock_schema = lockfile;",
        breaks: "forged lockfile report fields can enter a clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        json_u64(\n            json_field(lockfile, \"dependency-count\")?,\n            \"dependency-count\"\n        )? != 0,\n        \"cargo-audit report names an empty lockfile\"\n    );",
        replace: "    let _unchecked_advisory_dependency_count = lockfile;",
        breaks: "an empty reported lockfile can pose as the audited dependency graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    require_json_fields(\n        settings,\n        &[\n            \"ignore\",\n            \"informational_warnings\",\n            \"severity\",\n            \"target_arch\",\n            \"target_os\",\n        ],\n        \"cargo-audit settings\",\n    )?;",
        replace: "    let _unchecked_advisory_settings_schema = settings;",
        breaks: "forged cargo-audit setting fields can enter a clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        json_array(json_field(settings, \"target_arch\")?, \"target_arch\")?.is_empty()\n            && json_array(json_field(settings, \"target_os\")?, \"target_os\")?.is_empty()\n            && matches!(json_field(settings, \"severity\")?, JsonValue::Null)\n            && json_array(json_field(settings, \"ignore\")?, \"ignore\")?.is_empty()\n            && json_string_array(\n                json_field(settings, \"informational_warnings\")?,\n                \"informational_warnings\"\n            )? == [\"unmaintained\", \"unsound\", \"notice\"],\n        \"cargo-audit report was generated with different audit settings\"\n    );",
        replace: "    let _unchecked_advisory_settings = settings;",
        breaks: "ignored advisories can be hidden inside a forged clean report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    require_json_fields(\n        vulnerabilities,\n        &[\"count\", \"found\", \"list\"],\n        \"cargo-audit vulnerabilities\",\n    )?;",
        replace: "    let _unchecked_vulnerability_schema = vulnerabilities;",
        breaks: "forged vulnerability fields can enter a clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        matches!(\n            json_field(vulnerabilities, \"found\")?,\n            JsonValue::Bool(false)\n        ) && json_u64(json_field(vulnerabilities, \"count\")?, \"vulnerability count\")? == 0\n            && json_array(json_field(vulnerabilities, \"list\")?, \"vulnerability list\")?.is_empty(),\n        \"cargo-audit report is not a consistent vulnerability-free result\"\n    );",
        replace: "    let _unchecked_vulnerability_result = vulnerabilities;",
        breaks: "a vulnerable or internally inconsistent report can pose as clean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        warnings.is_empty(),\n        \"cargo-audit report contains a dependency warning\"\n    );",
        replace: "    let _unchecked_advisory_warnings = warnings;",
        breaks: "a dependency warning can enter a forged clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        file: "kernel/src/vfs.rs",
        find: "            let listing = crate::tmpfs::list(index).ok_or(VfsError::NotFound)?;\n            for entry in listing.entries() {",
        replace: "            let listing = crate::tmpfs::list(index).ok_or(VfsError::NotFound)?;\n            static REGRESSION: wost::sync::SpinLock<()> = wost::sync::SpinLock::new(());\n            let _backend = REGRESSION.lock_leaf();\n            for entry in listing.entries() {",
        breaks: "a directory callback runs while its backend still owns a leaf lock",
        reddens: Caught::Check("enumeration callbacks run without backend ownership"),
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        match device.admit_bus_master(authority) {",
        replace: "        match device\n            .admit_bus_master(authority)\n            .or(Ok::<(), pci_config::EnableError>(()))\n        {",
        breaks: "a refused bus-master transition mints start authority",
        reddens: Caught::Check("a failed bus master admission stops before nvme start"),
    },
    Mutation {
        file: "kernel/src/input.rs",
        find: "            KeyState::Released => {\n                if self.down & bit != 0 {\n                    self.completed[index] = self.completed[index].wrapping_add(1);\n                }\n                self.down &= !bit;\n            }",
        replace: "            KeyState::Released => {\n                self.completed[index] = self.completed[index].wrapping_add(1);\n                self.down &= !bit;\n            }",
        breaks: "an interrupt witness accepts a release with no preceding press",
        reddens: Caught::Check("the controller route and ordered interrupt witness are exact"),
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    ensure!(\n        record.render() == text,\n        \"evidence manifest is not in its canonical sorted encoding\"\n    );",
        replace: "    let _canonical_encoding = text;",
        breaks: "an evidence manifest can be reordered or omit its terminal newline",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::rendering_is_sorted_and_round_trips",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "        match std::fs::symlink_metadata(&path) {",
        replace: "        match std::fs::metadata(&path) {",
        breaks: "a broken evidence staging symlink can survive invalidation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_broken_manifest_symlink_cannot_redirect_the_writer",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "fn require_evidence_root(root: &Path) -> Result<()> {\n    require_real_directory(root, \"evidence root\")?;\n    let parent = root.parent().context(\"evidence root has no parent\")?;\n    let canonical_parent = std::fs::canonicalize(parent)\n        .with_context(|| format!(\"canonicalizing {}\", parent.display()))?;\n    let canonical_root = std::fs::canonicalize(root)\n        .with_context(|| format!(\"canonicalizing {}\", root.display()))?;\n    ensure!(\n        canonical_root.parent() == Some(canonical_parent.as_path()),\n        \"evidence root escaped its canonical parent\"\n    );\n    Ok(())\n}",
        replace: "fn require_evidence_root(root: &Path) -> Result<()> {\n    let _unchecked_evidence_root = root;\n    Ok(())\n}",
        breaks: "a substituted evidence root can redirect recursive reset",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_substituted_evidence_root_is_never_recursively_reset",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    let path = checked_bundle_file(root, spec.relative, spec.prefix)?;",
        replace: "    let path = root.join(spec.relative);",
        breaks: "a qualification bundle parent link can substitute identical bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_bundle_parent_link_cannot_substitute_qualification_bytes",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    require_real_directory(directory, \"VMware selector directory\")?;",
        replace: "    let _unchecked_selector_directory = directory;",
        breaks: "a VMware selector name can substitute another in-root directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_vmware_selector_directory_cannot_be_substituted",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            require_contained_regular_file(&self.root, &path, \"qualification snapshot artifact\")?;",
        replace: "            let _unchecked_snapshot_parent = (&self.root, &path);",
        breaks: "a copied qualification snapshot can substitute a linked parent with identical bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_snapshot_parent_link_cannot_substitute_qualification_bytes",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    require_contained_real_directory(&root, &directory, \"qualification kind directory\")?;\n    Ok(directory)",
        replace: "    let _unchecked_qualification_directory = (&root, &directory);\n    Ok(directory)",
        breaks: "a qualification kind name can redirect record writes through a parent link",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_qualification_kind_directory_cannot_be_substituted",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "fn require_real_root(path: &Path, label: &str) -> Result<()> {\n    require_real_directory(path, label)?;\n    let parent = path.parent().context(\"contained root has no parent\")?;\n    let canonical_parent = std::fs::canonicalize(parent)\n        .with_context(|| format!(\"canonicalizing {}\", parent.display()))?;\n    let canonical_path = std::fs::canonicalize(path)\n        .with_context(|| format!(\"canonicalizing {}\", path.display()))?;\n    ensure!(\n        canonical_path.parent() == Some(canonical_parent.as_path()),\n        \"{label} escaped its canonical parent\"\n    );\n    Ok(())\n}",
        replace: "fn require_real_root(path: &Path, label: &str) -> Result<()> {\n    let _unchecked_qualification_root = (path, label);\n    Ok(())\n}",
        breaks: "a qualification root can redirect every kind through a parent link",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_qualification_root_cannot_be_substituted",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    require_current_source(paths, record)?;\n    std::fs::rename(&temporary, &final_path)",
        replace: "    std::fs::rename(&temporary, &final_path)",
        breaks: "a qualification record can commit after its source checkout changes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::source_is_rechecked_immediately_before_a_record_commits",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "    ensure!(\n        canonical_child.parent() == Some(canonical_parent.as_path()),\n        \"{label} escaped its canonical parent\"\n    );",
        replace: "    let _unchecked_canonical_parent = (&canonical_parent, &canonical_child, label);",
        breaks: "a destructive VMware path can resolve outside its exact parent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::destructive_vmware_paths_require_one_canonical_parent",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        let observation = observe_running_vm(self.paths.root(), &self.vmrun, artifacts.vmx)?;",
        replace: "        let observation = format!(\"{VMRUN_LIST_HEADER}1\\n{}\", artifacts.vmx.display());",
        breaks: "VMware qualification fabricates availability text instead of executing vmrun",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "const VMRUN_LIST_ARGS: &[&str] = &[\"-T\", \"ws\", \"list\"];",
        replace: "const VMRUN_LIST_ARGS: &[&str] = &[\"-T\", \"ws\", \"stop\"];",
        breaks: "VMware qualification invokes another vmrun operation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmrun_list_requires_one_exact_running_machine",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    let before = run(&tool.path, &[], ExitPolicy::Any)?;",
        replace: "    let before = tool.version.clone();",
        breaks: "the VMware observer can skip its immediate pre-execution tool identity probe",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmrun_observation_executes_list_between_exact_tool_probes",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    let after = run(&tool.path, &[], ExitPolicy::Any)?;",
        replace: "    let after = tool.version.clone();",
        breaks: "the VMware observer can skip its post-execution tool identity probe",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmrun_observation_executes_list_between_exact_tool_probes",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        !report.contains(['\\0', '\\r']),\n        \"vmrun list contains a forbidden control byte\"\n    );",
        replace: "    ensure!(!report.contains('\\0'), \"vmrun list contains a NUL byte\");",
        breaks: "the archived VMware observer can use noncanonical CR line endings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmrun_list_requires_one_exact_running_machine",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        parsed == 1 && paths.len() == parsed,",
        replace: "        parsed >= 1 && paths.len() == parsed,",
        breaks: "another running VMware machine can share the qualification observation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmrun_list_requires_one_exact_running_machine",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        observed == expected,\n        \"vmrun reported another machine: {}\",",
        replace: "        observed != expected,\n        \"vmrun reported another machine: {}\",",
        breaks: "vmrun may report a foreign VMX instead of the generated selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmrun_list_requires_one_exact_running_machine",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        actual == expected,\n        \"VMware generation inventory differs: missing {:?}; unexpected {:?}\",",
        replace: "        actual.is_superset(&expected),\n        \"VMware generation inventory differs: missing {:?}; unexpected {:?}\",",
        breaks: "stale VMware firmware or suspend state survives generation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::generation_receipt_binds_reset_source_selector_and_machine_bytes",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "                .is_file(),\n            \"VMware generation entry is not a regular file: {}\",",
        replace: "                .is_file() || true,\n            \"VMware generation entry is not a regular file: {}\",",
        breaks: "an authoritative VMware artifact name can be a directory or link",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::generation_receipt_binds_reset_source_selector_and_machine_bytes",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    generation_transcript_name(&generation)",
        replace: "    Ok(VMWARE_TRANSCRIPT.to_string())",
        breaks: "successive VMware generations reuse one replayable transcript path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::each_vmware_generation_owns_a_distinct_serial_file",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "            std::fs::remove_dir_all(&directory)\n                .with_context(|| format!(\"resetting {}\", directory.display()))?;",
        replace: "            let _stale_directory = &directory;",
        breaks: "VMware generation retains prior NVRAM and suspend state",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::generation_replaces_only_its_exact_selector_directory",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "        Kernel::SelfTest => paths.vm_dir().join(\"self-test\"),",
        replace: "        Kernel::SelfTest => paths.vm_dir(),",
        breaks: "the self-test selector shares its directory with VMware state and siblings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::selftest_and_shipped_vms_do_not_share_machine_state",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "        ensure!(\n            digest_tracked(&self.snapshot, &self.baseline.tracked)? == self.baseline.source_digest,\n            \"the immutable mutation source snapshot changed during the sweep\"\n        );",
        replace: "        let _snapshot_was_not_rechecked = &self.snapshot;",
        breaks: "a changed mutation snapshot can authorize a report for pristine source",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_changed_snapshot_cannot_authorize_a_report_for_pristine_source",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    BundleFile {\n        prefix: \"qualification.vmware-self-test.observer\",\n        relative: \"qualification/vmware-self-test/vmrun-list.txt\",\n    },",
        replace: "",
        breaks: "release evidence omits the VMware execution observer",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::qualification_records_cannot_be_omitted_or_changed_offline",
        },
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "    pub const fn is_consistent(&self) -> bool {\n        (self.reserved as u64) + (self.completion.occupied() as u64)\n            <= self.completion.capacity().entries() as u64\n    }",
        replace: "    pub const fn is_consistent(&self) -> bool {\n        true\n    }",
        breaks: "the ring reservation diagnostic cannot report a broken invariant",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::the_consistency_oracle_refuses_an_over_reserved_ring",
        },
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "    pub const fn tail(&self) -> u32 {\n        self.tail\n    }",
        replace: "    pub const fn tail(&self) -> u32 {\n        0\n    }",
        breaks: "the public tail accessor reports a slot-like constant instead of its counter",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::counter_accessors_report_counts_not_masked_slots",
        },
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "    pub const fn is_full(&self) -> bool {\n        self.occupied() == self.capacity.entries()\n    }",
        replace: "    pub const fn is_full(&self) -> bool {\n        self.occupied() != self.capacity.entries()\n    }",
        breaks: "the ring full-state predicate disagrees with its occupied count",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "occupancy_and_boundary_predicates_remain_consistent",
        },
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {\n        match *self {\n            Self::NotAPowerOfTwo(value) => write!(f, \"{value} is not a power of two\"),\n            Self::TooLarge(value) => write!(f, \"{value} entries is more than a ring may hold\"),\n        }\n    }",
        replace: "    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {\n        let _ = (self, f);\n        Ok(())\n    }",
        breaks: "capacity refusals lose the value and rule they report",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::capacity_errors_name_the_refused_value_and_rule",
        },
    },
    Mutation {
        file: "libs/frame-space/src/lib.rs",
        find: "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        match *self {\n            Self::NotManaged(frame) => write!(f, \"frame {frame} is not managed memory\"),\n            Self::AlreadyFree(frame) => write!(f, \"frame {frame} is already free\"),\n        }\n    }",
        replace: "    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        Ok(())\n    }",
        breaks: "frame release refusals lose the frame and ownership failure",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::release_errors_name_the_frame_and_ownership_failure",
        },
    },
    Mutation {
        file: "libs/frame-space/src/lib.rs",
        find: "    pub const fn is_free(&self, frame: usize) -> bool {\n        frame < Self::FRAME_COUNT && self.free[frame / FRAMES_PER_WORD] & bit(frame) != 0\n    }",
        replace: "    pub const fn is_free(&self, frame: usize) -> bool {\n        frame <= Self::FRAME_COUNT && self.free[frame / FRAMES_PER_WORD] & bit(frame) != 0\n    }",
        breaks: "the bitmap ceiling is treated as a free frame index",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::membership_stops_at_the_bitmap_ceiling",
        },
    },
    Mutation {
        file: "libs/frame-space/src/lib.rs",
        find: "    pub const fn is_managed(&self, frame: usize) -> bool {\n        frame < Self::FRAME_COUNT && self.managed[frame / FRAMES_PER_WORD] & bit(frame) != 0\n    }",
        replace: "    pub const fn is_managed(&self, frame: usize) -> bool {\n        frame <= Self::FRAME_COUNT && self.managed[frame / FRAMES_PER_WORD] & bit(frame) != 0\n    }",
        breaks: "the bitmap ceiling is treated as a managed frame index",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::membership_stops_at_the_bitmap_ceiling",
        },
    },
    Mutation {
        file: "libs/frame-space/src/lib.rs",
        find: "        if count == 0 || count > Self::FRAME_COUNT {",
        replace: "        if count == 0 || count >= Self::FRAME_COUNT {",
        breaks: "an allocation spanning the exact frame domain is refused as oversized",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::allocation_count_distinguishes_the_complete_domain_from_too_many",
        },
    },
    Mutation {
        file: "libs/frame-space/src/lib.rs",
        find: "    pub fn allocate(&mut self) -> Option<usize> {\n        self.allocate_contiguous(1, 1)\n    }",
        replace: "    pub fn allocate(&mut self) -> Option<usize> {\n        Some(0)\n    }",
        breaks: "a frame-allocation sweep never reaches exhaustion",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::a_sweep_of_the_whole_space_conserves_every_frame",
        },
    },
    Mutation {
        file: "libs/frame-space/src/lib.rs",
        find: "        for _ in 0..=Self::FRAME_COUNT {",
        replace: "        for _ in 0..=0 {",
        breaks: "first-fit stops after the first obstructed candidate window",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::a_contiguous_run_skips_past_a_hole",
        },
    },
    Mutation {
        file: "libs/frame-space/src/lib.rs",
        find: "            if end > to {",
        replace: "            if end == to {",
        breaks: "first-fit may cross the search interval's exclusive upper frame",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::search_treats_its_upper_frame_as_exclusive",
        },
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "        self.remaining -= 1;",
        replace: "        self.remaining += 1;",
        breaks: "an MMIO offset iterator grows instead of reaching exhaustion",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "every_small_repeated_access_matches_an_independent_oracle",
        },
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "        (self.remaining, Some(self.remaining))",
        replace: "        (0, None)",
        breaks: "an exact MMIO offset iterator hides its remaining length",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "every_small_repeated_access_matches_an_independent_oracle",
        },
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "            Self::InvalidWidth(width) => write!(formatter, \"access width {width} is invalid\"),",
        replace: "            Self::InvalidWidth(_) => formatter.write_str(\"invalid\"),",
        breaks: "an MMIO refusal omits the invalid width and its rule",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "tests::errors_name_the_refused_access",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/lib.rs",
        find: "        Self(self.0 | other.0)",
        replace: "        Self(self.0 ^ other.0)",
        breaks: "union removes a flag already held by both inputs",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::union_is_idempotent_when_the_sets_overlap",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/lib.rs",
        find: "            Self::Misaligned(address) => write!(f, \"{address:#x} is not page-aligned\"),",
        replace: "            Self::Misaligned(_) => f.write_str(\"misaligned\"),",
        breaks: "a page-entry refusal omits the address and alignment rule",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::entry_errors_name_the_refused_value_or_rule",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/lib.rs",
        find: "            Self::ZeroLength => core::fmt::Write::write_str(f, \"mapping length is zero\"),",
        replace: "            Self::ZeroLength => core::fmt::Write::write_str(f, \"zero\"),",
        breaks: "a mapping-range refusal omits the failed length rule",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::mapping_range_errors_name_the_refused_value_or_rule",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "        || !value.bytes().all(|byte| byte.is_ascii_digit())",
        replace: "",
        breaks: "a signed artifact length is accepted as canonical decimal",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::decimals_tokens_and_revisions_enforce_each_canonical_boundary",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "            Self::NonCanonicalLineEnding => \"handoff record does not use canonical LF endings\",",
        replace: "            Self::NonCanonicalLineEnding => \"invalid\",",
        breaks: "a handoff refusal omits the noncanonical line-ending rule",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::errors_name_the_refused_record_property",
        },
    },
    Mutation {
        file: "libs/frame-space/src/lib.rs",
        find: "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        f.debug_struct(\"FrameSpace\")\n            .field(\"managed\", &self.managed_count)\n            .field(\"free\", &self.free_count)\n            .finish()\n    }",
        replace: "    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        Ok(())\n    }",
        breaks: "the frame allocator diagnostic loses both ledger counts",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::debug_reports_both_ledger_counts",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    verify_record(paths, &root, &record)?;\n    let manifest = root.join(MANIFEST_NAME);\n    write_manifest(&root, &manifest, &record.render())?;",
        replace: "    let manifest = root.join(MANIFEST_NAME);\n    write_manifest(&root, &manifest, &record.render())?;\n    verify_record(paths, &root, &record)?;",
        breaks: "a failed final evidence verification leaves a committed manifest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::an_evidence_attempt_invalidates_an_older_success_first",
        },
    },
    Mutation {
        file: "xtask/src/mutate.rs",
        find: "        let mut impossible = MUTATIONS[0];\n        impossible.file = \"xtask/src/image/program.rs\";\n        impossible.reddens = Caught::Check(\"unused\");",
        replace: "        let mut impossible = MUTATIONS[0];\n        impossible.file = \"xtask/src/image/program.rs\";\n        let _ = &mut impossible.reddens;",
        breaks: "the mutation-driver safety test inherits an order-dependent witness class",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_mutation_cannot_target_the_running_driver",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            .rfind(\"let qualification_attempt = qualification::begin_mutation\")",
        replace: "            .find(\"let qualification_attempt = qualification::begin_mutation\")",
        breaks: "the exceptional-order check reads its registry fixture instead of production",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
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
        file: "xtask/src/mutate.rs",
        find: "    let qualification_attempt = qualification::begin_mutation(paths, &registry, MUTATIONS.len())?;\n    let mut session = Session::open(paths, &registry, MUTATIONS.len())?;",
        replace: "    let mut session = Session::open(paths, &registry, MUTATIONS.len())?;\n    let qualification_attempt = qualification::begin_mutation(paths, &registry, MUTATIONS.len())?;",
        breaks: "a failed mutation preflight leaves an older passing qualification current",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        file: "xtask/src/mutate.rs",
        find: "        let pass = crate::mutation_workspace::current_pass(paths, &registry, MUTATIONS.len())?;\n        qualification_attempt.pass(&pass)?;",
        replace: "        let pass = crate::mutation_workspace::current_pass(paths, &registry, MUTATIONS.len())?;\n        drop(qualification_attempt);",
        breaks: "a complete caught mutation sweep does not commit its qualification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "    let Some(xcr0) = xcr0.strip_suffix(\", compacted\") else {\n        return false;\n    };",
        replace: "    let xcr0 = xcr0.strip_suffix(\", compacted\").unwrap_or(xcr0);",
        breaks: "compound VMware execution evidence can be stitched across lines",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "fn info_records(text: &str) -> impl Iterator<Item = (usize, &str)> {\n    let mut offset = 0;\n    text.split_inclusive('\\n').filter_map(move |record| {\n        let at = offset;\n        offset += record.len();\n        let line = record.strip_suffix('\\n')?;\n        let message = line.strip_prefix(\"[info ] \")?;\n        Some((at, message))\n    })\n}",
        replace: "fn info_records(text: &str) -> impl Iterator<Item = (usize, &str)> {\n    let mut offset = 0;\n    text.split_inclusive('\\n').flat_map(move |record| {\n        let at = offset;\n        offset += record.len();\n        record.strip_suffix('\\n').into_iter().flat_map(move |line| {\n            line.match_indices(\"[info ] \").map(move |(inside_at, _)| {\n                let body = &line[inside_at + \"[info ] \".len()..];\n                let message = body.split(\" [info ] \").next().unwrap_or(body);\n                (at + inside_at, message)\n            })\n        })\n    })\n}",
        breaks: "unrelated VMware milestones can be stitched onto one line",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "    qualification::invalidate_vmware(paths, kernel)?;",
        replace: "    let _direct_generation = (paths, kernel);",
        breaks: "direct VMware generation leaves an older qualification valid",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::generation_invalidates_success_and_receipt_before_any_fallible_preflight",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        if sector[510] != 0x55 || sector[511] != 0xAA {",
        replace: "        if sector[510] != 0x55 && sector[511] != 0xAA {",
        breaks: "half of a FAT boot signature is accepted as the whole signature",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_boot_sector_that_is_not_one_is_refused",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "                + byte / SECTOR_BYTES as u64,",
        replace: "                + 0,",
        breaks: "FAT entries past the first table sector are read from sector zero",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::the_boot_sector_is_read_where_the_layout_puts_its_fields",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        wanted\n            .iter()\n            .zip(self.name())\n            .all(|(wanted, actual)| upper(*wanted) == upper(*actual))",
        replace: "        wanted.iter().zip(self.name()).all(|_| true)",
        breaks: "a same-length different FAT name is accepted as a match",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_name_is_matched_without_regard_to_case",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        if slot[8 + index] != b' ' {\n            return Slot::Skip;\n        }",
        replace: "        if false && slot[8 + index] != b' ' {\n            return Slot::Skip;\n        }",
        breaks: "a sparse FAT extension loses its only non-space byte",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::an_eight_three_name_is_unpacked",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "                    if visited >= self.chain_limit() {\n                        return Err(FatError::Unbounded);\n                    }",
        replace: "                    if false && visited >= self.chain_limit() {\n                        return Err(FatError::Unbounded);\n                    }",
        breaks: "a directory chunk follows more cluster links than the volume can contain",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::directory_chunks_retain_the_volume_derived_cycle_bound",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "                done += take;",
        replace: "                done = 0;",
        breaks: "a FAT file read makes no forward byte progress",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::file_reads_match_the_concatenated_sector_bytes_at_every_boundary",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "                if !entry.is_directory() {\n                    return Err(FatError::WrongKind);\n                }\n                cluster = entry.cluster;",
        replace: "                cluster = entry.cluster;",
        breaks: "path resolution descends through a non-directory component",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::path_resolution_descends_only_through_directory_components",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(actual == expected, \"qualification checksum does not match\");",
        replace: "    ensure!(is_digest(expected), \"qualification checksum does not match\");",
        breaks: "a changed qualification record retains its old checksum",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_qualification_record_is_canonical_and_checksummed",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        actual == expected,\n        \"qualification schema differs: missing {:?}; unknown {:?}\",",
        replace: "    ensure!(\n        expected.is_subset(&actual),\n        \"qualification schema differs: missing {:?}; unknown {:?}\",",
        breaks: "an invented qualification claim is accepted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        record.get(\"source.commit\")? == source.commit,",
        replace: "        is_git_id(record.get(\"source.commit\")?),",
        breaks: "qualification from another commit remains current",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::stale_source_identity_is_rejected_field_by_field",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        record.get(\"source.tree\")? == source.tree,",
        replace: "        is_git_id(record.get(\"source.tree\")?),",
        breaks: "qualification from another tree remains current",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::stale_source_identity_is_rejected_field_by_field",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            std::fs::remove_dir_all(&directory)\n                .with_context(|| format!(\"invalidating stale {} qualification\", kind.slug()))?;",
        replace: "            return Ok(());",
        breaks: "a new exceptional attempt leaves an older pass valid",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::starting_an_attempt_invalidates_an_older_pass",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        receipt == generation_record(source, kind, directory, generation)?,",
        replace: "        receipt.get(\"input.kernel\")? == kind.slug(),",
        breaks: "a VMware receipt ignores its source and machine bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::generation_receipt_binds_reset_source_selector_and_machine_bytes",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        transcript.bytes != 0 && transcript.sha256 != empty_digest(),",
        replace: "        transcript.bytes == 0 || transcript.sha256 != empty_digest(),",
        breaks: "an untouched VMware reset counts as execution",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::generation_receipt_binds_reset_source_selector_and_machine_bytes",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        reset.bytes == 0 && reset.sha256 == empty_digest(),",
        replace: "        reset.bytes <= reset.bytes,",
        breaks: "a generation receipt can be issued over an old transcript",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::generation_receipt_binds_reset_source_selector_and_machine_bytes",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    for path in [receipt.to_path_buf(), parent.join(VMWARE_GENERATION_TEMP)] {",
        replace: "    for path in std::iter::empty::<PathBuf>() {",
        breaks: "VMware generation preserves stale complete and partial receipts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::generation_start_invalidates_both_receipt_names",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            record.get(&format!(\"{prefix}.sha256\"))? == digest.sha256,",
        replace: "            digest.bytes == digest.bytes,",
        breaks: "inner qualification support can differ at equal length",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::copied_support_bytes_must_match_their_inner_record",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        record.get(\"input.registry.sha256\")? == registry,",
        replace: "        is_digest(registry),",
        breaks: "mutation qualification accepts another registry",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        record.get(\"input.mutations\")? == mutation_count.to_string(),",
        replace: "        mutation_count != 0,",
        breaks: "mutation qualification accepts another registry cardinality",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            mutation.registry == registry",
        replace: "            true",
        breaks: "the newest mutation pass can name another registry",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "                && mutation.mutation_count == mutation_count",
        replace: "                && true",
        breaks: "the newest mutation pass can name another count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "                && record.get(\"source.commit\")? == mutation.commit",
        replace: "                && is_git_id(&mutation.commit)",
        breaks: "the newest mutation pass can belong to another commit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "                && record.get(\"input.source.sha256\")? == mutation.source_digest",
        replace: "                && is_digest(&mutation.source_digest)",
        breaks: "the newest mutation pass can describe other source bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "                && record.get(\"input.index.sha256\")? == mutation.index_digest",
        replace: "                && is_digest(&mutation.index_digest)",
        breaks: "the newest mutation pass can describe another index",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "                && record.get(\"artifact.report.sha256\")? == mutation.report_digest,",
        replace: "                && is_digest(&mutation.report_digest),",
        breaks: "the newest mutation pass can point at another report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            record.get(key)? == value,",
        replace: "            record.fields.contains_key(key),",
        breaks: "current TLC and VMware tool inputs are compared by name only",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "    let evidence_end = require_once_in_order(text, SELFTEST_MARKERS)?;",
        replace: "    let evidence_end = 0;",
        breaks: "VMware self-test evidence markers are optional",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "    require_once_in_order(text, SHIPPED_MARKERS)?;",
        replace: "    let _ = (text, SHIPPED_MARKERS);",
        breaks: "VMware shipped liveness markers are optional",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_shipped_vmware_transcript_proves_liveness_without_selftest_substitution",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "            at >= end,",
        replace: "            at < text.len(),",
        breaks: "stitched VMware transcript markers may be out of order",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "            occurrences.len() == 1,",
        replace: "            !occurrences.is_empty(),",
        breaks: "duplicated VMware execution evidence is accepted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "        expected != 0 && passed == expected && failed == 0 && reported == expected,",
        replace: "        expected != 0 && failed == 0,",
        breaks: "a stale self-test verdict count qualifies",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "        !text.contains(\"KERNEL PANIC\"),",
        replace: "        true,",
        breaks: "a panicked VMware boot qualifies",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "        !text.lines().any(|line| {",
        replace: "        !text.lines().any(|line| false && {",
        breaks: "a failed VMware self-test check qualifies",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "        !text.contains('\\0'),",
        replace: "        true,",
        breaks: "a transcript containing a NUL byte qualifies",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "    let text = std::str::from_utf8(bytes).context(\"VMware transcript is not UTF-8\")?;",
        replace: "    let text_storage = String::from_utf8_lossy(bytes); let text = text_storage.as_ref();",
        breaks: "a non-UTF-8 VMware transcript is repaired and accepted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "        !text.contains(\"selftest: verdict\"),",
        replace: "        true,",
        breaks: "a self-test boot substitutes for the shipped VMware run",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_shipped_vmware_transcript_proves_liveness_without_selftest_substitution",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "        !text.contains(\"user: no program\"),",
        replace: "        true,",
        breaks: "a shipped VMware boot that refused its program qualifies",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_shipped_vmware_transcript_proves_liveness_without_selftest_substitution",
        },
    },
    Mutation {
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
        file: "xtask/src/vmware.rs",
        find: "    invalidate_generation(paths, kernel)?;",
        replace: "    let _generation_starts_here = (paths, kernel);",
        breaks: "a direct VMware generation preserves its old receipt",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::generation_invalidates_success_and_receipt_before_any_fallible_preflight",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "    write_new(&serial_log, &[])?;",
        replace: "    let _ = &serial_log;",
        breaks: "a generation receipt is attempted before resetting serial evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::generation_invalidates_success_and_receipt_before_any_fallible_preflight",
        },
    },
    Mutation {
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
        file: "xtask/src/evidence.rs",
        find: "            Ok(_) => {\n                std::fs::remove_file(&path).with_context(|| {\n                    format!(\"invalidating stale evidence manifest {}\", path.display())\n                })?;\n            }",
        replace: "            Ok(_) => {\n                let _preserved = &path;\n            }",
        breaks: "an evidence attempt preserves its older manifest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::an_evidence_attempt_invalidates_an_older_success_first",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    let root = invalidate_manifest(paths)?;",
        replace: "    let root = evidence_root(paths);",
        breaks: "evidence preflight runs before stale success is invalidated",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::an_evidence_attempt_invalidates_an_older_success_first",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        relative: \"qualification/model/record\",",
        replace: "        relative: \"qualification/model/record-omitted\",",
        breaks: "the exceptional evidence bundle omits a required record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::qualification_records_cannot_be_omitted_or_changed_offline",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "        ensure!(\n            record.get(&format!(\"{}.sha256\", spec.prefix))? == digest.sha256,\n            \"{} has a different digest than its manifest\",\n            spec.prefix\n        );\n    }\n    Ok(())\n}\n\nfn verify_qualification_artifact_identity",
        replace: "        ensure!(\n            digest.bytes == digest.bytes,\n            \"{} has a different digest than its manifest\",\n            spec.prefix\n        );\n    }\n    Ok(())\n}\n\nfn verify_qualification_artifact_identity",
        breaks: "a qualification support file can change at equal length",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::qualification_records_cannot_be_omitted_or_changed_offline",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "        shipped_image\n            == digest_bundle_file_exact(\n                root,\n                ARTIFACTS[1].relative,\n                ARTIFACTS[1].prefix,\n                expected_image_bytes,\n            )?,",
        replace: "        shipped_image.sha256 == shipped_image.sha256,",
        breaks: "the VMware shipped image can differ from the reproduced release",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::exceptional_artifacts_are_the_exact_reproduced_inputs",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "        selftest_image != shipped_image,",
        replace: "        selftest_image.bytes != 0,",
        breaks: "the VMware selectors can preserve the same boot image",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::exceptional_artifacts_are_the_exact_reproduced_inputs",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "            bundled == namespace,",
        replace: "            bundled.bytes != 0,",
        breaks: "VMware can use a namespace other than QEMU's recorded input",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::exceptional_artifacts_are_the_exact_reproduced_inputs",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if self.preflight(&range, flags, grain)? == MappingState::Identical {\n            return Ok(());\n        }",
        replace: "        let _ = self.preflight(&range, flags, grain);",
        breaks: "mapping rollback clears only leaves this transaction created",
        reddens: Caught::Check("a refused remap preserves every older entry"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let flags = flags.union(Flags::PRESENT);\n\n        if let Some(stack) = protected_stack_overlap(self.root(), virtual_base, len) {",
        replace: "        let flags = flags.union(Flags::PRESENT);\n\n        if let Some(stack) = protected_stack_overlap(self.root(), virtual_base, len).filter(|_| false) {",
        breaks: "the safe map path can admit a mapping over a retained stack or guard",
        reddens: Caught::Check("protected stacks refuse overlapping maps"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        self.shared.push(keepalive);",
        replace: "        drop(keepalive);",
        breaks: "a shared mapping retains ownership of every frame it reaches",
        reddens: Caught::Check("a shared mapping keeps its frame"),
    },
    // `map_run` restores `OWNED`, so only the `map_boot` strip is load-bearing.
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            flags.difference(NOT_THE_CALLERS),
            Grain::Largest,",
        replace: "            flags,
            Grain::Largest,",
        breaks: "a boot mapping cannot launder a frame into the allocator",
        reddens: Caught::Check("a boot mapping cannot claim ownership"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        space.reach = Reach::UserHalfOnly;",
        replace: "        space.reach = Reach::Everywhere;",
        breaks: "a user space cannot write the kernel's half",
        reddens: Caught::Check("a user space cannot reach the kernel half"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if flags.contains(Flags::USER) {\n            return Err(MapError::NotYours(virtual_base));\n        }\n        self.map_run(virtual_base, run, flags)",
        replace: "        self.map_run(virtual_base, run, flags)",
        breaks: "kernel memory cannot be published to ring 3",
        reddens: Caught::Check("kernel memory cannot be published to ring three"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if flags.contains(Flags::USER) {\n            return Err(MapError::NotYours(virtual_base));\n        }\n        self.may_reach(virtual_base, region.len)?;",
        replace: "        self.may_reach(virtual_base, region.len)?;",
        breaks: "boot evidence cannot publish kernel memory to ring 3",
        reddens: Caught::Check("a boot mapping cannot publish kernel memory"),
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        if segment.zero() {\n            return Some(segment);\n        }",
        replace: "        return Some(segment);\n        #[allow(unreachable_code)]\n        if segment.zero() {\n            return Some(segment);\n        }",
        breaks: "a reclaimed frame is cleared before the kernel sees it",
        reddens: Caught::Check("a user page comes back cleared"),
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            if published == 0 || published > SUBMISSIONS {",
        replace: "            if published == 0 {",
        breaks: "a runaway submission tail is bounded",
        reddens: Caught::Check("a rewritten submission cannot change what was served"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "    if xstate::is_enabled() {",
        replace: "    if false && xstate::is_enabled() {",
        breaks: "extended state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "    table.slots[current].thread_base = leaving;",
        replace: "",
        breaks: "thread-local storage follows the thread",
        reddens: Caught::Check("thread local storage follows the thread"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "    if live {\n        Err(bound)\n    } else {\n        Ok(Released { root })\n    }",
        replace: "    if false {\n        Err(bound)\n    } else {\n        Ok(Released { root })\n    }",
        breaks: "a live task retains its address-space release authority",
        reddens: Caught::Check("a bound space is not reclaimed"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "        unsafe { cpu::set_page_table_base(wanted) };",
        replace: "",
        breaks: "a thread's address space is installed by the switch",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::task_switch_loads_cr3_under_its_guard_before_context_state",
        },
    },
    // Without the fixup the fault is terminal, so its panic is the causal observer.
    Mutation {
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            if usercopy::recover(frame, fault) =>",
        replace: "            if false && usercopy::recover(frame, fault) =>",
        breaks: "a faulting user copy resumes instead of stopping the machine",
        reddens: Caught::Panic {
            marker: "faulting address 0x0000600000000000",
            why: "without the fixup the fault is terminal, so the kernel dies before it \
                  reaches a verdict at all",
        },
    },
    // Trap-context logging is caught by ThreadLock's fail-stop (D-122).
    Mutation {
        file: "kernel/src/user.rs",
        find: "            REFUSED_CALLS.fetch_add(1, Ordering::Relaxed);",
        replace: "            crate::warn!(\"refused\");",
        breaks: "an unserved call does not log from trap context",
        reddens: Caught::Panic {
            marker: "a trap handler may not take a thread lock",
            why: "logging from trap context trips `ThreadLock`'s assert, which is the \
                  defect being a defect",
        },
    },
    // A ring-3 copy-window target must retire rather than enter kernel recovery (D-126).
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "        code_selector: frame.code_selector(),",
        replace: "        code_selector: 0,",
        breaks: "a user copy is only recovered when the kernel was the one copying",
        reddens: Caught::Check("a program cannot aim at the copy window"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            if frames.zero() {",
        replace: "            if false && frames.zero() {",
        breaks: "a dropped user range is cleared before its frames go back",
        reddens: Caught::Check("a user page comes back cleared"),
    },
    // Inspecting the ring under the process-table guard recreates nested leaf locking.
    Mutation {
        file: "kernel/src/user.rs",
        find: "    let guard = BOOTSTRAPS.lock_leaf();
    let bootstrap = guard.get(slot).and_then(Option::as_ref)?;
    Some((
        alloc::sync::Arc::clone(&bootstrap.ring),
        alloc::sync::Arc::clone(&bootstrap.table),
    ))",
        replace: "    let guard = BOOTSTRAPS.lock_leaf();
    let bootstrap = guard.get(slot).and_then(Option::as_ref)?;
    let _ = bootstrap.ring.shape();
    Some((
        alloc::sync::Arc::clone(&bootstrap.ring),
        alloc::sync::Arc::clone(&bootstrap.table),
    ))",
        breaks: "no lock is taken while a leaf guard is held",
        reddens: Caught::Check("no lock is taken while a leaf guard is held"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "        .filter(|birth| birth.table == spawned.observed_table_incarnation)",
        replace: "        .filter(|birth| {\n            birth.table == spawned.observed_table_incarnation.wrapping_add(1)\n        })",
        breaks: "birth evidence belongs to the exact authority-table incarnation",
        reddens: Caught::Check("a spawned program holds exactly one handle"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if self.bound {
            match released.as_ref() {",
        replace: "        if false && self.bound {
            match released.as_ref() {",
        breaks: "a bound space is not reclaimed under its own thread",
        reddens: Caught::Check("a bound space is not reclaimed"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if !is_active() || self.bound {",
        replace: "        if !is_active() {",
        breaks: "one address space can issue a second live binding",
        reddens: Caught::Check("one space has one binding"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    let binding = construction\n        .space()?\n        .bind()\n        .map_err(SpawnError::NotBindable)?;",
        replace: "    let _duplicate_binding = AddressSpace::bind(construction.space()?);\n    let binding = construction\n        .space()?\n        .bind()\n        .map_err(SpawnError::NotBindable)?;",
        breaks: "a second production owner can bind one address space",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::one_production_owner_binds_and_retires_each_address_space",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "            authority: Some(BootstrapAuthority::Bound(bound)),",
        replace: "            authority: None,",
        breaks: "a committed process discards its live address-space binding",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::one_production_owner_binds_and_retires_each_address_space",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "        BootstrapAuthority::Bound(bound) => match task::release_address_space(bound) {\n            Ok(released) => Some(BootstrapAuthority::Released(released)),\n            Err(bound) => {\n                bootstrap.authority = Some(BootstrapAuthority::Bound(bound));\n                return Err(ReclaimRetention {\n                    refusal: ReclaimRefusal::TaskRelease,\n                    #[cfg(feature = \"selftest\")]\n                    observed_returned_pages: 0,\n                });\n            }\n        },",
        replace: "        BootstrapAuthority::Bound(bound) => Some(BootstrapAuthority::Bound(bound)),",
        breaks: "zombie reclamation skips release of a live address-space binding",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::one_production_owner_binds_and_retires_each_address_space",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    let tables = match bootstrap.space.reclaim(Some(released)) {",
        replace: "    let tables = match bootstrap.space.reclaim(None) {",
        breaks: "address-space reclamation withholds its exact release evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::one_production_owner_binds_and_retires_each_address_space",
        },
    },
    Mutation {
        file: "kernel/src/devfs.rs",
        find: "pub(crate) const DEVICES: [Device; 2] = [Device::Storage, Device::Console];",
        replace: "pub(crate) const DEVICES: [Device; 1] = [Device::Storage];",
        breaks: "a device variant is absent from the devfs listing",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_devfs_source_inventory_matches_every_variant",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: "    let Ok(mut victim) = AddressSpace::for_user() else {\n        if let Err(reason) = observer.reclaim(None) {\n            warn!(\"selftest: observer cleanup after victim-space setup refusal: {reason}\");\n        }\n        return false;\n    };",
        replace: "    let Ok(mut victim) = AddressSpace::for_user() else {\n        let _ = observer.reclaim(None);\n        return false;\n    };",
        breaks: "a setup refusal discards address-space cleanup",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::known_fallible_results_are_not_discarded",
        },
    },
    // Retained Phase 0-5 audit checks (D-135 to D-145).
    Mutation {
        file: "kernel/src/console.rs",
        find: "        self.hide_cursor();
        self.put(text);
        self.newline();
        self.show_cursor();",
        replace: "        self.put(text);
        self.newline();",
        breaks: "a line break happens with the cursor down",
        reddens: Caught::Check("a scroll leaves no cursor behind"),
    },
    Mutation {
        file: "kernel/src/dev.rs",
        find: "pub(crate) fn with_console(f: impl FnOnce(&mut Console)) {
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        f(console);
    }
}",
        replace: "pub(crate) fn with_console(f: impl FnOnce(&mut Console)) {
    let disabled = wost::arch::current::interrupt::disable();
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        f(console);
    }
    disabled.restore();
}",
        breaks: "console work keeps interrupts available",
        reddens: Caught::Check("a console scroll does not block interrupts"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/port.rs",
        find: "    if !CLAIMED.lock_leaf().reserve_all(&spans) {
        return None;
    }
    // SAFETY: the registry has just recorded every port both accesses drive,",
        replace: "    if !reserve(first, size_of::<T>()) {
        return None;
    }
    if !reserve(second, size_of::<U>()) {
        return None;
    }
    // SAFETY: the registry has just recorded every port both accesses drive,",
        breaks: "two claims are taken all at once or not at all",
        reddens: Caught::Check("a refused pair of claims strands neither"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "fn enable_write_protection() {
    write_cr0(read_cr0() | CR0_WRITE_PROTECT);
}",
        replace: "fn enable_write_protection() {
    write_cr0(read_cr0());
}",
        breaks: "the kernel enables supervisor write protection itself",
        reddens: Caught::Check("supervisor writes obey read-only pages"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "    unsafe { msr::write(EFER, msr::read(EFER) & !EFER_SYSCALL) };",
        replace: "",
        breaks: "the syscall door is shut before ring 3 runs",
        reddens: Caught::Check("ring three has no fast-syscall door"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    pub const fn is_reclaimed(&self) -> bool {
        self.root.is_none()
    }",
        replace: "    pub const fn is_reclaimed(&self) -> bool {
        matches!(self.root, None | Some(0))
    }",
        breaks: "a reclaimed space is not walked as physical frame zero",
        reddens: Caught::Check("a reclaimed space answers nothing"),
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        let slot = &mut self.slots[index];
        let generation = slot.generation;
        slot.entry = Some(entry);",
        replace: "        let generation = self.slots[index].generation;
        let _ = entry;",
        breaks: "a claimed slot is filled by the call that claimed it",
        reddens: Caught::Check("filling the arena never repeats a slot"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "    BYTES.store(bytes, Ordering::Release);",
        replace: "    BYTES.store(bytes / 2, Ordering::Release);",
        breaks: "a save area is sized for the form that writes it",
        reddens: Caught::Check("a save stays inside its area"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    \"verw word ptr [rsp]\",",
        replace: "",
        breaks: "the buffer flush is the instruction, not its neighbour",
        reddens: Caught::Check("the return to ring three clears buffers"),
    },
    // Charging one slot too few leaves the final insertion no prepaid room,
    // the boundary that would allocate or panic in trap context (D-182).
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        let id = commit_table(quota)?;",
        replace: "        let id = commit_table(quota.saturating_sub(1))?;",
        breaks: "a table's final quota slot is backed before any trap can use it",
        reddens: Caught::Check("the handle arena backs a table's whole quota"),
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        let granted = offered.attenuate(declared).ok_or(HandleError::Denied)?;
        if !granted.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        let handle = self.table.fill(object, granted);",
        replace: "        let granted = offered;
        let _ = declared;
        if !granted.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        let handle = self.table.fill(object, granted);",
        breaks: "rights attenuate on arrival",
        reddens: Caught::Check("a completion carries a handle with narrowed rights"),
    },
    Mutation {
        file: "kernel/src/object.rs",
        find: "            Self::Channel => Rights::NONE,",
        replace: "            Self::Channel => Rights::READ,",
        breaks: "a bootstrap identity admits no speculative channel data rights",
        reddens: Caught::Check("a meaningless right is refused"),
    },
    Mutation {
        file: "kernel/src/object.rs",
        find: "            Self::File => Rights::READ,",
        replace: "            Self::File => Rights::READ.union(Rights::WRITE),",
        breaks: "a file refuses a right no operation consults",
        reddens: Caught::Check("a meaningless right is refused"),
    },
    Mutation {
        file: "kernel/src/object.rs",
        find: "            Self::Directory => Rights::TRAVERSE.union(Rights::ENUMERATE),",
        replace: "            Self::Directory => Rights::TRAVERSE
                .union(Rights::ENUMERATE)
                .union(Rights::MODIFY),",
        breaks: "a directory refuses a right no operation consults",
        reddens: Caught::Check("a meaningless right is refused"),
    },
    // The kernel reads Echo's argument from the old handle field while the
    // independent ring-3 emitter keeps writing the widened offset.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "    pub(super) const ARGUMENT: usize = 24;",
        replace: "    pub(super) const ARGUMENT: usize = 8;",
        breaks: "the emitted program and kernel use one submission layout",
        reddens: Caught::Check("a program drives its own ring"),
    },
    // Skipping release leaks one promised handle slot after every refusal.
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        if !self.spent {\n            let released = self.table.step(Quota::release);\n            assert!(released, \"a live reservation refused quota release\");\n        }",
        replace: "        if false && !self.spent {\n            let released = self.table.step(Quota::release);\n            assert!(released, \"a live reservation refused quota release\");\n        }",
        breaks: "a refused operation gives its promised slot back",
        reddens: Caught::Check("a refused handle operation returns its promise"),
    },
    // Only a pre-deadline observation distinguishes deferral from immediate completion.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "self.begin_settlement(|entry| {
            entry
                .kind
                .deadline()
                .is_some_and(|deadline| !clock_live || deadline <= now_ns)
        })",
        replace: "self.begin_settlement(|entry| entry.kind.deadline().is_some())",
        breaks: "an operation is not due before its deadline",
        reddens: Caught::Check("an operation completes after its deadline and only once"),
    },
    // Cancellation is a second operation, not a substitute for its target.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        if !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
            // The cancellation itself still gets the completion reserved for",
        replace: "        if false && !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
            // The cancellation itself still gets the completion reserved for",
        breaks: "a cancelled operation still gets its own completion",
        reddens: Caught::Check("a cancelled operation and its cancellation complete separately"),
    },
    // A plausible shifted clock is exposed only by comparing both publication doors.
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    write_u32(page, CLOCK_AT + clock::SHIFT, conversion.shift());",
        replace: "    write_u32(page, CLOCK_AT + clock::SHIFT, conversion.shift() - 1);",
        breaks: "the page and the kernel clock tell the same time",
        reddens: Caught::Check("the time page and the kernel clock tell the same time"),
    },
    // Leaving the sequence odd makes every correct reader spin until timeout.
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    write_u32(page, CLOCK_AT + clock::SEQ, Seq::end(Seq::begin(seq)));",
        replace: "    write_u32(page, CLOCK_AT + clock::SEQ, Seq::begin(seq));",
        breaks: "a published clock closes its update window",
        reddens: Caught::Check("the time page and the kernel clock tell the same time"),
    },
    // Publishing counter mode after revocation defeats the fail-closed reader path.
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    write_u32(page, CLOCK_AT + clock::MODE, mode);",
        replace: "    write_u32(page, CLOCK_AT + clock::MODE, MODE_COUNTER);",
        breaks: "a revoked clock reads as revoked",
        reddens: Caught::Check("revoking the clock sends readers to ring Uptime"),
    },
    // The time page mapped writable, which is a program able to tell every
    // other program a lie about the time.
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    let result = space.map_shared(TIME, &shared, Flags::NO_EXECUTE);",
        replace: "    let result = space.map_shared(TIME, &shared, Flags::NO_EXECUTE.union(Flags::WRITABLE));",
        breaks: "the time page is read-only to ring 3",
        reddens: Caught::Check("a program sees the time page and cannot write it"),
    },
    // Forgetting dead work strands both its completion and its reservation.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            if !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
                break;
            }
            ended += 1;",
        replace: "            let _ = entry;
            self.pending.lock_leaf().slots[index] = None;
            ended += 1;",
        breaks: "a dead program's work is finished, not forgotten",
        reddens: Caught::Check("work in flight is ended before the ring is released"),
    },
    // A separately allocated table can die beneath the deferred promise.
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        Ok(SlotReservation {
            table: Arc::clone(self),
            spent: false,
        })",
        replace: "        Ok(SlotReservation {
            table: if wost::arch::current::interrupt::in_trap() {
                Arc::clone(self)
            } else {
                Arc::new(HandleTable::new(self.quota).expect(\"mutation table\"))
            },
            spent: false,
        })",
        breaks: "a promise holds the table it is against",
        reddens: Caught::Check("a promise outlives the program that made it"),
    },
    // Q35 leaves a bus-zero-only walk looking plausible, so the proof counts
    // buses rather than functions (D-168).
    Mutation {
        file: "kernel/src/pci.rs",
        find: "                if entry.class.is_bridge()
                    && entry.header.is_bridge()",
        replace: "                if false
                    && entry.header.is_bridge()",
        breaks: "enumeration descends through bridges",
        reddens: Caught::Check("the bus walk descends through a bridge"),
    },
    // Two offsets in one header treated as two device identities. Exact-offset
    // duplicate checks cannot catch this; the claim must canonicalise first.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let at = at.function_base();
        if self.is_claimed(at) {",
        replace: "    let at = at;
        if self.is_claimed(at) {",
        breaks: "configuration offsets alias one function claim",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::function_claims_validate_headers_and_aliases_before_commit",
        },
    },
    // The ports a device decodes left free for anyone to claim, which is the
    // arrangement D-77 said proves the wrong thing.
    Mutation {
        file: "kernel/src/pci.rs",
        find: "        let owned = wost::arch::current::port::claim_spans::<BARS>(&requested[..port_count])",
        replace: "        let owned = wost::arch::current::port::claim_spans::<BARS>(&requested[..0])",
        breaks: "a device claim covers the ports it decodes",
        reddens: Caught::Check("a claimed device's io window is not claimable bare"),
    },
    // Consume the span handles without returning their bits; the witness
    // reacquires the same BDF, so registry-only cleanup cannot pass.
    Mutation {
        file: "kernel/src/pci.rs",
        find: "        if let Err(failure) = wost::arch::current::port::release_spans(ports) {
            self.ports = failure.spans;
            return ReleaseOutcome::refused(ReleaseError::Ports, self);
        }",
        replace: "        drop(ports);",
        breaks: "a transient pci lease returns every dynamic io span",
        reddens: Caught::Check("a claimed device's io window is not claimable bare"),
    },
    // A bridge's bus numbers decoded as a base address and then probed —
    // all ones written into the register that says which bus is behind it.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    match probe_bars_unleased(authority.bar_registers(), |event| access(function, event)) {",
        replace: "    match probe_bars_unleased(BARS, |event| access(function, event)) {",
        breaks: "a bridge has two base address registers, not six",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::the_claimed_header_alone_sets_the_public_bar_probe_extent",
        },
    },
    // Decode left enabled while BARs hold all ones; a fake device observes
    // the cycle where QEMU might forgive it.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "const PROBE_DECODE: u16 = command::IO_SPACE | command::MEMORY_SPACE;",
        replace: "const PROBE_DECODE: u16 = 0;",
        breaks: "bar probing disables both decode spaces",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_probe_is_one_restored_decode_off_transaction",
        },
    },
    // A window one past the port-space width must not truncate to zero.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if size >> u16::BITS == 0 {",
        replace: "    if true {",
        breaks: "an io window must fit before it is narrowed to a port count",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_probe_that_decoded_nothing_is_refused",
        },
    },
    // An out-of-range register count reaches past the fixed transaction arrays,
    // while every smaller header is refused before its first access.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if registers > BARS {",
        replace: "    if registers < BARS {",
        breaks: "bar probing accepts exactly the bounded header register counts",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_probe_refuses_only_register_counts_above_its_bound",
        },
    },
    // The high half is no longer all ones when the low mask is read.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        if let Some(high) = slots.high {
            modified |= 1 << high;",
        replace: "        if false && let Some(high) = slots.high {
            modified |= 1 << high;",
        breaks: "both halves of a wide bar are probed together",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_probe_is_one_restored_decode_off_transaction",
        },
    },
    // Bit two is an address bit in an I/O BAR, not the wide-memory type.
    // Treating it as width consumes the next header field.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        let high = if matches!(
            Bar::decode(original[low], 0)?,
            Bar::Memory { wide: true, .. }
        ) {",
        replace: "        let high = if original[low] & 0b110 == 0b100 {",
        breaks: "an io address bit cannot consume a second bar slot",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_io_address_bit_is_not_a_wide_memory_type",
        },
    },
    // A zero base is still an implemented narrow memory BAR when its probe
    // mask reports a window.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "                let Some(size) = memory_window_size(masks[index], 0, MemoryWidth::Bits32) else {",
        replace: "                let Some(size) = memory_window_size(0, 0, MemoryWidth::Bits32) else {",
        breaks: "an address-zero memory bar remains discoverable",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_address_zero_memory_bar_is_discovered_from_its_mask",
        },
    },
    // A successful probe returns while the device still holds probe values.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if restore.phase == ProbeRestorePhase::Bars {",
        replace: "    if false {",
        breaks: "a successful BAR probe restores every BAR before staging its command",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_probe_is_one_restored_decode_off_transaction",
        },
    },
    // Rollback restores BARs but forgets the command register after an access
    // fails; exhaustive cycle injection reaches every such exit.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "fn after_probe_error<E>(
    reason: ProbeError<E>,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
    restore: ProbeRestore,
) -> RawProbeFailure<E> {
    match finish_probe_restoration(restore, access) {",
        replace: "fn after_probe_error<E>(
    reason: ProbeError<E>,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
    mut restore: ProbeRestore,
) -> RawProbeFailure<E> {
    restore.command &= !PROBE_DISABLED;
    match finish_probe_restoration(restore, access) {",
        breaks: "a failed BAR probe restores decode and DMA state",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_probe_access_failure_restores_the_device",
        },
    },
    // Record each requested run before the all-or-none preflight finishes.
    // A conflict later in the list then strands all earlier runs.
    Mutation {
        file: "libs/port-space/src/lib.rs",
        find: "            room[index] = Some(span);
        }

        for span in room.iter().flatten() {
            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] |= bit(port);",
        replace: "            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] |= bit(port);
            }
            room[index] = Some(span);
        }

        for span in room.iter().flatten() {
            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] |= bit(port);",
        breaks: "a refused multi-span reservation records no earlier span",
        reddens: Caught::HostTest {
            package: "port-space",
            name: "tests::a_refused_multi_span_reservation_takes_nothing",
        },
    },
    // Keep the unique handles consumed but leave their bitmap bits set.
    Mutation {
        file: "libs/port-space/src/lib.rs",
        find: "                self.reserved[port / PORTS_PER_WORD] &= !bit(port);",
        replace: "                self.reserved[port / PORTS_PER_WORD] |= bit(port);",
        breaks: "a transient multi-span release clears every exact port",
        reddens: Caught::HostTest {
            package: "port-space",
            name: "tests::a_multi_span_release_is_exact_and_all_or_none",
        },
    },
    // An address keeps its register offset when asked for its device identity.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        Self(self.0 & !0xFF)",
        replace: "        Self(self.0)",
        breaks: "a function identity contains no register offset",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_offset_of_one_function_has_one_identity",
        },
    },
    // Consuming the probe result must retain every decoded window.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    pub const fn into_windows(self) -> [Option<Window>; BARS] {
        self.windows
    }",
        replace: "    pub const fn into_windows(self) -> [Option<Window>; BARS] {
        [None; BARS]
    }",
        breaks: "consuming probed bars returns their decoded windows",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::probed_bars_return_their_complete_slot_array",
        },
    },
    // Report release while retaining the command sublease.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        *slot = None;
        true",
        replace: "        *slot = Some(lease);
        true",
        breaks: "a settled PCI command returns its exact registry slot",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_leases_refuse_nesting_and_preserve_uncertain_retry_authority",
        },
    },
    // A wide BAR in a bridge's last slot would consume its bus-number field.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "            Some(core::iter::Iterator::next(&mut indices)?)",
        replace: "            let _ = core::iter::Iterator::next(&mut indices);
            None",
        breaks: "a wide bar cannot consume the field after the header's last slot",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_wide_bar_in_the_final_slot_is_refused_before_mutation",
        },
    },
    // A wide non-prefetchable BAR must not acquire a property merely because
    // other bits in the low word are set.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "                address: base | ((high as u64) << 32),
                wide: true,
                prefetchable: low & Self::PREFETCHABLE != 0,",
        replace: "                address: base | ((high as u64) << 32),
                wide: true,
                prefetchable: true,",
        breaks: "a wide bar reports only its actual prefetchable bit",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_base_address_register_decodes_to_its_window",
        },
    },
    // A non-power-of-two mask happens to be aligned and otherwise plausible.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        if !size.is_power_of_two() || !base.is_multiple_of(size) {",
        replace: "        if false || !base.is_multiple_of(size) {",
        breaks: "a bar window size is a power of two",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_invalid_bar_mask_is_refused_after_restoration",
        },
    },
    // A power-of-two mask does not authorize a base in the middle of that
    // window's own alignment unit.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        if !size.is_power_of_two() || !base.is_multiple_of(size) {",
        replace: "        if !size.is_power_of_two() || false {",
        breaks: "a bar base is aligned to its decoded window size",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_misaligned_bar_base_is_refused_after_restoration",
        },
    },
    // The first refused restore aborts rollback instead of continuing through
    // the command register and every older BAR.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "            if restore.modified(index) {
                let _ = access(ProbeAccess::WriteBar(index, restore.original[index]));
            }",
        replace: "            if restore.modified(index)
                && let Err(error) = access(ProbeAccess::WriteBar(index, restore.original[index]))
            {
                return Err((ProbeError::Restore(error), restore));
            }",
        breaks: "bar rollback continues after a restore is refused",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_restore_failure_still_attempts_every_later_restore",
        },
    },
    // Record the BDF before its fallible device/resource preparation. Any
    // later refusal then poisons a permanent registry slot.
    Mutation {
        file: "kernel/src/pci.rs",
        find: "fn release_clean_function(function: pci::FunctionClaim, reason: ClaimError) -> ClaimError {
    if let Err(failure) = pci::release_function(function) {
        retain_failed_function(failure.into_function());
        return ClaimError::FunctionRetained;
    }
    reason
}",
        replace: "fn release_clean_function(function: pci::FunctionClaim, reason: ClaimError) -> ClaimError {
    drop(function);
    reason
}",
        breaks: "a refused device preparation leaves its registry unchanged",
        reddens: Caught::Check("a device is claimed once and a refusal strands nothing"),
    },
    // Require memory decode but omit bus master from the transaction. MMIO
    // works and every DMA queue then waits forever without an error register.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        command::MEMORY_SPACE | command::BUS_MASTER,\n        0,\n        access,",
        replace: "        command::MEMORY_SPACE,\n        0,\n        access,",
        breaks: "bus master enable preserves and verifies every required command bit",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::bus_master_enable_is_verified_and_preserves_the_command",
        },
    },
    // If either required bit was already present, toggling instead of adding
    // both would turn the working half back off.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let installed = (before | set) & !clear;",
        replace: "    let installed = (before ^ set) & !clear;",
        breaks: "bus master enable preserves either required bit already set",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::bus_master_enable_preserves_either_already_required_bit",
        },
    },
    // Configuration failures are surfaced in the boot log, so an empty error
    // loses the only diagnosis available before a controller exists.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "            Self::Access => core::fmt::Write::write_str(f, \"configuration access failed\"),",
        replace: "            Self::Access => Ok(()),",
        breaks: "every pci enable error has a diagnostic description",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_enable_error_describes_its_cause",
        },
    },
    // Treat a successful configuration cycle as proof the device accepted the
    // command bits, skipping the readback which distinguishes the two claims.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "            Some(bits) if bits as u16 == wanted => return Ok(()),",
        replace: "            Some(_) => return Ok(()),",
        breaks: "a bus master command write is read back before success",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_refused_bus_master_enable_restores_the_exact_command",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "            Some(bits) if bits as u16 == wanted => return Ok(()),",
        replace: "            Some(bits) if bits as u16 & wanted == wanted => return Ok(()),",
        breaks: "an unwritten command bit can enable an unclaimed decoder",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bus_master_enable_refuses_and_restores_an_unwritten_command_bit",
        },
    },
    // Read the captured command word back without first restoring it.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let _ = access(at, CommandAccess::Write(state.before));\n    match access(at, CommandAccess::Read) {",
        replace: "    match access(at, CommandAccess::Read) {",
        breaks: "a successful pci lease restores its captured command word",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bus_master_lease_restores_the_exact_command_or_reports_uncertainty",
        },
    },
    // Return the initiating access error without restoring a command write
    // which may already have enabled only part of the device.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let _ = access(at, CommandAccess::Write(original));\n    match access(at, CommandAccess::Read) {",
        replace: "    return Err(failure);\n    #[expect(unreachable_code)]\n    match access(at, CommandAccess::Read) {",
        breaks: "every failed bus master access restores or reports uncertainty",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_failed_bus_master_access_restores_or_reports_uncertainty",
        },
    },
    // Admit an empty read to the zero-based NLB encoder. In release that
    // underflows to the largest command rather than panicking.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks == 0 {\n            return Err(TransferError::Empty);\n        }",
        replace: "        if false && blocks == 0 {\n            return Err(TransferError::Empty);\n        }",
        breaks: "an empty transfer has no nlb encoding",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::an_empty_transfer_has_no_nlb_encoding",
        },
    },
    // Admit the first count whose zero-based encoding no longer fits the
    // command's sixteen-bit NLB field.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks > MAX_TRANSFER_BLOCKS {\n            return Err(TransferError::TooManyBlocks);\n        }",
        replace: "        if false && blocks > MAX_TRANSFER_BLOCKS {\n            return Err(TransferError::TooManyBlocks);\n        }",
        breaks: "the nlb field has one exact maximum block count",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_nonempty_nlb_count_has_one_exact_boundary",
        },
    },
    // Wrap the LBA end back into the namespace instead of refusing the
    // overflow which would make a controller reach an unrelated block.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        let Some(end) = lba.checked_add(blocks as u64) else {\n            return Err(TransferError::OutsideNamespace);\n        };",
        replace: "        let end = lba.wrapping_add(blocks as u64);",
        breaks: "a transfer extent cannot wrap into its namespace",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_transfer_never_crosses_or_wraps_its_namespace",
        },
    },
    // Let the device write one byte past the allocation that proves the DMA
    // extent. Exact-size and short buffers differ only at this comparison.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if bytes > buffer_bytes {\n            return Err(TransferError::BufferTooSmall);\n        }",
        replace: "        if false && bytes > buffer_bytes {\n            return Err(TransferError::BufferTooSmall);\n        }",
        breaks: "a transfer fits wholly inside its dma allocation",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::transfer_bytes_must_fit_the_dma_buffer",
        },
    },
    // Replace the controller-bound validation with one known-good request;
    // the submission interceptor catches it before any doorbell.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        let request = self.validate_read(lba, blocks, buffer_bytes)?;",
        replace: "        let request = self\n            .validate_read(lba, blocks, buffer_bytes)\n            .or_else(|_| {\n                ReadTransfer::new(\n                    0,\n                    1,\n                    self.blocks,\n                    u64::from(self.block_bytes),\n                    u64::MAX,\n                )\n                .map_err(BringUpError::InvalidTransfer)\n            })?;",
        breaks: "raw nvme reads cannot bypass controller-bound validation",
        reddens: Caught::Check("invalid nvme reads stop before submission"),
    },
    // Treat a zeroed Identify response as a usable namespace. There is no LBA
    // zero when NSZE is zero, however plausible the remaining format looks.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks == 0 {\n            return Err(NamespaceError::Empty);\n        }",
        replace: "        if false && blocks == 0 {\n            return Err(NamespaceError::Empty);\n        }",
        breaks: "a namespace has at least one logical block",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_namespace_has_at_least_one_logical_block",
        },
    },
    // Accept the extended format table even though this host never enabled
    // LBAFEE and copied only the sixteen-entry prefix it negotiated.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if last_format >= MAX_LBA_FORMATS {\n            return Err(NamespaceError::TooManyFormats(u16::from(last_format) + 1));\n        }",
        replace: "        if false && last_format >= MAX_LBA_FORMATS {\n            return Err(NamespaceError::TooManyFormats(u16::from(last_format) + 1));\n        }",
        breaks: "only negotiated lba formats are accepted",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Ignore FLBAS's reserved bit, making a malformed selector indistinguishable
    // from a format this host knows how to interpret.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if formatted & 0x80 != 0 {\n            return Err(NamespaceError::ReservedFormatBits(formatted));\n        }",
        replace: "        if false && formatted & 0x80 != 0 {\n            return Err(NamespaceError::ReservedFormatBits(formatted));\n        }",
        breaks: "reserved formatted lba selector bits are refused",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Select a packed table entry beyond zero-based NLBAF. Its bytes are not a
    // supported format even when the fixed Identify buffer happens to hold it.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if selected > last_format {\n            return Err(NamespaceError::FormatOutsideList {\n                selected,\n                formats: last_format + 1,\n            });\n        }",
        replace: "        if false && selected > last_format {\n            return Err(NamespaceError::FormatOutsideList {\n                selected,\n                formats: last_format + 1,\n            });\n        }",
        breaks: "the formatted lba index is inside the reported table",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Interpret reserved LBAF control bits as relative-performance bits. The
    // low two are descriptive; every higher bit is an unknown layout contract.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if control & 0xfc != 0 {\n            return Err(NamespaceError::ReservedLbaFormatBits(control));\n        }",
        replace: "        if false && control & 0xfc != 0 {\n            return Err(NamespaceError::ReservedLbaFormatBits(control));\n        }",
        breaks: "every lba format control bit has an explicit policy",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_lba_format_control_value_has_one_explicit_policy",
        },
    },
    // Drop per-LBA metadata from the transfer length and pointer contract.
    // Even one byte makes the controller's data shape differ from the host's.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        replace: "        if false && metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        breaks: "selected lba metadata is never silently discarded",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Confuse support capabilities with active metadata settings. A device may
    // advertise both transfer forms while the selected LBAF uses no metadata.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        replace: "        if metadata != 0 || identify[27] != 0 || identify[28] != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        breaks: "metadata capabilities do not enable metadata settings",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Read protected blocks without providing or checking their protection
    // information, turning integrity errors into ordinary data.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if protection != 0 {\n            return Err(NamespaceError::Protection(protection));\n        }",
        replace: "        if false && protection != 0 {\n            return Err(NamespaceError::Protection(protection));\n        }",
        breaks: "enabled protection information is never silently discarded",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Admit every exponent at or above 512 bytes, restoring the old trust in a
    // byte which may be 32 and which the only storage caller cannot consume.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if block_shift != SUPPORTED_BLOCK_SHIFT {\n            return Err(NamespaceError::BlockSize(block_shift));\n        }",
        replace: "        if block_shift < SUPPORTED_BLOCK_SHIFT {\n            return Err(NamespaceError::BlockSize(block_shift));\n        }",
        breaks: "every lbads byte has one explicit supported policy",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_lbads_value_has_one_explicit_policy",
        },
    },
    // Replace refused device data with a known-good synthetic namespace. The
    // self-test queue interceptor catches the bypass before allocation or MMIO.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        let namespace = Namespace::parse(identify).map_err(BringUpError::InvalidNamespace)?;",
        replace: "        let namespace = Namespace::parse(identify)\n            .or_else(|_| {\n                let mut accepted = [0u8; NAMESPACE_IDENTIFY_BYTES];\n                accepted[0] = 1;\n                accepted[128 + 2] = 9;\n                Namespace::parse(&accepted)\n            })\n            .map_err(BringUpError::InvalidNamespace)?;",
        breaks: "invalid namespace data cannot bypass io queue admission",
        reddens: Caught::Check("invalid nvme namespace stops before io queues"),
    },
    // A disk of identical blocks hides a dropped LBA, so the namespace gives
    // every block a distinct signature (D-169).
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        command.words[10] = lba as u32;
        command.words[11] = (lba >> 32) as u32;",
        replace: "        command.words[10] = 0;
        command.words[11] = 0;",
        breaks: "a read returns the block that was asked for",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_command_puts_every_field_where_the_specification_says",
        },
    },
    // The block count sent unadjusted, so one block becomes two. The first
    // block is still exactly right; only the bytes past it say so.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        command.words[12] = transfer.zero_based_blocks() as u32;",
        replace: "        command.words[12] = transfer.zero_based_blocks() as u32 + 1;",
        breaks: "every count in this specification is zero-based",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_command_puts_every_field_where_the_specification_says",
        },
    },
    // The phase tag never flips: fine until the queue wraps, so the check
    // must issue more commands than the queue has entries.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "                if completion_head == 0 {
                    queue.phase = !queue.phase;
                }",
        replace: "                if false {
                    queue.phase = !queue.phase;
                }",
        breaks: "a completion queue wraps and stays readable",
        reddens: Caught::Check("the queues wrap and the phase tag still works"),
    },
    // A register file mapped write-back, which an emulator forgives and
    // hardware does not (`012` §2).
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            .union(Flags::NO_CACHE);",
        replace: "            .union(Flags::NONE);",
        breaks: "a device window is uncacheable",
        reddens: Caught::Check("the controller's registers are uncacheable"),
    },
    // A missing cluster bias reads valid sectors belonging to another file,
    // so the image corpus distinguishes adjacent clusters (D-170).
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        let index = (cluster - FIRST_DATA_CLUSTER) as u64;",
        replace: "        let index = cluster as u64;",
        breaks: "a cluster number is biased by two",
        reddens: Caught::Check("the kernel reads the file the host wrote"),
    },
    // A read bounded by the cluster rather than by the file, which hands back
    // the last cluster's tail: its previous occupant's data.
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        let wanted = into.len().min((entry.size - offset) as usize);",
        replace: "        let wanted = into.len();",
        breaks: "a read stops at the end of the file",
        reddens: Caught::Check("the kernel reads the file the host wrote"),
    },
    // The deleted marker taken for the end of the directory, which loses every
    // file below it. Only reachable because the corpus deletes one on purpose.
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "    if slot[0] == 0xE5 {
        return Slot::Deleted;
    }",
        replace: "    if slot[0] == 0xE5 {
        return Slot::End;
    }",
        breaks: "a deleted entry is not the end of a directory",
        reddens: Caught::Check("the kernel reads the file the host wrote"),
    },
    // The volume mounted at sector zero, which is where a reader that could
    // not be given an offset would look.
    Mutation {
        file: "kernel/src/fs.rs",
        find: "pub(crate) const VOLUME_SECTOR: u64 = 2048;",
        replace: "pub(crate) const VOLUME_SECTOR: u64 = 0;",
        breaks: "the volume is where the partition puts it",
        reddens: Caught::Check("the volume mounts and says what a fat32 volume says"),
    },
    // Treating `..` as a name lets a caller ask the resolver to escape its
    // starting handle, contrary to `012` section 5 (D-171).
    Mutation {
        file: "libs/path-walk/src/lib.rs",
        find: "        if component == b\"..\" {",
        replace: "        if false {",
        breaks: "a resolver cannot be asked to ascend",
        reddens: Caught::Check("no path can name anything above the handle"),
    },
    // Ascending past the start clamped rather than refused, which silently
    // answers a different question than the caller asked.
    Mutation {
        file: "libs/path-walk/src/lib.rs",
        find: "            depth = depth.checked_sub(1).ok_or(PathError::Escapes)?;",
        replace: "            depth = depth.saturating_sub(1);",
        breaks: "an escape is refused rather than clamped",
        reddens: Caught::Check("no path can name anything above the handle"),
    },
    // A component that is not a directory walked through, so a file's bytes
    // are read as directory entries — with the caller choosing the bytes.
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "                if !entry.is_directory() {
                    return Err(FatError::WrongKind);
                }",
        replace: "                if false {
                    return Err(FatError::WrongKind);
                }",
        breaks: "a file cannot be walked through",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::path_resolution_descends_only_through_directory_components",
        },
    },
    // Globally unique fixture names hide a missing parent comparison, so two
    // fixture directories deliberately contain `MOTD` (D-172).
    Mutation {
        file: "kernel/src/tmpfs.rs",
        find: "        if index == ROOT || entry.parent != directory {
            continue;
        }
        if &entry.name[..entry.name_len] != name {",
        replace: "        if index == ROOT {
            continue;
        }
        if &entry.name[..entry.name_len] != name {",
        breaks: "a directory holds only its own entries",
        reddens: Caught::Check("a second backend answers the same three operations"),
    },
    // A path walked *through* a device, which has nothing inside it — the
    // "everything is a file" conflation arriving as a defect.
    Mutation {
        file: "kernel/src/vfs.rs",
        find: "    if rest.is_empty() {
        Ok(Node::Device(device))
    } else {
        Err(VfsError::NotADirectory)
    }",
        replace: "    let _ = rest;
    Ok(Node::Device(device))",
        breaks: "a device is not a directory",
        reddens: Caught::Check("a device is not a file"),
    },
    // A tmpfs read bounded by the arena rather than by the file, which hands
    // back the next file's bytes.
    Mutation {
        file: "kernel/src/tmpfs.rs",
        find: "    let wanted = into.len().min(len - offset);",
        replace: "    let wanted = into.len();",
        breaks: "a read stops at the end of the file, in memory too",
        reddens: Caught::Check("a second backend answers the same three operations"),
    },
    // A list-only directory handle must not become traversal authority
    // (D-173).
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        if !held.contains(Rights::TRAVERSE) {
            return refuse(token, STATUS_DENIED);
        }",
        replace: "        if false {
            return refuse(token, STATUS_DENIED);
        }",
        breaks: "an open needs the right to traverse",
        reddens: Caught::Check("a ring operation cannot reach past its rights or its pages"),
    },
    // A file handle granted WRITE, which nothing can honour: there is no write
    // path, and a right that grants nothing is a syscall name in disguise.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "                held.intersection(Rights::UNIVERSAL).union(Rights::READ),
            ),",
        replace: "                held.intersection(Rights::UNIVERSAL).union(Rights::READ).union(Rights::WRITE),
            ),",
        breaks: "an open grants only what it can honour",
        reddens: Caught::Check("a completion carries a file handle"),
    },
    // A file opened through an attenuated directory regains the universal
    // rights its parent gave up. TRAVERSE authorizes READ, not re-delegation.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "                held.intersection(Rights::UNIVERSAL).union(Rights::READ),
            ),",
        replace: "                Rights::UNIVERSAL.union(Rights::READ),
            ),",
        breaks: "an open never restores a universal right",
        reddens: Caught::Check("opening a node does not restore universal rights"),
    },
    // The same widening defect on the other branch: copying a directory's
    // identity must not copy rights its parent did not hold.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            crate::vfs::Node::Directory(directory) => (crate::object::directory(directory), held),",
        replace: "            crate::vfs::Node::Directory(directory) => (
                crate::object::directory(directory),
                Rights::UNIVERSAL.union(held),
            ),",
        breaks: "an opened directory never restores a universal right",
        reddens: Caught::Check("opening a node does not restore universal rights"),
    },
    // A malformed OpenAt path inspected before its slot is promised; a full
    // table must report quota first, as Duplicate does.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        let Ok(reservation) = table.reserve_slot() else {
            return refuse(token, STATUS_NO_HANDLE_SLOT);
        };
        let mut path = [0u8; MAX_PATH];
        let Some(len) = self.read_data(submission.argument, submission.length, &mut path) else {
            return refuse(token, STATUS_BAD_BUFFER);
        };",
        replace: "        let mut path = [0u8; MAX_PATH];
        let Some(len) = self.read_data(submission.argument, submission.length, &mut path) else {
            return refuse(token, STATUS_BAD_BUFFER);
        };
        let Ok(reservation) = table.reserve_slot() else {
            return refuse(token, STATUS_NO_HANDLE_SLOT);
        };",
        breaks: "every handle-producing operation reserves its result before other admission work",
        reddens: Caught::Check("a full table refuses a handle operation at submission"),
    },
    // Duplicate's rights check pulled ahead of the promise; a full table
    // must report quota even for malformed rights.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        let named = submission.handle;",
        replace: "        let named = submission.handle;
        if u32::try_from(submission.argument)
            .ok()
            .and_then(Rights::from_bits)
            .is_none()
        {
            return refuse(token, STATUS_DENIED);
        }",
        breaks: "Duplicate reserves its result before validating requested rights",
        reddens: Caught::Check("a full table refuses a handle operation at submission"),
    },
    // A read that ignores its offset. Nothing holds a cursor, so the offset
    // alone says where the bytes came from.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        match crate::vfs::read(request.file, request.offset, &mut scratch[..request.wanted]) {",
        replace: "        match crate::vfs::read(request.file, 0, &mut scratch[..request.wanted]) {",
        breaks: "a read starts where it was told to",
        reddens: Caught::Check("a read is a submission with its own offset"),
    },
    // The buffer bound moved after the read. The output is unchanged, so
    // only the driver's command counter can tell the kernels apart (D-173).
    Mutation {
        file: "kernel/src/queue.rs",
        find: "    (end <= DATA_BYTES).then_some((at, len))",
        replace: "    let _ = end;
    Some((at, len))",
        breaks: "a buffer is checked before the device is asked",
        reddens: Caught::Check("a ring operation cannot reach past its rights or its pages"),
    },
    // A rights word read as its low half, so undefined bits are dropped
    // rather than refused (D-174).
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        let Some(declared) = u32::try_from(submission.argument)
            .ok()
            .and_then(Rights::from_bits)
        else {",
        replace: "        let Some(declared) = Rights::from_bits(submission.argument as u32) else {",
        breaks: "a rights word that does not fit is refused",
        reddens: Caught::Check("a word that does not fit is refused rather than truncated"),
    },
    // A file offset wrapped into the file rather than past it, so byte four
    // billion and one is answered with byte one.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        let offset = u32::try_from(submission.offset).unwrap_or(u32::MAX);",
        replace: "        let offset = submission.offset as u32;",
        breaks: "an offset that does not fit is past the end",
        reddens: Caught::Check("a word that does not fit is refused rather than truncated"),
    },
    // A cancellation token truncated, so a program ends an operation it did
    // not name.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        let Ok(target) = u32::try_from(submission.argument) else {
            return refuse(submission.token, STATUS_NOT_FOUND);
        };",
        replace: "        let target = submission.argument as u32;",
        breaks: "a token that does not fit names no operation",
        reddens: Caught::Check("a word that does not fit is refused rather than truncated"),
    },
    // D-133's exemption, put back. The waiter still runs on the slice already
    // armed, so only the counter distinguishes this from a working wake.
    Mutation {
        file: "kernel/src/thread.rs",
        find: "    if woken > 0 {
        #[cfg(feature = \"selftest\")]
        if interrupt::in_trap() {",
        replace: "    if woken > 0 && !interrupt::in_trap() {
        #[cfg(feature = \"selftest\")]
        if interrupt::in_trap() {",
        breaks: "a syscall wake arms the timer for what it woke",
        reddens: Caught::Check("a syscall wake arms the timer for what it woke"),
    },
    // A discriminant and its `match` arm drifting apart, which is how a
    // diagnosis comes to name the wrong guard (D-176).
    Mutation {
        file: "kernel/src/time.rs",
        find: "            6 => Self::Disagreed,",
        replace: "            6 => Self::NotBelievable,",
        breaks: "the clock's refusal decodes to the one that was recorded",
        reddens: Caught::Check("every refusal the clock can record reads back as itself"),
    },
    // A run offered from a region that is described but is not memory, which
    // is the hole the four probes fell into on VMware (D-177).
    Mutation {
        file: "libs/memory-map/src/lib.rs",
        find: "            stored[len] = region;\n            len += 1;",
        replace: "            if region.kind.is_direct_mapped() {\n                stored[len] = region;\n            }\n            len += 1;",
        breaks: "a probed run is memory, not merely described",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::every_accepted_region_informs_memory_policy",
        },
    },
    // Launch the program on its Echo route; only the handle count proves the
    // trap exercised Duplicate. Kernel source, because the driver cannot rebuild itself.
    Mutation {
        file: "kernel/src/user.rs",
        find: "    spawn(\"user-duplicate\", 6, DUPLICATE)",
        replace: "    spawn(\"user-duplicate\", 6, BEHAVE)",
        breaks: "the shipped program uses its bootstrap authority through Duplicate",
        reddens: Caught::Check("a program duplicates its bootstrap handle"),
    },
    // The identity read as a translating controller would rewrite it, which is
    // the state the check exists to catch (D-178).
    Mutation {
        file: "kernel/src/selftest.rs",
        find: "const UNTRANSLATED_MF2: (u8, u8) = (0xAB, 0x83);",
        replace: "const UNTRANSLATED_MF2: (u8, u8) = (0xAB, 0x41);",
        breaks: "the keyboard's identity is the untranslated one",
        reddens: Caught::Check("the keyboard identifies as one the decoder knows"),
    },
    // Distinct keys keep the translated and untranslated rounds independent.
    Mutation {
        file: "libs/ps2-keys/src/set1.rs",
        find: "        0x30 => Key::B,",
        replace: "        0x30 => Key::Pause,",
        breaks: "set 1's numbering is the specification's",
        reddens: Caught::Check("a translated keystroke decodes to the same key"),
    },
    // QEMU makes the bit and identity agree, so only omitted re-derivation is observable.
    Mutation {
        file: "kernel/src/input.rs",
        find: "    keyboard.decoder = decoder_for(&mut keyboard.controller);",
        replace: "    let _ = &mut keyboard.controller;",
        breaks: "the decoder is re-derived when translation changes",
        reddens: Caught::Check("a translated keystroke decodes to the same key"),
    },
    // Seeding against the stream makes an unadopted numbering change observable.
    Mutation {
        file: "kernel/src/input.rs",
        find: "        if translated != self.decoder.reads_translated() {",
        replace: "        if false && translated != self.decoder.reads_translated() {",
        breaks: "the numbering the stream proved is the one the decoder adopts",
        reddens: Caught::Check("the keyboard's numbering and the decoder's are the same"),
    },
    Mutation {
        file: "libs/ps2-keys/src/arriving.rs",
        find: "const BREAK: u8 = 0xF0;",
        replace: "const BREAK: u8 = 0xF1;",
        breaks: "set 2's break prefix is the byte set 2 actually sends",
        reddens: Caught::HostTest {
            package: "ps2-keys",
            name: "arriving::tests::one_keystroke_from_either_machine_decides_it",
        },
    },
    Mutation {
        file: "libs/ps2-keys/src/arriving.rs",
        find: "const SET_2_HIGHEST_MAKE: u8 = 0x83;",
        replace: "const SET_2_HIGHEST_MAKE: u8 = 0x7F;",
        breaks: "the probe's bounds are the two tables', not numbers beside them",
        reddens: Caught::HostTest {
            package: "ps2-keys",
            name: "arriving::tests::the_two_tables_still_bound_this_probe",
        },
    },
    Mutation {
        file: "libs/ps2-keys/src/arriving.rs",
        find: "const AMBIGUOUS: [u8; 2] = [0xAA, 0xAB];",
        replace: "const AMBIGUOUS: [u8; 2] = [0xAA, 0xAA];",
        breaks: "a reply in the stream cannot answer what the stream is asked",
        reddens: Caught::HostTest {
            package: "ps2-keys",
            name: "arriving::tests::an_identity_reply_in_the_stream_decides_nothing",
        },
    },
    Mutation {
        file: "kernel/src/input.rs",
        find: "pub(crate) const HUMAN_WAIT_MS: u64 = 45_000;",
        replace: "pub(crate) const HUMAN_WAIT_MS: u64 = 90_000;",
        breaks: "three unanswered keystroke rounds end as a red check, not a hang",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_human_wait_fits_the_harness_patience",
        },
    },
    Mutation {
        file: "xtask/templates/wizard-os.vmx.in",
        find: "nvme0.present = \"TRUE\"\nnvme1.present = \"TRUE\"",
        replace: "nvme0.present = \"TRUE\"\nnvme0:0.present = \"TRUE\"\nnvme1.present = \"TRUE\"",
        breaks: "the VMware machine offers a candidate worth retiring",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::the_vmx_boots_efi_and_logs_serial_to_a_file",
        },
    },
    Mutation {
        file: "libs/memory-map/src/lib.rs",
        find: "                region.len != 0\n                    && region.base < end\n                    && start < region.end()\n                    && !allowed(region.kind)",
        replace: "                region.len != 0\n                    && region.base <= start\n                    && start < region.end()\n                    && !allowed(region.kind)",
        breaks: "a forbidden claim nested inside an allowed span refuses the whole range",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "coverage_matches_every_byte_claim_in_a_bounded_space",
        },
    },
    Mutation {
        file: "libs/memory-map/src/lib.rs",
        find: "                if first.is_none_or(|known| base < known) && self.covers_memory(base, end) {",
        replace: "                if first.is_none_or(|known| base < known) && self.covers(base, end) {",
        breaks: "a named memory run is one the coverage contract also calls memory",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_named_memory_run_is_memory_by_the_other_contract",
        },
    },
    // Treat an unverified stop as verified; a fake device completes after
    // the allocator could already reuse the command's memory.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "            self.disposition = DmaDisposition::Retained;",
        replace: "            self.disposition = DmaDisposition::Quiesced;",
        breaks: "an unverified controller stop retains its dma memory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::unproven_stop_retains_every_reachable_run",
        },
    },
    // Return a retained DMA segment to the frame allocator. Safe access is
    // still gone, so only the allocator count can distinguish this defect.
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        if let Some(segment) = self.segment.take() {\n            core::mem::forget(segment);\n        }",
        replace: "        if let Some(segment) = self.segment.take() {\n            drop(segment);\n        }",
        breaks: "retained dma memory never returns to the allocator",
        reddens: Caught::Check("uncertain dma memory is never recycled"),
    },
    // Repair follows the red observation so the outstanding command remains observable.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "            core::hint::spin_loop();\n        }\n        self.fail_submitted(BringUpError::NotReady, data)",
        replace: "            core::hint::spin_loop();\n        }\n        Err(BringUpError::NotReady)",
        breaks: "a command timeout settles dma before returning",
        reddens: Caught::Check("an nvme timeout quiesces before dma memory returns"),
    },
    // Re-export the private closure that can hand any safe kernel module the
    // running `AddressSpace`, including its reclaim and arbitrary-unmap doors.
    Mutation {
        file: "kernel/src/mem.rs",
        find: "fn with_active_space<T>(",
        replace: "pub(crate) fn with_active_space<T>(",
        breaks: "the running address space has no broad safe borrow",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::the_running_space_has_no_broad_safe_borrow",
        },
    },
    // Forget every ownership entry after D-117's old fixed ceiling. The table
    // frames remain live and reachable, but reclamation silently loses them.
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "for table in self.tables.drain(..) {",
        replace: "for table in self.tables.drain(..).take(32) {",
        breaks: "a large address space returns every table frame",
        reddens: Caught::Check("a large space returns every table frame"),
    },
    // Skip terminal table reclamation while reporting success.
    Mutation {
        file: "kernel/src/user.rs",
        find: "        match space.reclaim(released) {",
        replace: "        match { drop(released); Ok::<usize, wost::arch::current::paging::ReclaimFailure>(0) } {",
        breaks: "a refused process construction returns its page tables",
        reddens: Caught::Check("every refused spawn returns every resource"),
    },
    // Admit an entry outside every executable segment. The image is otherwise
    // valid, so this reaches mapping unless the entry participates in preflight.
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !entry_is_executable {\n        return Err(LoadError::InvalidEntry(entry));\n    }",
        replace: "    if false && !entry_is_executable {\n        return Err(LoadError::InvalidEntry(entry));\n    }",
        breaks: "an ELF entry must be inside executable program memory",
        reddens: Caught::Check("the loader refuses what it may not map"),
    },
    // A second fabricated launch reuses the stack address retired by the first.
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "        let stack_index = table.slots[slot].stack_index;\n        table.slots[slot] = Slot {\n            stack_index,\n            ..Slot::VACANT\n        };",
        replace: "        table.slots[slot] = Slot::VACANT;",
        breaks: "a retired construction preserves its reusable kernel stack",
        reddens: Caught::Panic {
            marker: "address 0xffffffffffffffe8",
            why: "the second fabrication writes through the zeroed stack top before a verdict",
        },
    },
    // Return after the first injected retirement refusal without completing
    // the task-table retirement or publishing the scheduler slot.
    Mutation {
        file: "kernel/src/thread.rs",
        find: "    if refuse(SpawnPoint::Fabricated) {\n        if let Err(reason) = retire_prepared(slot) {\n            finish_refused_retirement(slot, reason);\n            return Err(PreparedFailure::before_binding(\n                SpawnError::Retirement { slot, reason },\n                binding,\n            ));\n        }\n        return Err(PreparedFailure::before_binding(SpawnError::Full, binding));\n    }",
        replace: "    if refuse(SpawnPoint::Fabricated) {\n        if let Err(reason) = retire_prepared(slot) {\n            return Err(PreparedFailure::before_binding(\n                SpawnError::Retirement { slot, reason },\n                binding,\n            ));\n        }\n        return Err(PreparedFailure::before_binding(SpawnError::Full, binding));\n    }",
        breaks: "a first retirement refusal is completed before slot publication",
        reddens: Caught::Check("a retirement refusal is settled before slot reuse"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "const MAX_REGIONS: usize = MAX_SEGMENTS + 1;",
        replace: "const MAX_REGIONS: usize = MAX_SEGMENTS;",
        breaks: "the largest image and its stack both fit the cleanup ledger",
        reddens: Caught::Check("a maximum image and stack fit the cleanup ledger"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "                if let Err(refusal) = undo(space, &placements[..done]) {\n                    return Err(LoadError::Rollback {\n                        reason: refusal.reason,\n                        pages: refusal.pages,\n                    });\n                }",
        replace: "                if let Err(refusal) = undo(space, &placements[..done]) {\n                    let _ = refusal;\n                }",
        breaks: "a refused loader rollback is returned with its exact progress",
        reddens: Caught::Check("cleanup refusals retain reachable tables"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "                    #[cfg(feature = \"selftest\")]\n                    if CLEANUP_REFUSAL_INJECTED.swap(false, Ordering::AcqRel) {\n                        REFUSED_MAPPING_RECLAIMS.fetch_add(1, Ordering::AcqRel);\n                        continue;\n                    }",
        replace: "                    #[cfg(feature = \"selftest\")]\n                    if CLEANUP_REFUSAL_INJECTED.swap(false, Ordering::AcqRel) {\n                        REFUSED_MAPPING_RECLAIMS.fetch_add(1, Ordering::AcqRel);\n                        return;\n                    }",
        breaks: "construction cleanup retries the exact mapping that refused once",
        reddens: Caught::Check("cleanup refusals retain reachable tables"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !extended_state_is_isolated() {\n        return Err(SpawnError::NoExtendedState);\n    }",
        replace: "    if false && !extended_state_is_isolated() {\n        return Err(SpawnError::NoExtendedState);\n    }",
        breaks: "ring three stays closed without isolated extended state",
        reddens: Caught::Check("missing extended state closes the user boundary"),
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: "    let Some(Ok(returned)) = mem::unmap_for_test(&mut context(), mapping) else {\n        return false;\n    };\n    drop(returned);",
        replace: "    drop(mapping);",
        breaks: "the live-root mapping check returns its owned frame",
        reddens: Caught::Check("a mapping added after the switch is live"),
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: "    let checked = check(&mut space);\n    let reclaimed = space.reclaim(None).is_ok();\n    drop(space);",
        replace: "    let checked = check(&mut space);\n    drop(space);\n    let reclaimed = true;",
        breaks: "throwaway address spaces explicitly return every table frame",
        reddens: Caught::Check("a second address space is refused"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if self.owned_pages != 0 {",
        replace: "        if false && self.owned_pages != 0 {",
        breaks: "reachable owned pages prevent page-table reclamation",
        reddens: Caught::Check("cleanup refusals retain reachable tables"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            drop(returned);\n            remaining -= 1;",
        replace: "            drop(returned);\n            remaining = 0;",
        breaks: "stack rollback loses its exact remaining cleanup authority",
        reddens: Caught::Check("partial stack rollback is retryable"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if let Err(cause) = self.verify_guard(slot) {\n            return match self.rollback_stack(slot, 1, 0) {\n                Ok(_) => Err(StackMapError::Mapping(cause)),\n                Err(cleanup) => Err(StackMapError::Rollback { cause, cleanup }),\n            };\n        }",
        replace: "        if let Err(cause) = self.verify_guard(slot) {\n            let _ = self.rollback_stack(slot, 1, 0);\n            return Err(StackMapError::Mapping(cause));\n        }",
        breaks: "guard rollback refusal retains typed ownership and progress",
        reddens: Caught::Check("guard rollback refusal retains the stack"),
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "    match space.reclaim(None) {",
        replace: "    match {\n        drop(space);\n        Ok::<usize, ReclaimFailure>(0)\n    } {",
        breaks: "a failed boot-space build reclaims its partially built tables",
        reddens: Caught::Check("a failed boot-space build returns every table frame"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "            Err(bound) => {\n                bootstrap.authority = Some(BootstrapAuthority::Bound(bound));\n                return Err(ReclaimRetention {\n                    refusal: ReclaimRefusal::TaskRelease,\n                    #[cfg(feature = \"selftest\")]\n                    observed_returned_pages: 0,\n                });\n            }",
        replace: "            Err(bound) => {\n                drop(bound);\n                return Ok(ReclaimSuccess {\n                    reclaimed: Reclaimed::default(),\n                    #[cfg(feature = \"selftest\")]\n                    observed_returned_pages: 0,\n                });\n            }",
        breaks: "a refused task release retains its linear retry authority",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        file: "kernel/src/thread.rs",
        find: "            .find(|slot| threads.queue.slots[*slot].state == State::Free && !zombie_slots[*slot])",
        replace: "            .find(|slot| threads.queue.slots[*slot].state == State::Free)",
        breaks: "scheduler admission excludes every slot with a retained zombie",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        let released = FRAMES.lock_leaf().free(self.index);",
        replace: "        let released = FRAMES.lock_leaf().free(usize::MAX);",
        breaks: "an impossible frame-ledger refusal fails stopped",
        reddens: Caught::Panic {
            marker: "frame ownership ledger refused a live handle",
            why: "the corrupted live handle must stop before ownership can be forgotten",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        let released = FRAMES.lock_leaf().free_contiguous(self.first, self.count);",
        replace: "        let released = FRAMES.lock_leaf().free_contiguous(usize::MAX, self.count);",
        breaks: "an impossible segment-ledger refusal fails stopped",
        reddens: Caught::Panic {
            marker: "segment ownership ledger refused a live handle",
            why: "the corrupted live segment must stop before ownership can be forgotten",
        },
    },
    // Consume a pending operation when its terminal post refuses; only a
    // witness inspecting ownership before the retry sees the loss.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            pending.slots[index] = Some(Pending {\n                settling: false,\n                ..entry\n            });\n            #[cfg(feature = \"selftest\")]\n            self.settlement_refusals.fetch_add(1, Ordering::AcqRel);",
        replace: "            pending.slots[index] = None;\n            #[cfg(feature = \"selftest\")]\n            self.settlement_refusals.fetch_add(1, Ordering::AcqRel);",
        breaks: "a refused terminal post retains its pending operation",
        reddens: Caught::Check("a refused settlement retains every owner"),
    },
    // Continue construction after the boot stack guard refused; only the
    // explicit boundary state distinguishes this from a normal launch.
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !crate::user_launch_is_safe() {\n        return Err(SpawnError::UnguardedBootStack);\n    }",
        replace: "    if false && !crate::user_launch_is_safe() {\n        return Err(SpawnError::UnguardedBootStack);\n    }",
        breaks: "an unguarded boot stack cannot open the user boundary",
        reddens: Caught::Check("an unguarded boot stack closes the user boundary"),
    },
    // Erase what makes a forgotten guard distinct from an ordinary holder;
    // the lock still excludes, so the witness must read the reason.
    Mutation {
        file: "wost/src/sync.rs",
        find: "        self.lock.forgotten_for_test.store(true, Ordering::Release);",
        replace: "        self.lock.forgotten_for_test.store(false, Ordering::Release);",
        breaks: "deliberately forgotten ownership is distinct from ordinary contention",
        reddens: Caught::Check("forgotten lock ownership is distinct from contention"),
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "    survey_the_bus();\n    survey_the_filesystems();\n    start_interrupts(routing.as_ref());",
        replace: "    survey_the_bus();\n    if wost::arch::current::pci::is_available() {\n        survey_the_filesystems();\n    }\n    start_interrupts(routing.as_ref());",
        breaks: "hardware-free namespaces do not depend on PCI availability",
        reddens: Caught::BootMarker(qemu::DEGRADED_FILESYSTEM_MARKER),
    },
    // Leave the failed controller's ownership record behind; the sibling
    // still maps, so only the selection ledger sees the incomplete retirement.
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let Some(slot) = self.devices.get_mut(mapping.device_slot) else {\n            panic!(\"a preflighted device ledger slot disappeared\");\n        };\n        *slot = None;",
        replace: "        let _ = mapping.device_slot;",
        breaks: "device-window teardown releases its reusable ledger slot",
        reddens: Caught::Check("device window reuses arbitrary-order gaps"),
    },
    // User roots copy top-level entries only once, so every future shared
    // device mapping needs its root slot before the first activation.
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    space\n        .reserve_shared_kernel_topology()\n        .map_err(ActivateError::SharedTopology)?;",
        replace: "    if false {\n        space\n            .reserve_shared_kernel_topology()\n            .map_err(ActivateError::SharedTopology)?;\n    }",
        breaks: "late device mappings share top-level topology with every user root",
        reddens: Caught::Check("late device mappings reach earlier user roots"),
    },
    // Once roots have copied the higher half, a new active-root PML4 entry
    // would exist in only one of them.
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if level == Level::Pml4\n            && !is_user_address(virtual_address)\n            && is_active()\n            && self.root() == active_root()\n        {",
        replace: "        if false\n            && level == Level::Pml4\n            && !is_user_address(virtual_address)\n            && is_active()\n            && self.root() == active_root()\n        {",
        breaks: "the active kernel cannot add an unshared higher-half root entry",
        reddens: Caught::Check("active kernel topology refuses a new shared root"),
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "    for _ in 0..=occupied.len() {",
        replace: "    for _ in 0..occupied.len() {",
        breaks: "first-fit gives up before checking the gap after every occupied extent",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "tests::first_fit_reuses_an_interior_gap_in_any_active_order",
        },
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "            return Some(cursor);",
        replace: "            return cursor.checked_add(align);",
        breaks: "first-fit skips the earliest aligned byte in an available gap",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "every_small_first_fit_matches_a_bytewise_oracle",
        },
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "            base: base & !(align - 1),",
        replace: "            base,",
        breaks: "a covering MMIO range begins after the unaligned bytes it must include",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "tests::a_covering_range_refuses_rounding_past_the_address_space",
        },
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "    pub const fn new(base: u64, len: u64) -> Option<Self> {\n        if len == 0 {",
        replace: "    pub const fn new(base: u64, len: u64) -> Option<Self> {\n        if false {",
        breaks: "an empty MMIO address range is admitted as a usable extent",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "tests::address_ranges_refuse_empty_and_wrapping_bounds",
        },
    },
    // Restore everything but stop enumeration anyway, reproducing the
    // first-candidate-only selector without conflating it with cleanup.
    Mutation {
        file: "kernel/src/main.rs",
        find: "    info!(\"nvme: candidate {ordinal} retired cleanly; trying next\");
    StorageSelection::Seeking
}",
        replace: "    info!(\"nvme: candidate {ordinal} retired cleanly; trying next\");
    StorageSelection::Halted
}",
        breaks: "nvme selection advances after an exact candidate retirement",
        reddens: Caught::Check("a later nvme controller survives an earlier failure"),
    },
    // Publish queue addresses before moving them into their cleanup owner.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    // Cleanup owns every published queue address before the enable write.
    let controller = Controller {",
        replace: "    let _ = write(&mut registers, register::CC, cc);
    let controller = Controller {",
        breaks: "nvme cleanup owns every published queue before controller enable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::nvme_cleanup_and_bus_master_authority_precede_the_enable_write",
        },
    },
    Mutation {
        file: "libs/timebase/src/lib.rs",
        find: "            Some(now) => Self::Deadline(now.saturating_add(timeout_ms.saturating_mul(1_000_000))),",
        replace: "            Some(_) => Self::Attempts(fallback_attempts),",
        breaks: "a calibrated poll is bounded by real time rather than iteration rate",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "tests::a_monotonic_poll_limit_expires_by_time_not_iteration_rate",
        },
    },
    Mutation {
        file: "libs/timebase/src/lib.rs",
        find: "            Self::Attempts(remaining) if *remaining > 0 => {",
        replace: "            Self::Attempts(remaining) if *remaining > 1 => {",
        breaks: "a clockless poll spends exactly its declared fallback attempts",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "tests::a_clockless_poll_limit_permits_its_exact_fallback_count",
        },
    },
    Mutation {
        file: "libs/timebase/src/lib.rs",
        find: "            Some(now) => Self::Deadline(now.saturating_add(timeout_ms.saturating_mul(1_000_000))),",
        replace: "            Some(now) => Self::Deadline(now.wrapping_add(timeout_ms.saturating_mul(1_000_000))),",
        breaks: "a poll deadline saturates instead of wrapping into the past",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "tests::a_deadline_saturates_instead_of_wrapping_into_the_past",
        },
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "const WORK_DEPTH: usize = 8;",
        replace: "const WORK_DEPTH: usize = 7;",
        breaks: "file-work admission accepts its exact documented finite capacity",
        reddens: Caught::Check("file work pressure is bounded at admission"),
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        matches!(
            self,
            Self::Timer { .. }
                | Self::Read {
                    state: ReadState::Queued,
                    ..
                }
        )",
        replace: "        matches!(self, Self::Timer { .. } | Self::Read { .. })",
        breaks: "a running file read remains owned through teardown until its worker settles",
        reddens: Caught::Check("a delayed read preserves interrupts and exact teardown"),
    },
    Mutation {
        file: "kernel/src/wait.rs",
        find: "            let mut waiters = self.waiters.lock(context);
            if ready() {",
        replace: "            let mut waiters = self.waiters.lock(context);
            if false && ready() {",
        breaks: "a waiter blocks without rereading its condition after registration",
        reddens: Caught::Check("a waiter rereads its condition before blocking"),
    },
    Mutation {
        file: "kernel/src/wait.rs",
        find: "        let switched = registered && thread::wait_from_trap(frame, until);",
        replace: "        let switched = registered && false && thread::wait_from_trap(frame, until);",
        breaks: "RingWait hands its caller away through the live trap frame",
        reddens: Caught::Check("a program waits through its trap frame"),
    },
    Mutation {
        file: "kernel/src/thread.rs",
        find: "            entity: Entity::new(weight),",
        replace: "            entity: Entity::new(BASE_WEIGHT),",
        breaks: "the weighted scheduler check constructs two equal entities",
        reddens: Caught::Check("runtime divides in proportion to weight"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !no_execute_is_enabled() {\n        return Err(SpawnError::NoExecuteProtection);\n    }",
        replace: "    if false && !no_execute_is_enabled() {\n        return Err(SpawnError::NoExecuteProtection);\n    }",
        breaks: "ring three requires enforced non-executable mappings",
        reddens: Caught::Check("no-execute protection closes the user boundary"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !crate::time::is_calibrated() {\n        return Err(SpawnError::NoClock);\n    }",
        replace: "    if false && !crate::time::is_calibrated() {\n        return Err(SpawnError::NoClock);\n    }",
        breaks: "an uncalibrated boot never launches an unpreemptible program",
        reddens: Caught::Check("an uncalibrated clock admits no user or timer"),
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "                if !crate::time::is_calibrated() {\n                    return Some(refuse(token, STATUS_UNAVAILABLE));\n                }",
        replace: "                if false && !crate::time::is_calibrated() {\n                    return Some(refuse(token, STATUS_UNAVAILABLE));\n                }",
        breaks: "a clockless Timer is refused before it becomes pending",
        reddens: Caught::Check("an uncalibrated clock admits no user or timer"),
    },
    Mutation {
        file: "kernel/src/thread.rs",
        find: "    if spawned.is_err() {\n        IDLE_STATE.store(IDLE_UNSTARTED, Ordering::Release);\n        return;\n    }",
        replace: "    if spawned.is_err() {\n        IDLE_STATE.store(IDLE_STARTING, Ordering::Release);\n        return;\n    }",
        breaks: "a refused idle spawn leaves its reservation permanently burned",
        reddens: Caught::Check("a failed idle start can be retried exactly once"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !crate::queue::worker_is_ready() {\n        return Err(SpawnError::NoServiceWorker);\n    }",
        replace: "    if false && !crate::queue::worker_is_ready() {\n        return Err(SpawnError::NoServiceWorker);\n    }",
        breaks: "ring three opens without a thread able to reclaim it",
        reddens: Caught::Check("a busy machine reaps a program without another spawn"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if stored {\n        crate::queue::request_reap();\n    }",
        replace: "    let _ = stored;",
        breaks: "retirement leaves the service worker asleep beside a zombie",
        reddens: Caught::Check("a busy machine reaps a program without another spawn"),
    },
    Mutation {
        file: "libs/handle-space/src/lib.rs",
        find: "    identity.checked_add(1)",
        replace: "    Some(identity.wrapping_add(1))",
        breaks: "table identity arithmetic wraps its exhaustion sentinel to zero",
        reddens: Caught::HostTest {
            package: "handle-space",
            name: "tests::table_identity_exhaustion_never_wraps",
        },
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        let successor = handle_space::next_table_identity(self.next_table)\n            .ok_or(HandleError::IdentityExhausted)?;",
        replace: "        let successor = self.next_table.wrapping_add(1);",
        breaks: "handle-table construction crosses the exhausted identity boundary",
        reddens: Caught::Check("table identities never wrap or repeat"),
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        self.next_table = successor;",
        replace: "        if successor != u64::MAX {\n            self.next_table = successor;\n        }",
        breaks: "two simultaneously live handle tables receive one ownership identity",
        reddens: Caught::Check("table identities never wrap or repeat"),
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    model.io_apic_count += 1;",
        replace: "                    model.io_apic_count = 1;",
        breaks: "the MADT inventory discards every I/O APIC after the first",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_bounded_interrupt_inventory_keeps_every_controller",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    let Some(slot) = model.io_apics.get_mut(model.io_apic_count) else {\n                        return Err(InterruptError::TooManyIoApics { limit: IO_APICS });\n                    };",
        replace: "                    let Some(slot) = model.io_apics.get_mut(model.io_apic_count) else {\n                        break;\n                    };",
        breaks: "an over-capacity MADT is silently truncated into a usable inventory",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::an_interrupt_inventory_refuses_overflow_instead_of_truncating",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/io.rs",
        find: "        let Some(distance) = entries.checked_sub(1) else {\n            return None;\n        };",
        replace: "        let distance = entries.saturating_sub(1);",
        breaks: "an I/O APIC reporting zero entries claims its base GSI",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "io::tests::a_range_is_non_empty_and_stays_inside_the_gsi_namespace",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/io.rs",
        find: "    Ok(ranges.iter().position(|range| range.contains(gsi)))",
        replace: "    Ok(ranges\n        .first()\n        .filter(|range| range.contains(gsi))\n        .map(|_| 0))",
        breaks: "GSI ownership search stops at the first I/O APIC",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "io::tests::a_gsi_selects_its_later_controller_across_a_gap",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/io.rs",
        find: "            if ranges[first].overlaps(ranges[second]) {",
        replace: "            if ranges[first].overlaps(ranges[second]) && first == second {",
        breaks: "two I/O APICs may ambiguously own the same GSI",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "io::tests::any_overlap_refuses_the_complete_inventory",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "#![cfg_attr(\n    not(test),\n    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)\n)]",
        replace: "",
        breaks: "a hostile-input parser regains expect, unwrap and panic shortcuts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "boundary::tests::target_boundaries_keep_panicking_shortcuts_out",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/port.rs",
        find: "pub struct PortSpan {\n    base: u16,\n    len: usize,\n}",
        replace: "pub struct PortSpan {\n    base: u16,\n    len: usize,\n}\n\nimpl PortSpan {\n    pub const fn base(&self) -> u16 {\n        self.base\n    }\n}",
        breaks: "the dynamic port ownership lease regains an uncalled register API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "boundary::tests::target_boundaries_keep_panicking_shortcuts_out",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        if first_data_sector >= total_sectors as u64 {",
        replace: "        if false && first_data_sector >= total_sectors as u64 {",
        breaks: "a FAT table extent at or beyond the volume is accepted",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::impossible_volume_geometry_is_refused_before_arithmetic",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        if (root_cluster - FIRST_DATA_CLUSTER) as u64 >= data_clusters {",
        replace: "        if false && (root_cluster - FIRST_DATA_CLUSTER) as u64 >= data_clusters {",
        breaks: "a FAT root directory may begin outside the complete data clusters",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::the_root_cluster_must_lie_inside_the_data_area",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        if data_clusters + FIRST_DATA_CLUSTER as u64 > fat_entries {",
        replace: "        if false && data_clusters + FIRST_DATA_CLUSTER as u64 > fat_entries {",
        breaks: "a FAT too short to index every advertised data cluster is accepted",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::the_fat_indexes_every_advertised_data_cluster",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        if data_clusters < MIN_FAT32_CLUSTERS || data_clusters > MAX_FAT32_CLUSTERS {",
        replace: "        if false || data_clusters > MAX_FAT32_CLUSTERS {",
        breaks: "a FAT12 or FAT16 cluster count is accepted as FAT32",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_fat32_volume_has_at_least_the_fat32_cluster_count",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        if data_clusters < MIN_FAT32_CLUSTERS || data_clusters > MAX_FAT32_CLUSTERS {",
        replace: "        if data_clusters < MIN_FAT32_CLUSTERS || false {",
        breaks: "a FAT32 data domain reaches reserved cluster numbers",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_fat32_volume_stops_before_reserved_cluster_numbers",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "            || (extended_flags & MIRRORING_DISABLED == 0 && selector != 0)",
        replace: "            || false",
        breaks: "a mirrored FAT32 volume accepts a meaningless inactive-table selector",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::extended_flags_reject_reserved_bits_and_meaningless_selectors",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "            || active_fat >= fats",
        replace: "            || false",
        breaks: "an active FAT selector may name a table the volume does not have",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::an_active_fat_must_name_an_existing_copy",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "                + self.active_fat as u64 * self.sectors_per_fat as u64",
        replace: "                + self.active_fat as u64 * 0",
        breaks: "FAT reads ignore the selected active table",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::extended_flags_choose_the_fat_that_is_read",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        if let Next::Cluster(cluster) = next\n            && !self.bpb.holds(cluster)\n        {\n            return Err(FatError::BadCluster(cluster));\n        }",
        replace: "",
        breaks: "a FAT chain link may name a marker or a cluster outside this volume",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_chain_link_must_name_a_cluster_in_this_volume",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        let skip_clusters = offset / cluster_bytes;",
        replace: "        let skip_clusters = offset;",
        breaks: "a FAT byte offset is mistaken for a cluster count",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_maximum_file_offset_does_not_overflow_chain_progress",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "    const fn chain_limit(&self) -> u32 {\n        self.bpb.clusters()\n    }",
        replace: "    const fn chain_limit(&self) -> u32 {\n        1 << 24\n    }",
        breaks: "a FAT chain walk is bounded by the volume that could hold it",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_cyclic_chain_is_refused_within_the_volume",
        },
    },
    Mutation {
        file: "tools/fat32-corpus/foreign-fat32.sparse",
        find: "sector 7 ",
        replace: "sector 1 ",
        breaks: "the foreign FAT envelope records one sector twice and omits another",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::foreign::foreign_envelope_is_exact_and_hostile_records_are_refused",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "                Next::Cluster(next) => {\n                    let Some(visited) = cursor.visited.checked_add(1) else {\n                        return Err(FatError::Unbounded);\n                    };\n                    if visited >= self.chain_limit() {\n                        return Err(FatError::Unbounded);\n                    }\n                    cursor.cluster = next;\n                    cursor.sector = 0;\n                    cursor.visited = visited;\n                }\n                Next::End => {\n                    cursor.done = true;\n                    return Ok(DirectoryChunk::EMPTY);\n                }",
        replace: "                Next::Cluster(_) | Next::End => {\n                    cursor.done = true;\n                    return Ok(DirectoryChunk::EMPTY);\n                }",
        breaks: "a FAT directory walk silently stops after its first cluster",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::foreign::both_readers_agree_on_the_adversarial_foreign_layout",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "            workspace: Paths::new(workspace_root, Some(&build)),",
        replace: "            workspace: Paths::new(paths.root().to_path_buf(), Some(&build)),",
        breaks: "mutation builds run from the user checkout instead of the tracked copy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_session_builds_only_from_its_tracked_copy",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "    file.try_lock().with_context(|| {",
        replace: "    std::io::Result::<()>::Ok(()).with_context(|| {",
        breaks: "two mutation sweeps can use one sandbox concurrently",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_killed_build_or_boot_resumes_outside_the_user_tree",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "    write_source(&destination, &bytes)?;\n    ensure!(\n        read_bounded_bytes(\n            &destination,\n            MAX_TRACKED_FILE_BYTES,\n            \"restored mutation source\",\n        )? == bytes,",
        replace: "    ensure!(\n        read_bounded_bytes(\n            &destination,\n            MAX_TRACKED_FILE_BYTES,\n            \"restored mutation source\",\n        )? == bytes,",
        breaks: "resume leaves the interrupted mutation applied in its sandbox",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_killed_build_or_boot_resumes_outside_the_user_tree",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: ".max_by_key(|journal| journal.generation)",
        replace: ".min_by_key(|journal| journal.generation)",
        breaks: "resume chooses stale progress over the newest complete journal generation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::journal_uses_the_newest_complete_slot_and_falls_back_from_a_torn_write",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "        restore_workspace(\n            &self.snapshot,\n            self.workspace.root(),\n            &self.baseline.tracked,\n        )?;\n        self.journal.completed.push(outcome);",
        replace: "        self.journal.completed.push(outcome);",
        breaks: "a completed mutation leaves collateral tracked or generated files in its sandbox",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_finished_mutation_restores_the_whole_sandbox",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "            verdict: if survived == 0 && inconclusive == 0 {",
        replace: "            verdict: if true || survived == 0 && inconclusive == 0 {",
        breaks: "a failed mutation sweep is serialized as a passing report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_completed_report_binds_totals_and_verdict_to_every_outcome",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "            .max_by_key(|(_, report)| report.sequence))",
        replace: "            .min_by_key(|(_, report)| report.sequence))",
        breaks: "an older passing sweep hides the newest failed mutation report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_new_failed_sweep_supersedes_but_does_not_erase_an_older_pass",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "        _lock: pass_lock,",
        replace: "        _lock: {\n            drop(pass_lock);\n            acquire_lock(&state_root.join(\"unrelated-lock\"))?\n        },",
        breaks: "a newer mutation sweep can supersede a pass while its consumer publishes it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_current_pass_blocks_a_newer_sweep_until_consumed",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "            self.commit == baseline.commit\n                && self.registry == registry",
        replace: "            true\n                && self.registry == registry",
        breaks: "a passing mutation report is accepted for another commit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_completed_report_binds_totals_and_verdict_to_every_outcome",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "        ensure!(\n            report.encode_with_checksum() == text,\n            \"mutation report is not in its canonical encoding\"\n        );",
        replace: "        let _ = text;",
        breaks: "a checksummed mutation report has multiple accepted byte encodings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_completed_report_binds_totals_and_verdict_to_every_outcome",
        },
    },
    Mutation {
        file: "xtask/src/cargo.rs",
        find: "        .env(\"CARGO_TARGET_DIR\", child_process_path(paths.target_dir()));",
        replace: "        .env_remove(\"CARGO_TARGET_DIR\");",
        breaks: "isolated cargo builds write outside their private target directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::a_cargo_command_uses_the_paths_private_target",
        },
    },
    Mutation {
        file: "xtask/src/mutate.rs",
        find: "            marker: \"faulting address 0x0000600000000000\",",
        replace: "            marker: \"\",",
        breaks: "a terminal mutation can name no exact panic cause",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_panicking_mutation_names_an_exact_marker_and_why_it_is_terminal",
        },
    },
    Mutation {
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
        file: "libs/panic-record/src/lib.rs",
        find: "                '\\n' => Some(\"\\\\n\"),",
        replace: "                '\\n' => None,",
        breaks: "a panic body can forge its terminal record with a physical newline",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::a_hostile_body_cannot_forge_the_terminal_record",
        },
    },
    Mutation {
        file: "libs/panic-record/src/lib.rs",
        find: "                '\\r' => Some(\"\\\\r\"),",
        replace: "                '\\r' => None,",
        breaks: "a panic body can retain a physical carriage return",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::every_ascii_character_uses_the_canonical_escape_encoding",
        },
    },
    Mutation {
        file: "libs/panic-record/src/lib.rs",
        find: "                '\\\\' => Some(\"\\\\\\\\\"),",
        replace: "                '\\\\' => None,",
        breaks: "a literal escape sequence is indistinguishable from a physical panic line break",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::the_escape_encoding_is_injective_over_ascii_pairs",
        },
    },
    Mutation {
        file: "libs/panic-record/src/lib.rs",
        find: "            core::fmt::Write::write_str(&mut *self.writer, escaped)?;",
        replace: "            let _ = core::fmt::Write::write_str(&mut *self.writer, escaped);",
        breaks: "an escaped panic segment can fail without withholding completion",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::writer_errors_propagate_from_plain_and_escaped_segments",
        },
    },
    Mutation {
        file: "libs/panic-record/src/lib.rs",
        find: "                core::fmt::Write::write_str(&mut *self.writer, &text[start..index])?;",
        replace: "                let _ = core::fmt::Write::write_str(&mut *self.writer, &text[start..index]);",
        breaks: "a refused plain segment before an escape is reported as written",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::writer_errors_propagate_from_plain_and_escaped_segments",
        },
    },
    Mutation {
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
        file: "xtask/src/qemu.rs",
        find: "    if panic.is_some() && !*panic_complete {\n        bail!(\"the kernel continued a single-line panic record\");\n    }",
        replace: "    if panic.is_some() && !*panic_complete {\n        return Ok(Some(PanicTransition::Unchanged));\n    }",
        breaks: "the single-line panic grammar accepts a continuation or second opening record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_panic_outcome_preserves_earlier_failures_and_its_exact_transcript",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        if panic.is_none() {\n            bail!(\"the kernel completed a panic record it never started\");\n        }",
        replace: "        if false && panic.is_none() {\n            bail!(\"the kernel completed a panic record it never started\");\n        }",
        breaks: "a completion record is accepted without an opening record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        if *panic_complete {\n            bail!(\"the kernel completed its panic record more than once\");\n        }",
        replace: "        if false && *panic_complete {\n            bail!(\"the kernel completed its panic record more than once\");\n        }",
        breaks: "a panic record is allowed to publish completion more than once",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    if *panic_complete {\n        bail!(\"the kernel printed serial data after its terminal panic record\");\n    }",
        replace: "    if false && *panic_complete {\n        bail!(\"the kernel printed serial data after its terminal panic record\");\n    }",
        breaks: "serial data is accepted after a terminal panic record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        return Ok(Some(PanicTransition::Completed));",
        replace: "        return Ok(Some(PanicTransition::Unchanged));",
        breaks: "a complete panic record never releases the bounded observer",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    fn publish(self, panicked: &AtomicBool) {\n        if self == Self::Completed {\n            panicked.store(true, Ordering::Release);\n        }\n    }",
        replace: "    fn publish(self, panicked: &AtomicBool) {\n        panicked.store(true, Ordering::Release);\n    }",
        breaks: "a panic opening line releases QEMU before the transcript is complete",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "fn is_complete_panic_record(line: &str) -> bool {\n    terminated_serial_text(line).and_then(|line| line.strip_prefix(\"[error] \"))\n        == Some(COMPLETE_MARKER)\n}",
        replace: "fn is_complete_panic_record(line: &str) -> bool {\n    line.contains(COMPLETE_MARKER)\n}",
        breaks: "a malformed or non-error panic terminator completes the record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    if line.starts_with(OPEN_PREFIX) {",
        replace: "    if line.contains(OPEN_PREFIX.trim_start_matches(\"[error] \")) {",
        breaks: "an unframed line can forge the kernel panic opening record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "fn is_exact_no_x2apic_diagnosis(line: &str) -> bool {\n    is_exact_log_message(line, LogSeverity::Warning, NO_X2APIC_DIAGNOSIS)\n}",
        replace: "fn is_exact_no_x2apic_diagnosis(line: &str) -> bool {\n    line.contains(\"no x2apic mode\")\n}",
        breaks: "an arbitrary prior line can forge the no-x2APIC diagnosis",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_degraded_no_x2apic_requirement_uses_the_exact_warning_record",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(QEMU_TCG_ISOLATION_MARKER),\n    NO_X2APIC_RECORD,",
        replace: "const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(QEMU_TCG_ISOLATION_MARKER),\n    RequiredLogRecord::warning(\"no x2apic mode\"),",
        breaks: "the settled degraded boot requires only a no-x2APIC phrase instead of the canonical warning record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_degraded_no_x2apic_requirement_uses_the_exact_warning_record",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        (Some(_), false) => bail!(\"QEMU stopped with an incomplete serial panic record\"),",
        replace: "        (Some(transcript), false) => Ok(Some(transcript)),",
        breaks: "a truncated panic report is credited as complete",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_panic_outcome_preserves_earlier_failures_and_its_exact_transcript",
        },
    },
    Mutation {
        file: "libs/panic-record/src/lib.rs",
        find: "    writeln!(writer, \"[error] {COMPLETE_MARKER}\")",
        replace: "    Ok(())",
        breaks: "the kernel omits the terminal panic record",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::completion_requires_every_prior_character_write",
        },
    },
    Mutation {
        file: "libs/panic-record/src/lib.rs",
        find: "        write!(record, \"{OPEN_PREFIX}{thread}': {body}\")?;",
        replace: "        let _ = write!(record, \"{OPEN_PREFIX}{thread}': {body}\");",
        breaks: "the kernel terminates a panic record whose body write failed",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::completion_requires_every_prior_character_write",
        },
    },
    Mutation {
        file: "kernel/src/panic.rs",
        find: "        let _ = write_record(uart, thread, info);",
        replace: "        let _ = writeln!(uart, \"[error] KERNEL PANIC on thread '{thread}': {info}\");",
        breaks: "the kernel writes PanicInfo bytes around the single-line adapter",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "libs/panic-record/src/lib.rs",
        find: "    writer.write_char('\\n')?;",
        replace: "    let _ = writer.write_char('\\n');",
        breaks: "the kernel emits completion after its body separator was refused",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::completion_requires_every_prior_character_write",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        *panic = Some(text.to_string());",
        replace: "        *panic = Some(text.trim_end().to_string());",
        breaks: "panic attribution silently drops trailing body whitespace",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        let text = terminated_serial_text(line)\n            .context(\"the kernel printed an unterminated panic opening record\")?;",
        replace: "        let text = terminated_serial_text(line).unwrap_or(line);",
        breaks: "an unterminated panic opening is accepted into the v1 record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    if text.contains('\\r') || text.contains('\\n') {\n        return None;\n    }",
        replace: "    let _ = text.contains('\\r') || text.contains('\\n');",
        breaks: "a panic record retains an internal or doubled physical carriage return",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        file: "kernel/src/arch/x86_64/uart.rs",
        find: "        if !self.wait_for(line_status::TRANSMIT_HOLDING_EMPTY) {\n            return Err(fmt::Error);\n        }",
        replace: "        let _ = self.wait_for(line_status::TRANSMIT_HOLDING_EMPTY);",
        breaks: "the UART reports a refused byte as transmitted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_uart_reports_every_refused_byte_to_its_formatter",
        },
    },
    Mutation {
        file: "kernel/src/arch/x86_64/uart.rs",
        find: "                self.write_byte(b'\\r')?;",
        replace: "                let _ = self.write_byte(b'\\r');",
        breaks: "the UART formatter hides a refused carriage return",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_uart_reports_every_refused_byte_to_its_formatter",
        },
    },
    Mutation {
        file: "kernel/src/arch/x86_64/uart.rs",
        find: "            self.write_byte(byte)?;",
        replace: "            let _ = self.write_byte(byte);",
        breaks: "the UART formatter hides a refused payload byte",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_uart_reports_every_refused_byte_to_its_formatter",
        },
    },
    Mutation {
        file: "libs/panic-record/src/lib.rs",
        find: "#![cfg_attr(\n    not(test),\n    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)\n)]",
        replace: "",
        breaks: "arbitrary panic text regains production panic shortcuts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "boundary::tests::target_boundaries_keep_panicking_shortcuts_out",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "this processor has no x2apic mode, and xapic needs page tables",
        replace: "this processor has no x2apic mode",
        breaks: "the no-x2APIC attribution grammar drifts from the emitted warning",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "const EXPECTED_SELFTEST_CHECKS: usize = 261;",
        replace: "const EXPECTED_SELFTEST_CHECKS: usize = 260;",
        breaks: "the host accepts a different machine-check population than the kernel",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    if observation.failed.len() != verdict.failed {",
        replace: "    if false && observation.failed.len() != verdict.failed {",
        breaks: "a failed verdict may omit the names of its red checks",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_failed_verdict_must_name_every_failed_check",
        },
    },
    Mutation {
        file: "libs/eevdf/src/lib.rs",
        find: "        if self.slice_is_spent() {\n            return 0;\n        }",
        replace: "        if false && self.slice_is_spent() {\n            return 0;\n        }",
        breaks: "a thread past its deadline wraps into an enormous remaining slice",
        reddens: Caught::HostTest {
            package: "eevdf",
            name: "tests::a_thread_past_its_deadline_has_no_slice_left",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "        match self.offset.checked_add(self.file_size) {\n            Some(end) => end <= bytes as u64,\n            None => false,\n        }",
        replace: "        self.offset.wrapping_add(self.file_size) <= bytes as u64",
        breaks: "a segment extent wraps around into the elf image",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::a_loader_can_ask_whether_the_contents_fit",
        },
    },
    Mutation {
        file: "libs/frame-space/src/lib.rs",
        find: "        for frame in first..end {\n            self.check_releasable(frame)?;\n        }",
        replace: "        self.check_releasable(first)?;",
        breaks: "a multi-frame free mutates before its complete run is valid",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::freeing_a_run_is_all_or_nothing",
        },
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "        if end > region_len {",
        replace: "        if end > region_len.saturating_add(1) {",
        breaks: "an mmio access may end one byte beyond its claimed region",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "tests::a_span_one_byte_past_the_region_is_refused",
        },
    },
    Mutation {
        file: "libs/path-walk/src/lib.rs",
        find: "            depth = depth.checked_sub(1).ok_or(PathError::Escapes)?;",
        replace: "            depth = depth.saturating_sub(1);",
        breaks: "ascending above a directory handle is clamped to its root",
        reddens: Caught::HostTest {
            package: "path-walk",
            name: "tests::ascending_past_the_start_is_refused_not_clamped",
        },
    },
    Mutation {
        file: "libs/pixel/src/lib.rs",
        find: "        let Some(span) = pitch.checked_mul(height) else {",
        replace: "        let Some(span) = row_bytes.checked_mul(height) else {",
        breaks: "a padded framebuffer mapping omits every row's padding",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::a_padded_framebuffer_spans_its_pitch",
        },
    },
    Mutation {
        file: "libs/ps2-keys/src/decode.rs",
        find: "            Set::One => protocol::is_reply(byte) && byte != protocol::SELF_TEST_PASSED,",
        replace: "            Set::One => protocol::is_reply(byte),",
        breaks: "set one swallows a left-shift release as a controller reply",
        reddens: Caught::HostTest {
            package: "ps2-keys",
            name: "decode::tests::set_1_reads_aa_as_a_shift_release_not_a_self_test_result",
        },
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "        if self.reserved + self.completion.occupied() >= self.completion.capacity().entries() {",
        replace: "        if self.reserved >= self.completion.capacity().entries() {",
        breaks: "ring admission ignores completions already occupying their queue",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::the_reservation_invariant_holds_under_interleaving",
        },
    },
    Mutation {
        file: "libs/slab-layout/src/lib.rs",
        find: "        Ok(None) if size == 0 => Ok(Placement::Frames {\n            count: 1,\n            align: align.div_ceil(FRAME_SIZE),\n        }),",
        replace: "        Ok(None) if false && size == 0 => Ok(Placement::Frames {\n            count: 1,\n            align: align.div_ceil(FRAME_SIZE),\n        }),",
        breaks: "an over-aligned zero-size allocation asks for zero frames",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "tests::a_zero_sized_request_too_aligned_for_a_class_still_gets_a_frame",
        },
    },
    Mutation {
        file: "libs/stack-layout/src/lib.rs",
        find: "            stack_base: guard_base + self.guard_len,",
        replace: "            stack_base: guard_base,",
        breaks: "a stack begins on the page reserved as its guard",
        reddens: Caught::HostTest {
            package: "stack-layout",
            name: "tests::every_stack_has_its_guard_immediately_below",
        },
    },
    Mutation {
        file: "libs/timebase/src/lib.rs",
        find: "        let carried = advanced - (whole << self.shift);",
        replace: "        let carried = 0;",
        breaks: "each clock update discards its fractional nanosecond carry",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "published_tests::a_thousand_updates_do_not_drift_from_one_conversion",
        },
    },
    Mutation {
        file: "libs/trap-cause/src/lib.rs",
        find: "        self.0 & (1 << 1) != 0",
        replace: "        self.0 & (1 << 2) != 0",
        breaks: "a page fault reports user access as a write",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::every_page_fault_bit_is_read_on_its_own",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "pub const fn has_permitted_components(available: u64) -> bool {\n    available & PERMITTED == PERMITTED\n}",
        replace: "pub const fn has_permitted_components(available: u64) -> bool {\n    available & PERMITTED != 0\n}",
        breaks: "a partial x87, SSE or AVX mask can satisfy the fixed state policy",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::every_permitted_component_must_be_available",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "    if !component.is_present() || offset < HEADER_BYTES as u32 {",
        replace: "    if !component.is_present() || offset > HEADER_BYTES as u32 {",
        breaks: "a standard XSAVE component can overlap the legacy area",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::a_standard_component_extent_uses_the_reported_offset",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "        let mut complete = mask & AVX == 0 || mask & SSE != 0;",
        replace: "        let mut complete = true;",
        breaks: "AVX is enabled without its SSE dependency",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::avx_without_sse_is_not_consistent",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "    mask & AVX == 0 || basic_ecx & AVX_SUPPORTED != 0",
        replace: "    mask & AVX == 0 || basic_ecx & AVX_SUPPORTED == 0",
        breaks: "AVX state is enabled when CPUID does not advertise AVX instructions",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::avx_state_requires_avx_instruction_support",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "            if !component.is_present() {\n                complete = false;\n                break;\n            }",
        replace: "            if !component.is_present() {\n                continue;\n            }",
        breaks: "an enabled extended-state component has no CPUID sub-leaf",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::every_enabled_extended_component_must_be_present",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "            let Ok(narrowed) = u32::try_from(cursor) else {\n                complete = false;\n                break;\n            };",
        replace: "            let narrowed = cursor as u32;",
        breaks: "a compacted xstate offset truncates through the u32 boundary",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::a_layout_that_overflows_a_u32_offset_is_not_consistent",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/lib.rs",
        find: "        Self(self.0 & other.0)",
        replace: "        Self(self.0 | other.0)",
        breaks: "a page walk unions permissions that every level must grant",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::intersection_keeps_only_what_both_hold",
        },
    },
    Mutation {
        file: "libs/x86-pit/src/calibrate.rs",
        find: "        Some(product) => product.checked_add(denominator / 2),",
        replace: "        Some(product) => product.checked_add(denominator & 0),",
        breaks: "pit calibration floors instead of rounding to nearest",
        reddens: Caught::HostTest {
            package: "x86-pit",
            name: "calibrate::tests::rounding_is_to_nearest_not_floor",
        },
    },
    Mutation {
        file: "libs/x86-tables/src/idt.rs",
        find: "        gate.attributes |= 0b11 << 5;",
        replace: "        gate.attributes |= 0b11 << 4;",
        breaks: "a user-callable interrupt gate writes the wrong privilege bits",
        reddens: Caught::HostTest {
            package: "x86-tables",
            name: "idt::tests::a_user_callable_gate_differs_only_in_privilege",
        },
    },
    Mutation {
        file: "xtask/src/image/mod.rs",
        find: "            Self::SelfTest => &[paths::SELFTEST_FEATURE],",
        replace: "            Self::SelfTest => &[],",
        breaks: "the self-test image selector compiles the shipped kernel",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::selectors_build_only_their_named_kernel",
        },
    },
    Mutation {
        file: "xtask/src/image/mod.rs",
        find: "            .open_file(paths::KERNEL_ON_ESP)",
        replace: "            .open_file(paths::PROGRAM_ON_ESP)",
        breaks: "image verification reads another payload instead of the kernel",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::both_kernel_kinds_are_read_back_and_identified",
        },
    },
    Mutation {
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
        file: "libs/elf64/src/lib.rs",
        find: "            offset: read_u64(header, 40)?,",
        replace: "            offset: read_u64(header, 32)?,",
        breaks: "the section table offset is read from the program table field",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::the_section_table_fields_are_where_the_file_header_puts_them",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if inventory.is_reexport(file, at) {\n            return Ok(());\n        }",
        replace: "        if false && inventory.is_reexport(file, at) {\n            return Ok(());\n        }",
        breaks: "a public re-export is mistaken for an executable caller",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::reexports_docs_and_same_names_are_resolved_by_the_compiler",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    public_name();\n    api::nested::same_name();",
        replace: "    public_name();\n    api::documented();\n    api::nested::same_name();",
        breaks: "the fixture's documentation-only API secretly gains a code caller",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::reexports_docs_and_same_names_are_resolved_by_the_compiler",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    api::nested::same_name();",
        replace: "    api::same_name();",
        breaks: "the same-name fixture calls the other declaration",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::reexports_docs_and_same_names_are_resolved_by_the_compiler",
        },
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "        if count != 0 && !offset.is_multiple_of(width) {",
        replace: "        if !offset.is_multiple_of(width) {",
        breaks: "an empty mmio span acquires an alignment requirement despite accessing nothing",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "every_small_repeated_access_matches_an_independent_oracle",
        },
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "        self.next += self.width;",
        replace: "        self.next += 0;",
        breaks: "a checked MMIO span repeats its first offset instead of covering the range",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "every_small_repeated_access_matches_an_independent_oracle",
        },
    },
    Mutation {
        file: "libs/stack-layout/src/lib.rs",
        find: "    left < right_end && right < left_end",
        replace: "    left <= right_end && right < left_end",
        breaks: "a range ending beside a stack is mistaken for an overlap",
        reddens: Caught::HostTest {
            package: "stack-layout",
            name: "half_open_ranges_distinguish_crossing_adjacent_and_one_past",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if current_mapping_root(root).is_none() {",
        replace: "        if false && current_mapping_root(root).is_none() {",
        breaks: "a recycled inactive root can mint hardware stack authority",
        reddens: Caught::Check("the double-fault stack cannot be unmapped"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    pinned.iter().flatten().find_map(|identity| {",
        replace: "    pinned.iter().skip(1).flatten().find_map(|identity| {",
        breaks: "the first permanent stack can lose its mapping protection",
        reddens: Caught::Check("the double-fault stack cannot be unmapped"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    pinned.iter().flatten().find_map(|identity| {",
        replace: "    pinned.iter().take(1).flatten().find_map(|identity| {",
        breaks: "a later permanent stack can lose its mapping protection",
        reddens: Caught::Check("the boot thread stands on a guarded stack"),
    },
    Mutation {
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
        file: "wost/src/mmio.rs",
        find: "    pub(crate) const unsafe fn new(base: *mut u8, len: usize) -> Self {",
        replace: "    pub(crate) const fn new(base: *mut u8, len: usize) -> Self {",
        breaks: "the raw MMIO constructor loses its caller safety contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::every_unsafe_source_has_one_reviewed_fingerprint",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "    unsafe { core::arch::asm!(\"pushfq\", \"pop {}\", out(reg) flags, options(preserves_flags)) };",
        replace: "    unsafe { core::arch::asm!(\"pushfq\", \"pop {}\", out(reg) flags, options(nomem, preserves_flags)) };",
        breaks: "the flags probe claims no memory access while using a stack slot",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::every_unsafe_source_has_one_reviewed_fingerprint",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/segment.rs",
        find: "            \"ltr {selector:x}\",\n            selector = in(reg) TASK_STATE.bits(),\n            options(nostack, preserves_flags),",
        replace: "            \"ltr {selector:x}\",\n            selector = in(reg) TASK_STATE.bits(),\n            options(nomem, nostack, preserves_flags),",
        breaks: "the task-register load hides its GDT descriptor update from the compiler",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::every_unsafe_source_has_one_reviewed_fingerprint",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/segment.rs",
        find: "            \"mov ss, {selector:x}\",\n            \"mov ds, {selector:x}\",\n            \"mov es, {selector:x}\",\n            selector = in(reg) KERNEL_DATA.bits(),\n            options(nostack, preserves_flags),",
        replace: "            \"mov ss, {selector:x}\",\n            \"mov ds, {selector:x}\",\n            \"mov es, {selector:x}\",\n            selector = in(reg) KERNEL_DATA.bits(),\n            options(nomem, nostack, preserves_flags),",
        breaks: "segment reload hides its GDT descriptor reads from the compiler",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::every_unsafe_source_has_one_reviewed_fingerprint",
        },
    },
    Mutation {
        file: "README.md",
        find: "See [docs/002-architecture.md](docs/002-architecture.md).",
        replace: "See [docs/002-architecture.md](docs/002-architectur.md).",
        breaks: "a local Markdown link names a path the repository does not hold",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_local_markdown_link_and_anchor_resolves",
        },
    },
    Mutation {
        file: "AGENTS.md",
        find: "[Development](README.md#development)",
        replace: "[Development](README.md#developmnt)",
        breaks: "a local Markdown fragment names no heading in its target",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_local_markdown_link_and_anchor_resolves",
        },
    },
    Mutation {
        file: "AGENTS.md",
        find: "`kernel/src/selftest.rs` and runs under `cargo xtask test-boot`.",
        replace: "`kernel/src/selftest.rs` and runs under `cargo xtask test-bot`.",
        breaks: "an operational document offers an xtask command the parser rejects",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_documented_xtask_invocation_parses",
        },
    },
    Mutation {
        file: "docs/014-storage.md",
        find: "| Machine checks | 261 |",
        replace: "| Machine checks | 260 |",
        breaks: "the current documentation count disagrees with the machine oracle",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::the_phase_close_says_what_the_source_says",
        },
    },
    Mutation {
        file: "docs/000-vision.md",
        find: "| Boot | UEFI + Limine (vendored); an own bootloader is not planned without a concrete unmet handoff requirement (D-219) |",
        replace: "| Boot | UEFI + Limine (vendored); own bootloader is a future side-quest |",
        breaks: "the current roadmap reschedules the declined own-bootloader project",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::current_facing_foundation_prose_names_the_later_resolutions",
        },
    },
    Mutation {
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
        file: "xtask/src/mutate.rs",
        find: "            package: \"eevdf\",\n            name: \"tests::a_thread_past_its_deadline_has_no_slice_left\",",
        replace: "            package: \"xtask\",\n            name: \"tests::a_thread_past_its_deadline_has_no_slice_left\",",
        breaks: "a pure host crate can lose its only persistent source mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::every_host_library_has_a_persistent_source_mutation",
        },
    },
    Mutation {
        file: "xtask/src/cargo.rs",
        find: "    let mut args = vec![\"build\".to_string(), \"--locked\".to_string()];",
        replace: "    let mut args = vec![\"build\".to_string()];",
        breaks: "a release build may resolve dependencies outside Cargo.lock",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::kernel_builds_are_locked_to_the_recorded_dependencies",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "            self.fields.insert(key.clone(), value).is_none(),",
        replace: "            self.fields.insert(key.clone(), value).is_none() || true,",
        breaks: "a later evidence field silently replaces an earlier claim",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_manifest_rejects_duplicate_keys",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "        actual == *expected,",
        replace: "        actual != *expected,",
        breaks: "missing and unknown evidence fields are accepted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_manifest_schema_is_exact",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    first.bytes == second.bytes && first.sha256 == second.sha256",
        replace: "    first.bytes == second.bytes || first.sha256 == second.sha256",
        breaks: "equal-sized release artifacts are mistaken for equal bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::equal_lengths_do_not_make_artifacts_identical",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "            record.get(key)? == PASS,",
        replace: "            record.get(key)? == PASS || true,",
        breaks: "a failed qualification result verifies as a pass",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::every_recorded_result_must_pass",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    status.trim().is_empty()",
        replace: "    true",
        breaks: "modified and untracked source can claim a clean identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_dirty_source_cannot_be_called_clean",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "        for (key, value) in &self.fields {",
        replace: "        for (key, value) in self.fields.iter().rev() {",
        breaks: "the same evidence fields render in a different byte order",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::rendering_is_sorted_and_round_trips",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "            record.get(&format!(\"{}.sha256\", spec.prefix))? == digest.sha256,\n            \"{} has a different digest than its manifest\",\n            spec.prefix\n        );\n        digests.push(digest);",
        replace: "            record.get(&format!(\"{}.sha256\", spec.prefix))? == digest.sha256 || true,\n            \"{} has a different digest than its manifest\",\n            spec.prefix\n        );\n        digests.push(digest);",
        breaks: "bundled bytes can differ from the digest the manifest records",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::artifact_bytes_must_match_the_manifest",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "pub(crate) const MACHINE_TYPE: &str = \"pc-q35-9.2\";",
        replace: "pub(crate) const MACHINE_TYPE: &str = \"q35\";",
        breaks: "QEMU's moving q35 alias changes the qualified hardware model",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_machine_type_is_a_versioned_q35_not_an_alias",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    Ok(version.to_string())",
        replace: "    Ok(report.trim().to_string())",
        breaks: "vmrun usage text is recorded as part of its version identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmrun_version_is_separated_from_its_usage_text",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    record.insert(format!(\"{prefix}.version\"), version)?;",
        replace: "    record.insert(format!(\"{prefix}.reported-version\"), version)?;",
        breaks: "an unavailable optional tool changes the evidence schema",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::optional_tools_never_change_the_manifest_schema",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    for prefix in QUALIFICATION_TOOLS {",
        replace: "    for prefix in QUALIFICATION_TOOLS.iter().take(0) {",
        breaks: "an unavailable qualification dependency is reported as an unrelated byte mismatch",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::an_unavailable_qualification_dependency_is_named_exactly",
        },
    },
    Mutation {
        file: "supply-chain/inventory.txt",
        find: "crate|anyhow|1.0.104|development|330a5ed07fa54e4702c9d6c4174f74427fc0ef6e214bbd677ae50a5099946470|MIT|selected from MIT OR Apache-2.0",
        replace: "crate|anyhow|1.0.104|development|0000000000000000000000000000000000000000000000000000000000000000|MIT|selected from MIT OR Apache-2.0",
        breaks: "a reviewed dependency checksum can differ from Cargo.lock",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_locked_registry_package_has_one_exact_inventory_row",
        },
    },
    Mutation {
        file: "supply-chain/inventory.txt",
        find: "crate|noto-sans-mono-bitmap|0.3.2|runtime|f102c477fd4ec864c149803bf606ecc23bc9329602ec0f99073a728abf454abc|MIT AND OFL-1.1|MIT crate code and OFL-1.1 Noto glyph data",
        replace: "crate|noto-sans-mono-bitmap|0.3.2|runtime|f102c477fd4ec864c149803bf606ecc23bc9329602ec0f99073a728abf454abc|GPL-3.0-only|MIT crate code and OFL-1.1 Noto glyph data",
        breaks: "an unreviewed license conclusion enters the dependency record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_license_conclusion_is_from_the_reviewed_policy",
        },
    },
    Mutation {
        file: "supply-chain/inventory.txt",
        find: "artifact|boot/limine/BOOTX64.EFI|v10.x-binary-2026-07-18|runtime|91ef81c1f95c4ac8bf77c4393a0b2fcd2628550827d7abf89a43194400e17a85|BSD-2-Clause|Limine UEFI binary",
        replace: "artifact|boot/limine/BOOTX64.EFI|v10.x-binary-2026-07-18|runtime|0000000000000000000000000000000000000000000000000000000000000000|BSD-2-Clause|Limine UEFI binary",
        breaks: "a vendored runtime artifact can differ from the inventory digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_vendored_digest_is_in_the_inventory_and_matches_the_bytes",
        },
    },
    Mutation {
        file: "supply-chain/inventory.txt",
        find: "crate|bitflags|2.13.1|runtime|b588b76d00fde79687d7646a9b5bdf3cc0f655e0bbd080335a95d7e96f3587da|MIT|selected from MIT OR Apache-2.0",
        replace: "crate|bitflags|2.13.1|development|b588b76d00fde79687d7646a9b5bdf3cc0f655e0bbd080335a95d7e96f3587da|MIT|selected from MIT OR Apache-2.0",
        breaks: "a crate linked into the kernel is labelled development-only",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::runtime_scope_is_the_kernel_dependency_closure",
        },
    },
    Mutation {
        file: "supply-chain/foundation.spdx",
        find: "DocumentName: wizards-os-foundation",
        replace: "DocumentName: unrelated-foundation",
        breaks: "the committed SBOM differs from its deterministic source facts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::the_committed_spdx_is_the_deterministic_inventory_rendering",
        },
    },
    Mutation {
        file: "supply-chain/THIRD-PARTY-NOTICES.md",
        find: "SIL Open Font License 1.1 (`OFL-1.1`), copyright 2022 The Noto Project",
        replace: "SIL Open Font License 1.1 (`OFL 1.1`), copyright 2022 The Noto Project",
        breaks: "runtime notices omit the exact font license identifier",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_runtime_notice_and_required_license_is_present",
        },
    },
    Mutation {
        file: "xtask/src/supply_chain.rs",
        find: "        matches.len() == 1,",
        replace: "        !matches.is_empty(),",
        breaks: "an ambiguous locked dependency silently selects one version",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::a_dependency_with_two_locked_versions_must_name_which_one",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/shared_memory.rs",
        find: "        asm!(\n            \"mov dword ptr [{address}], {value:e}\",\n            address = in(reg) address,\n            value = in(reg) value,\n            options(nostack, preserves_flags),\n        );",
        replace: "        asm!(\n            \"mov dword ptr [{address}], {value:e}\",\n            address = in(reg) address,\n            value = in(reg) value,\n            options(nostack, preserves_flags, nomem),\n        );",
        breaks: "a published queue index can become visible before its payload",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::release_publication_excludes_an_old_payload",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/shared_memory.rs",
        find: "        asm!(\n            \"mov {value:e}, dword ptr [{address}]\",\n            address = in(reg) address,\n            value = out(reg) value,\n            options(nostack, preserves_flags),\n        );",
        replace: "        asm!(\n            \"mov {value:e}, dword ptr [{address}]\",\n            address = in(reg) address,\n            value = out(reg) value,\n            options(nostack, preserves_flags, nomem),\n        );",
        breaks: "a shared payload read can move before its published index",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::acquire_consumption_excludes_an_early_payload_read",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        let value = unsafe { shared_memory::load_acquire_u32(address) };",
        replace: "        let value = unsafe { (address as *const u32).read_volatile() };",
        breaks: "a hostile ring-3 writer races a non-atomic volatile publication load",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        unsafe { shared_memory::store_release_u32(address, value.to_le()) };",
        replace: "        unsafe { (address as *mut u32).write_volatile(value.to_le()) };",
        breaks: "a hostile ring-3 writer races a non-atomic volatile publication store",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        unsafe { destination.write_volatile(byte) };",
        replace: "        unsafe { destination.write(byte) };",
        breaks: "the compiler can merge or elide shared payload writes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::every_shared_payload_copy_is_volatile",
        },
    },
    Mutation {
        file: "xtask/src/image/program.rs",
        find: "    const COMPLETION_TAIL_AT: u8 = 28;",
        replace: "    const COMPLETION_TAIL_AT: u8 = 24;",
        breaks: "the real ring client acquires its own head instead of the completion tail",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::program::tests::the_ring_client_orders_shared_memory",
        },
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        let tail = ring.protocol.completions().tail();\n\n        let at = COMPLETIONS_AT + slot * COMPLETION_BYTES;",
        replace: "        let tail = ring.protocol.completions().tail();\n        drop(ring);\n        let mut ring = self.ring.lock_leaf();\n\n        let at = COMPLETIONS_AT + slot * COMPLETION_BYTES;",
        breaks: "a later kernel completion can publish past an unwritten earlier slot",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::kernel_completion_producers_publish_in_tail_order",
        },
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        if entry.owner != self.id {\n            return Err(HandleError::Stale);\n        }",
        replace: "",
        breaks: "a resolved handle cannot cross its ownership table",
        reddens: Caught::Check("a handle reveals no foreign occupancy"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "pub struct InterruptState {
    owner: usize,
    level: usize,
    live: bool,
    stays_here: PhantomData<*mut ()>,
}",
        replace: "pub struct InterruptState {
    owner: usize,
    level: usize,
    live: bool,
    stays_here: PhantomData<()>,
}",
        breaks: "a spin-lock guard cannot restore another CPU's interrupt flag",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::guard_types_do_not_cross_cpus",
        },
    },
    Mutation {
        file: "wost/src/sync.rs",
        find: "pub struct ThreadLockGuard<'a, T> {
    lock: &'a ThreadLock<T>,
    stays_here: PhantomData<*const ()>,
}",
        replace: "pub struct ThreadLockGuard<'a, T> {
    lock: &'a ThreadLock<T>,
    stays_here: PhantomData<()>,
}",
        breaks: "a thread-lock guard cannot be released on another CPU",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::guard_types_do_not_cross_cpus",
        },
    },
    Mutation {
        file: "wost/src/sync.rs",
        find: "impl<T> Drop for SpinLockGuard<'_, T> {",
        replace: "#[cfg(any())]\nimpl<T> Drop for SpinLockGuard<'_, T> {",
        breaks: "a live spin guard keeps its thread-context borrow until Drop",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::a_live_spin_guard_keeps_the_thread_context_borrowed",
        },
    },
    Mutation {
        file: "xtask/src/contracts.rs",
        find: "    DirectDoor {\n        call: \"RingEnter\",\n        ownership: WaitOwnership::CannotWait,\n    },\n",
        replace: "",
        breaks: "a direct user door has no wait-ownership classification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::every_direct_door_has_an_explicit_wait_owner",
        },
    },
    Mutation {
        file: "libs/memory-map/src/lib.rs",
        find: "            if len == MAX_REGIONS {
                return Err(MemoryMapError::TooMany {
                    observed: MAX_REGIONS + 1,
                    capacity: MAX_REGIONS,
                });
            }",
        replace: "            if len == MAX_REGIONS {
                break;
            }",
        breaks: "an oversized boot map cannot become an accepted prefix",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::the_region_limit_is_all_or_nothing",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "                let product = (crystal_hz as u64).checked_mul(numerator as u64);",
        replace: "                let product = (crystal_hz as u64).checked_mul(denominator as u64);",
        breaks: "the CPUID TSC ratio cannot exchange its numerator and denominator",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::frequency_leaves_keep_exact_and_nominal_evidence_distinct",
        },
    },
    Mutation {
        file: "libs/x86-pit/src/calibrate.rs",
        find: "    if observed_hz.abs_diff(expected_tsc_hz) <= expected_tsc_hz / AGREEMENT_DIVISOR {",
        replace: "    if observed_hz.abs_diff(observed_hz) <= expected_tsc_hz / AGREEMENT_DIVISOR {",
        breaks: "a stopped TSC cannot agree with an advancing PIT",
        reddens: Caught::HostTest {
            package: "x86-pit",
            name: "calibrate::tests::the_watchdog_distinguishes_disagreement_from_delay",
        },
    },
    Mutation {
        file: "libs/x86-pit/src/calibrate.rs",
        find: "    if apic_remaining != 0 || expected_pit_ticks == 0 || expected_tsc_hz == 0 {",
        replace: "    if false && apic_remaining != 0 || expected_pit_ticks == 0 || expected_tsc_hz == 0 {",
        breaks: "an older pending APIC interrupt cannot judge a newly armed timer",
        reddens: Caught::HostTest {
            package: "x86-pit",
            name: "calibrate::tests::the_watchdog_distinguishes_disagreement_from_delay",
        },
    },
    Mutation {
        file: "kernel/src/time.rs",
        find: "            if failures >= WATCHDOG_FAILURES {",
        replace: "            if failures > WATCHDOG_FAILURES {",
        breaks: "the required watchdog failures revoke the live timing source",
        reddens: Caught::Check(
            "a clock disagreement revokes time without stranding timers",
        ),
    },
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    DESIRED_MODE.store(mode, Ordering::Release);",
        replace: "    DESIRED_MODE.store(MODE_COUNTER, Ordering::Release);",
        breaks: "clock revocation survives the time page's temporary mapping absence",
        reddens: Caught::Check("revoking the clock sends readers to ring Uptime"),
    },
    Mutation {
        file: "kernel/src/time.rs",
        find: "        Some(now.saturating_add(elapsed))",
        replace: "        Some(now.saturating_add(elapsed.saturating_sub(elapsed)))",
        breaks: "the retained PIT advances scheduler time after public-clock revocation",
        reddens: Caught::Check(
            "a clock disagreement revokes time without stranding timers",
        ),
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "pub const FORMAT: &str = \"wizard-wizzy-handoff-v1\";",
        replace: "pub const FORMAT: &str = \"wizard-wizzy-handoff-v2\";",
        breaks: "the pinned handoff example uses the one supported envelope version",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::a_canonical_record_is_accepted",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    if core::iter::Iterator::next(&mut lines) != Some(FORMAT) {",
        replace: "    if core::iter::Iterator::next(&mut lines).is_none() {",
        breaks: "an unknown handoff envelope version cannot be parsed as version one",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::an_unknown_envelope_version_is_refused",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    extra.is_none()",
        replace: "    extra.is_none() || true",
        breaks: "an unknown handoff field cannot be silently ignored",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::unknown_or_noncanonical_fields_are_refused",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    first.is_ascii_lowercase()",
        replace: "    true",
        breaks: "handoff values cannot use ambiguous encodings or ambient provenance",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::decimals_tokens_and_revisions_enforce_each_canonical_boundary",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    record.target_id == TARGET_ID",
        replace: "    true || record.target_id == TARGET_ID",
        breaks: "every handoff target and ABI version must match exactly",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::every_version_must_match_exactly",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    record.artifact_bytes == artifact.bytes && record.artifact_sha256 == artifact.sha256",
        replace: "    record.artifact_bytes == artifact.bytes || record.artifact_sha256 == artifact.sha256",
        breaks: "artifact length and digest must both match the consumed bytes",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::artifact_length_and_digest_identify_the_same_bytes",
        },
    },
    Mutation {
        file: "handoff/conformance-v1.artifact",
        find: "This payload is opaque to the handoff envelope.",
        replace: "This payload is altered for the handoff envelope.",
        breaks: "the committed handoff payload cannot differ from its pinned digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "handoff::the_committed_handoff_fixture_is_self_contained",
        },
    },
    Mutation {
        file: "userland/abi/wizard_abi.h",
        find: "#define WIZ_TIME_VERSION 1u",
        replace: "#define WIZ_TIME_VERSION 2u",
        breaks: "platform ABI v1 cannot silently contain a version-two wire region",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "handoff::platform_v1_is_entry_ring_and_time_v1",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    let escaped = path.to_string_lossy().replace(',', \",,\");",
        replace: "    let escaped = path.to_string_lossy().into_owned();",
        breaks: "a comma in a disk path remains QEMU drive syntax",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::drive_paths_escape_commas_without_quoting_spaces",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    drive_file(&format!(\"{options},snapshot=on\"), path)",
        replace: "    drive_file(&format!(\"{options},snapshot=off\"), path)",
        breaks: "two QEMU runs may write the same shared build media",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::shared_media_writes_use_a_private_snapshot",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        let monitor = control_listener(\"monitor\")?;",
        replace: "        let monitor = qmp.try_clone().context(\"cloning the QMP listener\")?;",
        breaks: "QMP and the human monitor contend for one control port",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::simultaneous_control_sets_reserve_four_distinct_ports",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "const MONITOR_CLIENT_SUFFIX: &str = \",server=off\";",
        replace: "const MONITOR_CLIENT_SUFFIX: &str = \",server=on\";",
        breaks: "QEMU tries to bind the monitor port already held by the harness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_monitor_dials_the_reserved_listener",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "            let sequence = NEXT_RUN.fetch_add(1, Ordering::Relaxed);",
        replace: "            let sequence = NEXT_RUN.load(Ordering::Relaxed);",
        breaks: "concurrent QEMU runs reuse one firmware-store name",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::simultaneous_runs_get_distinct_writable_firmware",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "                if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {",
        replace: "                if cancelled.is_some_and(|_| false) {",
        breaks: "a cancelled QEMU control accept waits until its old deadline",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_cancelled_control_wait_returns_without_a_connection",
        },
    },
    Mutation {
        file: "xtask/src/image/mod.rs",
        find: "            let sequence = NEXT_RUN_IMAGE.fetch_add(1, Ordering::Relaxed);",
        replace: "            let sequence = NEXT_RUN_IMAGE.load(Ordering::Relaxed);",
        breaks: "concurrent QEMU builds reserve one boot-image path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::two_qemu_images_are_private_and_removed",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        match file.try_lock() {",
        replace: "        match std::result::Result::<(), TryLockError>::Ok(()) {",
        breaks: "concurrent runners can rewrite the NVMe corpus together",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::storage_construction_is_serialized_across_runners",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "const OPERATION_MASK: u32 = u32::MAX;",
        replace: "const OPERATION_MASK: u32 = u32::MAX & !(xstate::X87 as u32);",
        breaks: "x87 state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "const OPERATION_MASK: u32 = u32::MAX;",
        replace: "const OPERATION_MASK: u32 = u32::MAX & !(xstate::SSE as u32);",
        breaks: "SSE state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "const OPERATION_MASK: u32 = u32::MAX;",
        replace: "const OPERATION_MASK: u32 = u32::MAX & !(xstate::AVX as u32);",
        breaks: "AVX state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "                self.reserved += 1;",
        replace: "                self.reserved += 0;",
        breaks: "an admitted operation has no completion reservation",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "scheduled_calls_preserve_the_production_reservation",
        },
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            if !entry.settling && wanted(*entry) {",
        replace: "            if wanted(*entry) {",
        breaks: "two terminal paths can claim one pending operation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::every_terminal_path_claims_one_settlement_until_publication",
        },
    },
    Mutation {
        file: "models/RingMemory.tla",
        find: "        \\/ BeginRetire(op)\n",
        replace: "",
        breaks: "retirement is absent from the terminal-ownership state graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::every_terminal_cause_is_a_reachable_model_action",
        },
    },
    Mutation {
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
            name: "model::tests::a_tlc_verdict_is_complete_unambiguous_and_exact",
        },
    },
    Mutation {
        file: "xtask/src/model.rs",
        find: "        !lines.iter().any(|line| is_runtime_failure(line)),",
        replace: "        true,",
        breaks: "a plausible TLC verdict can hide a Java throwable or fatal worker record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::a_tlc_verdict_is_complete_unambiguous_and_exact",
        },
    },
    Mutation {
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
        file: "kernel/src/user.rs",
        find: "    spawn(\"user-baseline\", 8, MEASURE)",
        replace: "    spawn(\"user-baseline\", 8, 0)",
        breaks: "the ring boundary baseline never enables its timestamp witness",
        reddens: Caught::Check("a ring enter round trip is measurable"),
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        self.read_blocks(0, blocks, into).ok()?;",
        replace: "        self.read_blocks(self.blocks, blocks, into).ok()?;",
        breaks: "the storage baseline never submits inside the namespace",
        reddens: Caught::Check("a full-buffer nvme read is measurable"),
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        if core::iter::Iterator::next(&mut fields).is_some() {\n            bail!(\"trailing fields in foundation baseline\");\n        }",
        replace: "        if false && core::iter::Iterator::next(&mut fields).is_some() {\n            bail!(\"trailing fields in foundation baseline\");\n        }",
        breaks: "a performance record can append unversioned fields",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::baseline_record_is_versioned_ordered_and_unique",
        },
    },
    Mutation {
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
        file: "xtask/src/image/program.rs",
        find: "    code.extend_from_slice(&[0x41, 0xB8, 1, 0, 0, 0]);",
        replace: "    code.extend_from_slice(&[0x41, 0xB8, 0, 0, 0, 0]);",
        breaks: "the measured ring behaviour never arms its timestamp path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_c_header_and_the_kernel_agree_about_the_abi",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    if port == 0 {",
        replace: "    if false && port == 0 {",
        breaks: "the debugger accepts QEMU's unallocated port-zero placeholder",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_debug_endpoint_is_allocated_and_loopback_only",
        },
    },
    Mutation {
        file: "xtask/src/debugger.rs",
        find: "    if identity != GDB_IDENTITY {",
        replace: "    if false && identity != GDB_IDENTITY {",
        breaks: "an unreviewed debugger identity becomes part of the supported setup",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "debugger::tests::only_the_reviewed_native_gdb_identity_is_accepted",
        },
    },
    Mutation {
        file: "xtask/src/debugger.rs",
        find: "        format!(\"hbreak {ENTRY_SYMBOL}\"),",
        replace: "        format!(\"break {ENTRY_SYMBOL}\"),",
        breaks: "the debugger tries to write a software breakpoint before the kernel is mapped",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "debugger::tests::the_debugger_commands_are_entry_first_and_the_proof_is_bounded",
        },
    },
    Mutation {
        file: "xtask/src/debugger.rs",
        find: "    if !frame.contains(\"wizard_kernel::kmain\") || !frame.contains(\"wost/src/boot/mod.rs\") {",
        replace: "    if !frame.contains(\"wizard_kernel::kmain\") && !frame.contains(\"wost/src/boot/mod.rs\") {",
        breaks: "a backtrace may omit either the entry symbol or its source location",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "debugger::tests::the_debugger_proof_requires_the_entry_marker_symbol_and_source",
        },
    },
    Mutation {
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
        file: "xtask/src/mutate.rs",
        find: "const MACHINE_OBSERVATIONS: usize = 2;\n\nfn qualify_machine_mutation",
        replace: "const MACHINE_OBSERVATIONS: usize = 1;\n\nfn qualify_machine_mutation",
        breaks: "one incidental machine failure can invalidate an otherwise exact mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::an_inconclusive_machine_observation_gets_one_bounded_retry",
        },
    },
    // The bytes on disk are pristine either way; only a rebuild of everything
    // that inlined them can tell the two apart.
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "    write_source(&destination, &bytes)?;\n    ensure!(",
        replace: "    std::fs::copy(&source, &destination)\n        .map(|_| ())\n        .with_context(|| format!(\"writing {}\", destination.display()))?;\n    ensure!(",
        breaks: "a restored mutation carries its snapshot timestamp backwards",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_restored_input_is_what_the_next_build_observes",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "        let bytes = read_bounded_bytes(&source, MAX_TRACKED_FILE_BYTES, \"tracked mutation file\")?;\n        write_source(&destination, &bytes)?;",
        replace: "        std::fs::copy(&source, &destination)\n            .map(|_| ())\n            .with_context(|| format!(\"copying {}\", source.display()))?;",
        breaks: "a re-copied workspace carries its snapshot timestamps backwards",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_recopied_workspace_is_what_the_next_build_observes",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "            failed: observation.failed,",
        replace: "            failed: Vec::new(),",
        breaks: "a terminal observation discards exact checks that failed before its panic",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_panic_outcome_preserves_earlier_failures_and_its_exact_transcript",
        },
    },
    Mutation {
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
        file: "xtask/src/vmware.rs",
        find: "        Kernel::Shipped => paths.vm_dir().join(\"shipped\"),",
        replace: "        Kernel::Shipped => paths.vm_dir(),",
        breaks: "the shipped VMware round overwrites the self-test machine state",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::selftest_and_shipped_vms_do_not_share_machine_state",
        },
    },
    // The property tier's own falsifiability (D-242): the defect must
    // survive every generated case, not one a unit test names.
    Mutation {
        file: "libs/path-walk/src/lib.rs",
        find: "        if component == b\".\" {",
        replace: "        if false && component == b\".\" {",
        breaks: "a bare dot is a no-op rather than a component the walker descends into",
        reddens: Caught::HostTest {
            package: "path-walk",
            name: "no_split_output_can_name_an_ancestor",
        },
    },
    Mutation {
        file: "libs/stack-layout/src/lib.rs",
        find: "    if left_len == 0 || right_len == 0 {",
        replace: "    if left_len == 0 {",
        breaks: "an empty range on either side touches nothing, not merely the left",
        reddens: Caught::HostTest {
            package: "stack-layout",
            name: "an_empty_range_touches_nothing",
        },
    },
    Mutation {
        file: "libs/frame-space/src/lib.rs",
        find: "        if !self.is_managed(frame) {\n            return Err(FreeError::NotManaged(frame));\n        }",
        replace: "        if !self.is_managed(frame) && frame >= self.managed_count {\n            return Err(FreeError::NotManaged(frame));\n        }",
        breaks: "an in-range hole is authorized by the managed-frame cardinality",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "release_authority_distinguishes_managed_in_range_unmanaged_and_out_of_range",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "            && address.checked_add(len).is_some()",
        replace: "            && true",
        breaks: "a command buffer's contiguous physical extent can wrap the address space",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::identify_binds_one_complete_aligned_page",
        },
    },
    Mutation {
        file: "docs/016-pre-wizzy-foundation.md",
        find: "| D-121 | TRIGGER | PW-707 fires at the fourth leaf-lock static or a public callback-shaped leaf guard. |",
        replace: "| D-121 | TRIGGER | PW-999 fires at the fourth leaf-lock static or a public callback-shaped leaf guard. |",
        breaks: "a historical handoff can cite a register owner that does not exist",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_historical_handoff_has_one_current_disposition",
        },
    },
    Mutation {
        file: "libs/timebase/src/lib.rs",
        find: "        if multiplier > MAX_MULTIPLIER {\n            return None;\n        }",
        replace: "",
        breaks: "raw timebase construction publishes a multiplier wider than its wire field",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "tests::raw_parts_refuse_a_multiplier_that_cannot_be_published",
        },
    },
    Mutation {
        file: "libs/timebase/src/lib.rs",
        find: "        if shift > MAX_SHIFT {\n            return None;\n        }",
        replace: "",
        breaks: "raw timebase construction accepts an unevaluable shift",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "tests::raw_parts_refuse_a_shift_that_cannot_be_evaluated",
        },
    },
    Mutation {
        file: "libs/path-walk/src/lib.rs",
        find: "#[cfg(test)]\nmod tests",
        replace: "/// Whether `path` begins as a relative path.\npub const fn is_relative(path: &[u8]) -> bool {\n    !path.is_empty() && path[0] != SEPARATOR\n}\n\n#[cfg(test)]\nmod tests",
        breaks: "a weaker duplicate path predicate re-enters the public compiler surface",
        reddens: Caught::ApiAudit("fn is_relative"),
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    if manifest.contains('\\r') || !manifest.ends_with('\\n') {",
        replace: "    if manifest.contains('\\r') {",
        breaks: "a Wizzy handoff without its canonical terminal line feed is accepted",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::a_canonical_record_has_exactly_one_terminal_lf",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/lib.rs",
        find: "            (Self::OWNED, \"owned\"),",
        replace: "",
        breaks: "page-table diagnostics omit software ownership authority",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::debug_says_what_the_entry_is",
        },
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "                self.reserved -= 1;",
        replace: "                self.reserved -= 0;",
        breaks: "a reported operation keeps its completion reservation",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "scheduled_calls_preserve_the_production_reservation",
        },
    },
    Mutation {
        file: "libs/trap-cause/src/lib.rs",
        find: "}\n\n/// An architectural exception.",
        replace: "    /// Whether the interrupted code can be resumed.\n    pub const fn is_resumable(self) -> bool {\n        matches!(self, Self::Fault(Fault::Breakpoint))\n    }\n}\n\n/// An architectural exception.",
        breaks: "a speculative trap-resumption policy re-enters the public kernel boundary",
        reddens: Caught::ApiAudit("method impl::is_resumable"),
    },
    Mutation {
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    fn require_config(&mut self, value: u8) -> Result<(), I8042Error> {\n        self.write_config(value)\n            .then_some(())\n            .ok_or(I8042Error::ConfigurationRefused)\n    }",
        replace: "    fn require_config(&mut self, value: u8) -> Result<(), I8042Error> {\n        let _ = self.write_config(value);\n        Ok(())\n    }",
        breaks: "a required controller configuration refusal is silently accepted",
        reddens: Caught::Check(
            "a refused controller configuration is typed and changes nothing",
        ),
    },
    Mutation {
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    pub(crate) fn enable_interrupt(&mut self) -> Result<(), InterruptEnableError> {\n        if !Self::flush(self) {\n            return Err(InterruptEnableError::Unavailable);\n        }",
        replace: "    pub(crate) fn enable_interrupt(&mut self) -> Result<(), InterruptEnableError> {\n        let _ = Self::flush(self);",
        breaks: "an i8042 transaction proceeds through stale undrained output",
        reddens: Caught::Check(
            "a failed controller flush stops every following transaction",
        ),
    },
    Mutation {
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    fn controller_query(&mut self, command: u8) -> Option<u8> {\n        if !self.controller_command(command) {\n            return None;\n        }\n        self.read_reply(RESPONSE_ATTEMPTS)\n    }",
        replace: "    fn controller_query(&mut self, command: u8) -> Option<u8> {\n        let _ = self.controller_command(command);\n        self.read_reply(RESPONSE_ATTEMPTS)\n    }",
        breaks: "a refused i8042 command consumes an unrelated reply byte",
        reddens: Caught::Check(
            "a refused controller command is typed and consumes no reply",
        ),
    },
    Mutation {
        file: "wost/src/boot/limine_glue.rs",
        find: "    let region = unsafe { MmioRegion::new(fb.addr(), layout.span()) };",
        replace: "    let region = unsafe { MmioRegion::new(fb.addr(), layout.span() - 4) };",
        breaks: "a validated framebuffer lease omits its terminal word",
        reddens: Caught::Panic {
            marker: "validated framebuffer access left its mapping",
            why: "the framebuffer is the boot console and a contradictory validated MMIO lease must stop before its shadow can claim an unwritten device word",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "#[cfg(feature = \"refuse-pci\")]\nfn refuse_pci_for_degraded_boot() {\n    assert!(\n        wost::arch::current::port::claim::<u32>(0x0cf8).is_some(),\n        \"the degraded boot could not reserve the PCI configuration ports\"\n    );\n}",
        replace: "#[cfg(feature = \"refuse-pci\")]\nfn refuse_pci_for_degraded_boot() {}",
        breaks: "the degraded image silently retains live PCI configuration ports",
        reddens: Caught::BootMarker(
            "pci: the configuration ports are already claimed; no bus",
        ),
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            assert!(posted, \"a reserved completion slot was not there\");",
        replace: "            debug_assert!(posted, \"a reserved completion slot was not there\");",
        breaks: "a missing reserved completion wedges only release kernels",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "cpu: extended state unavailable; ring 3 stays closed",
        replace: "cpu: extended state unavailable",
        breaks: "the degraded CPU no longer proves the extended-state veto it exercised",
        reddens: Caught::BootMarker(qemu::DEGRADED_XSTATE_MARKER),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "maskable interrupts are not admitted; ring 3 is closed",
        replace: "interrupt admission unavailable; ring 3 is closed",
        breaks: "the degraded boot loses the exact first user-boundary refusal",
        reddens: Caught::BootMarker(qemu::DEGRADED_USER_MARKER),
    },
    Mutation {
        file: "kernel/src/time.rs",
        find: "        Err(apic::TimerError::Unavailable)\n            if !Timebase::from_bits(NANOSECONDS_TO_APIC.load(Ordering::Relaxed))\n                .is_calibrated() => {}\n",
        replace: "",
        breaks: "a boot with no admitted APIC timer panics while cleaning up idle scheduler state",
        reddens: Caught::DegradedPanic {
            marker: "the local timer would not stop: the local apic timer is unavailable",
            why: "the no-x2APIC boot reaches scheduler cleanup before any local timer can be admitted, so this exact terminal refusal is the defect",
        },
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        assert!(committed, \"a live reservation refused quota commit\");",
        replace: "        debug_assert!(committed, \"a live reservation refused quota commit\");",
        breaks: "a broken handle commitment is terminal only in debug kernels",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    assert!(index < ENTRIES, \"a level's index is always below 512\");",
        replace: "    debug_assert!(index < ENTRIES, \"a level's index is always below 512\");",
        breaks: "release page-table slot formation loses its index bound",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/segment.rs",
        find: "pub(super) fn write_kernel_stack(top: KernelStackTop<'_>) {\n    let top = top.address();\n    let at = TaskState::privilege_stack_offset(0);\n    assert!(",
        replace: "pub(super) fn write_kernel_stack(top: KernelStackTop<'_>) {\n    let top = top.address();\n    let at = TaskState::privilege_stack_offset(0);\n    debug_assert!(",
        breaks: "release TSS stack writes lose their alignment precondition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    assert!(answered, \"the live syscall frame could not be answered\");",
        replace: "    debug_assert!(answered, \"the live syscall frame could not be answered\");",
        breaks: "a release syscall can return with an unanswered stale register frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "            self.stop_dma(Some(data));",
        replace: "            let _ = self.fail_submitted(BringUpError::NotReady, Some(data));",
        breaks: "NVMe self-test repair hides cleanup behind a guaranteed error",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::known_fallible_results_are_not_discarded",
        },
    },
    Mutation {
        file: "libs/eevdf/src/lib.rs",
        find: "            Some((_, deadline)) if !before(entity.deadline, deadline) => {}",
        replace: "            Some((_, _deadline)) if false => {}",
        breaks: "a later eligible entity replaces the earliest deadline",
        reddens: Caught::HostTest {
            package: "eevdf",
            name: "tests::a_later_candidate_cannot_replace_an_earlier_deadline",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "            Self::NotAnElf => core::fmt::Write::write_str(f, \"does not begin with the elf magic\"),",
        replace: "            Self::NotAnElf => core::fmt::Write::write_str(f, \"\"),",
        breaks: "an ELF magic refusal loses its diagnostic reason",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::diagnostic_messages_keep_their_reason_and_values",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "    pub const fn flags(self) -> u32 {\n        self.flags\n    }",
        replace: "    pub const fn flags(self) -> u32 {\n        0\n    }",
        breaks: "ELF segment metadata erases every permission bit",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "    /// Where in the file its contents start.\n    pub const fn offset(self) -> u64 {\n        self.offset\n    }",
        replace: "    /// Where in the file its contents start.\n    pub const fn offset(self) -> u64 {\n        0\n    }",
        breaks: "ELF segment metadata erases the file offset",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "        self.flags & Self::READ != 0",
        replace: "        self.flags | Self::READ != 0",
        breaks: "every ELF segment is reported readable",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "        self.file_size > self.memory_size",
        replace: "        self.file_size >= self.memory_size",
        breaks: "an ELF segment whose contents exactly fit is rejected as oversized",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "    pub const fn segment_count(&self) -> u16 {\n        self.program_count\n    }",
        replace: "    pub const fn segment_count(&self) -> u16 {\n        1\n    }",
        breaks: "every ELF image reports one program segment",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        file: "libs/pixel/src/lib.rs",
        find: "            Self::Red => core::fmt::Write::write_str(f, \"red\"),",
        replace: "            Self::Red => core::fmt::Write::write_str(f, \"\"),",
        breaks: "pixel diagnostics omit the red channel name",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::diagnostic_messages_keep_their_channel_geometry_and_values",
        },
    },
    Mutation {
        file: "libs/pixel/src/lib.rs",
        find: "            Self::EmptyChannel(channel) => write!(f, \"{channel} channel has no bits\"),",
        replace: "            Self::EmptyChannel(_) => f.write_str(\"\"),",
        breaks: "an empty pixel channel loses its diagnostic reason",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::diagnostic_messages_keep_their_channel_geometry_and_values",
        },
    },
    Mutation {
        file: "libs/pixel/src/lib.rs",
        find: "            Self::ZeroWidth => core::fmt::Write::write_str(f, \"framebuffer width is zero\"),",
        replace: "            Self::ZeroWidth => core::fmt::Write::write_str(f, \"\"),",
        breaks: "a zero-width framebuffer loses its diagnostic reason",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::diagnostic_messages_keep_their_channel_geometry_and_values",
        },
    },
    Mutation {
        file: "libs/pixel/src/lib.rs",
        find: "    let bits = if size == u32::BITS {\n        u32::MAX\n    } else {\n        (1 << size) - 1\n    };",
        replace: "    let bits = if size == u32::BITS {\n        u32::MAX\n    } else {\n        (1 << size) / 1\n    };",
        breaks: "a one-bit pixel-channel overlap disappears from its mask",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::a_one_bit_channel_overlap_is_still_an_overlap",
        },
    },
    Mutation {
        file: "libs/port-space/src/lib.rs",
        find: "    left.start < right.end && right.start < left.end",
        replace: "    left.start < right.end && right.start <= left.end",
        breaks: "a port span touching above another is mistaken for overlap",
        reddens: Caught::HostTest {
            package: "port-space",
            name: "tests::a_request_that_overlaps_itself_is_refused",
        },
    },
    Mutation {
        file: "libs/slab-layout/src/lib.rs",
        find: "                write!(f, \"alignment {align} is not a non-zero power of two\")",
        replace: "                f.write_str(\"\")",
        breaks: "a slab placement refusal loses the invalid alignment",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "tests::placement_errors_name_the_invalid_alignment",
        },
    },
    Mutation {
        file: "libs/slab-layout/src/lib.rs",
        find: "            needed.next_power_of_two()",
        replace: "            needed.next_power_of_two() / 2",
        breaks: "a slab request rounds into the smaller size class",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "tests::every_slab_size_takes_the_exact_smallest_class",
        },
    },
    Mutation {
        file: "libs/handle-space/src/lib.rs",
        find: "    pub const fn is_identity(self) -> bool {\n        self.0 == 0\n    }",
        replace: "    pub const fn is_identity(self) -> bool {\n        false\n    }",
        breaks: "handle mixer identity detection ignores the mixer bits",
        reddens: Caught::HostTest {
            package: "handle-space",
            name: "tests::identity_detection_reads_the_mixer_bits",
        },
    },
    Mutation {
        file: "libs/handle-space/src/lib.rs",
        find: "    pub const fn union(self, other: Self) -> Self {\n        Self(self.0 | other.0)\n    }",
        replace: "    pub const fn union(self, other: Self) -> Self {\n        Self(self.0 ^ other.0)\n    }",
        breaks: "unioning a held handle right removes it",
        reddens: Caught::HostTest {
            package: "handle-space",
            name: "rights_tests::attenuation_only_narrows",
        },
    },
    Mutation {
        file: "libs/memory-map/src/lib.rs",
        find: "                region.len != 0\n                    && region.base < end\n                    && start < region.end()\n                    && !allowed(region.kind)",
        replace: "                region.len != 0\n                    && region.base <= end\n                    && start < region.end()\n                    && !allowed(region.kind)",
        breaks: "a disallowed region beginning at the exclusive end is mistaken for overlap",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_disallowed_region_at_the_exclusive_end_does_not_overlap",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "            Some((process, bound)) => {\n                prepared = Some(process);\n                Err(bound)\n            }",
        replace: "            Some((process, bound)) => {\n                core::mem::forget(process);\n                Err(bound)\n            }",
        breaks: "a commit refusal loses the still-bound process owner",
        reddens: Caught::Check("a commit refusal reunites cleanup authority"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "                self.undo(&range, offset, grain);\n                self.rollback_tables(table_mark);",
        replace: "                self.undo(&range, offset, grain);",
        breaks: "a failed mapping leaves its newly linked empty tables allocated",
        reddens: Caught::Check("a partial mapping failure returns its tables"),
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: "    paging::refuse_mapping_after_for_test(1);\n    let result = space.map_owned(BASE, run, flags);",
        replace: "    paging::refuse_mapping_after_for_test(1);\n    paging::refuse_mapping_undo_after_for_test(0);\n    let result = space.map_owned(BASE, run, flags);",
        breaks: "a rollback walk refusal returns ordinary failure and frees reachable frames",
        reddens: Caught::Panic {
            marker: "mapping rollback lost a written leaf: injected refusal",
            why: "an incomplete leaf rollback cannot return the still-addressable segment to the allocator, so the contradiction is terminal before ordinary failure escapes",
        },
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        let committed = self.table.step(Quota::commit);\n        assert!(committed, \"a live reservation refused quota commit\");\n        self.spent = true;",
        replace: "        self.spent = true;",
        breaks: "a kept handle discards its promised quota transition",
        reddens: Caught::Check("a completion carries a handle with narrowed rights"),
    },
    Mutation {
        file: "wost/src/mm/heap.rs",
        find: "        let Some(live) = self.live.checked_sub(1) else {",
        replace: "        let Some(live) = self.live.checked_sub(usize::MAX) else {",
        breaks: "a live slab slot is silently refused at deallocation",
        reddens: Caught::Panic {
            marker: "heap refused a live slot return",
            why: "valid GlobalAlloc provenance makes a slot-ledger refusal an internal contradiction, so deallocation must stop before ownership is forgotten",
        },
    },
    Mutation {
        file: "wost/src/mm/heap.rs",
        find: "                        heap.large_frames.checked_sub(count),",
        replace: "                        heap.large_frames.checked_sub(usize::MAX),",
        breaks: "a live large allocation is silently refused by frame accounting",
        reddens: Caught::Panic {
            marker: "heap refused a live frame return",
            why: "valid GlobalAlloc provenance proves the large-frame ledger contains this run, so refusal is terminal before the segment can be forgotten",
        },
    },
    Mutation {
        file: "wost/src/mm/heap.rs",
        find: "                let Some(physical) = physical_of(pointer) else {",
        replace: "                let Some(physical) = Option::<Paddr>::None else {",
        breaks: "a live direct-map allocation loses its physical provenance",
        reddens: Caught::Panic {
            marker: "heap could not recover a live allocation's physical address",
            why: "GlobalAlloc deallocation receives a pointer previously returned by this heap, so losing its direct-map provenance cannot be recovered as a leak",
        },
    },
    Mutation {
        file: "wost/src/boot/mod.rs",
        find: "        let words = self.words();",
        replace: "        let words = self.words() - 1;",
        breaks: "a nonempty framebuffer shadow omits its terminal device word",
        reddens: Caught::Panic {
            marker: "framebuffer shadow is shorter than its validated mapping",
            why: "the device write already succeeded, so silently omitting its shadow word would make later reads diverge from the displayed framebuffer",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "    pub const fn has_valid_alignment(self) -> bool {\n        self.alignment <= 1\n            || self.alignment.is_power_of_two()\n                && self.address % self.alignment == self.offset % self.alignment\n    }",
        replace: "    pub const fn has_valid_alignment(self) -> bool {\n        true\n    }",
        breaks: "ELF accepts non-power-of-two and file-incongruent segment alignment",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_alignment_is_power_of_two_and_congruent",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "        if !at.is_multiple_of(PAGE_SIZE) {\n            return Err(LoadError::Unaligned(at));\n        }",
        replace: "",
        breaks: "the loader reaches page mapping with an unaligned segment address",
        reddens: Caught::Check("the loader refuses what it may not map"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    bootstrap.abandoned = bootstrap\n        .abandoned\n        .checked_add(abandoned)\n        .unwrap_or_else(|| panic!(\"zombie abandonment accounting overflowed\"));",
        replace: "    bootstrap.abandoned = bootstrap.abandoned.saturating_add(abandoned);",
        breaks: "zombie abandonment accounting silently saturates",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::known_fallible_results_are_not_discarded",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            let Some((candidate_physical, flags)) = translate_in(candidate, address) else {\n                return Some(address);\n            };",
        replace: "            let Some((candidate_physical, flags)) = translate_in(candidate, address) else {\n                return Some(address);\n            };\n            let flags = flags.union(Flags::USER);",
        breaks: "activation accepts a user-accessible kernel-image leaf",
        reddens: Caught::Check("the loaded page table is ours"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            match walk(root, virtual_address) {",
        replace: "            match walk(root, virtual_address).map(|(mapped, flags, span)| (mapped, flags.union(Flags::NO_CACHE), span)) {",
        breaks: "activation accepts cache-policy drift in the direct map",
        reddens: Caught::Check("the loaded page table is ours"),
    },
    Mutation {
        file: "kernel/src/input.rs",
        find: "pub(crate) fn take_interrupts() {\n    assert!(\n        INTERRUPT_DRIVEN.claim(),\n        \"the keyboard interrupt handoff happened more than once\"\n    );\n}",
        replace: "pub(crate) fn take_interrupts() {}",
        breaks: "the routed keyboard never records its interrupt-driven handoff",
        reddens: Caught::Check("keystrokes arrive as interrupts"),
    },
    Mutation {
        file: "kernel/src/time.rs",
        find: "    assert!(\n        SCHEDULER_NOW_NS\n            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |now| {",
        replace: "    debug_assert!(\n        SCHEDULER_NOW_NS\n            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |now| {",
        breaks: "release scheduler-time accounting stops proving its unconditional update",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "        assert_eq!(\n            timepage::publish(mode, time::origin_ticks(), time::conversion()),",
        replace: "        debug_assert_eq!(\n            timepage::publish(mode, time::origin_ticks(), time::conversion()),",
        breaks: "release boot silently accepts refusal from an initialized time page",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "    #[must_use = \"a refused shared-memory write changed no bytes\"]\n    pub fn write_volatile_at",
        replace: "    pub fn write_volatile_at",
        breaks: "a shared-memory operation refusal can become silently discardable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::fallible_operation_bools_stay_must_use",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !crate::shared_kernel_mappings_are_safe() {",
        replace: "    if false && !crate::shared_kernel_mappings_are_safe() {",
        breaks: "ring three opens without a consumed shared-mapping isolation proof",
        reddens: Caught::Check(
            "shared kernel mappings without an isolation proof are refused",
        ),
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "        let leaf = cpuid(0x4000_0000, 0);",
        replace: "        let leaf = cpuid(0x4000_0001, 0);",
        breaks: "the running CPU's exact QEMU TCG identity is not consumed",
        reddens: Caught::BootMarker(
            "cpu: user isolation shared kernel mappings: qemu_tcg",
        ),
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "    if speculation.rdcl_no() {",
        replace: "    if false && speculation.rdcl_no() {",
        breaks: "RDCL_NO no longer admits the shared kernel mapping policy",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::shared_kernel_mappings_need_rdcl_no_or_exact_qemu_tcg",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "        ExecutionEnvironmentKind::QemuTcg => Some(SharedKernelMappingReason {\n            kind: SharedKernelMappingReasonKind::QemuTcg,\n        }),",
        replace: "        ExecutionEnvironmentKind::QemuTcg => None,",
        breaks: "the exact QEMU TCG exception no longer admits shared mappings",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::shared_kernel_mappings_need_rdcl_no_or_exact_qemu_tcg",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "        if leaf_1_ecx & HYPERVISOR_BIT == 0 {",
        replace: "        if false && leaf_1_ecx & HYPERVISOR_BIT == 0 {",
        breaks: "the hypervisor leaf is read without CPUID proving it exists",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::the_hypervisor_leaf_is_not_read_without_the_feature_bit",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "                u32::from_le_bytes(*b\"GTCG\"),",
        replace: "                u32::from_le_bytes(*b\"GTCF\"),",
        breaks: "a near-miss hypervisor identity is accepted as QEMU TCG",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::only_the_exact_qemu_tcg_base_leaf_is_accepted",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "            SharedKernelMappingReasonKind::QemuTcg => \"qemu_tcg\",",
        replace: "            SharedKernelMappingReasonKind::QemuTcg => \"qemu\",",
        breaks: "the consumed QEMU TCG policy marker loses its stable name",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::shared_mapping_reasons_have_stable_policy_names",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "const HYPERVISOR_BIT: u32 = 1 << 31;",
        replace: "const HYPERVISOR_BIT: u32 = 1 << 30;",
        breaks: "the hypervisor-present CPUID bit drifts from the architecture manual",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::the_speculation_constants_are_the_manual_s",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    \"-no-user-config\",\n    \"-accel\",",
        replace: "    \"-accel\",",
        breaks: "ambient QEMU configuration can replace the qualified machine profile",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::every_qemu_runner_pins_the_exact_tcg_profile",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    \"-accel\",\n    \"tcg\",\n    \"-machine\",",
        replace: "    \"-accel\",\n    \"whpx\",\n    \"-machine\",",
        breaks: "the qualified machine can run under an unqualified accelerator",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::every_qemu_runner_pins_the_exact_tcg_profile",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    \"-cpu\",\n    \"max,hypervisor=on,tcg-cpuid=on\",\n    \"-m\",",
        replace: "    \"-cpu\",\n    \"max\",\n    \"-m\",",
        breaks: "the shipped QEMU CPU omits the exact TCG identity witness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::every_qemu_runner_pins_the_exact_tcg_profile",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "const DEGRADED_CPU: [&str; 2] = [\n    \"-cpu\",\n    \"qemu64,-x2apic,-smep,-smap,-umip,hypervisor=on,tcg-cpuid=on\",\n];",
        replace: "const DEGRADED_CPU: [&str; 2] = [\n    \"-cpu\",\n    \"qemu64,-x2apic,-smep,-smap,-umip\",\n];",
        breaks: "the degraded QEMU CPU omits the exact TCG identity witness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::every_qemu_runner_pins_the_exact_tcg_profile",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    validate_run_arguments(extra)?;",
        replace: "    let _unchecked_extra = extra;",
        breaks: "interactive arguments can replace the fixed TCG profile",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::interactive_arguments_cannot_replace_the_tcg_profile",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "                | \"-cpu\"",
        replace: "",
        breaks: "interactive arguments can replace the qualified CPU",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::interactive_arguments_cannot_replace_the_tcg_profile",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "                | \"-global\"",
        replace: "",
        breaks: "interactive arguments can override a qualified QOM property",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::interactive_arguments_cannot_replace_the_tcg_profile",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "                | \"-readconfig\"",
        replace: "",
        breaks: "interactive arguments can import an unqualified QEMU configuration",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::interactive_arguments_cannot_replace_the_tcg_profile",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "const SHIPPED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(QEMU_TCG_ISOLATION_MARKER),\n    RequiredLogRecord::info(NVME_FALLBACK_MARKER),\n];",
        replace: "const SHIPPED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(NVME_FALLBACK_MARKER),\n];",
        breaks: "a shipped boot can pass without consuming the TCG isolation policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "fn is_exact_log_message(line: &str, severity: LogSeverity, expected: &str) -> bool {\n    log_message(line, severity) == Some(expected)\n}",
        replace: "fn is_exact_log_message(line: &str, severity: LogSeverity, expected: &str) -> bool {\n    log_message(line, severity).is_some_and(|message| message.contains(expected))\n}",
        breaks: "a forged substring can satisfy the consumed-policy marker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::required_boot_records_refuse_framing_and_message_near_misses",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    if verdict.status == VerdictStatus::Pass && !observation.qemu_tcg_isolation {",
        replace: "    if false && verdict.status == VerdictStatus::Pass && !observation.qemu_tcg_isolation {",
        breaks: "a passing self-test can omit the consumed TCG isolation policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_passing_selftest_requires_the_exact_consumed_tcg_policy",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        if observation.qemu_tcg_isolation {",
        replace: "        if false && observation.qemu_tcg_isolation {",
        breaks: "duplicate TCG isolation decisions can masquerade as one consumed policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_passing_selftest_requires_the_exact_consumed_tcg_policy",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "const VMWARE_ISOLATION_MARKER: &str = \"cpu: user isolation shared kernel mappings: rdcl_no\";",
        replace: "const VMWARE_ISOLATION_MARKER: &str = \"cpu: user isolation shared kernel mappings: qemu_tcg\";",
        breaks: "VMware qualification accepts the QEMU-only shared-mapping policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "            Self::Exact(expected) => message == expected,",
        replace: "            Self::Exact(expected) => message.contains(expected),",
        breaks: "a near-miss VMware isolation line satisfies the native policy proof",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "    TranscriptMarker::LocalApic,\n    TranscriptMarker::Exact(VMWARE_ISOLATION_MARKER),\n    TranscriptMarker::Exact(\"selftest: a thread crosses into ring three and calls back\"),",
        replace: "    TranscriptMarker::LocalApic,\n    TranscriptMarker::Exact(\"selftest: a thread crosses into ring three and calls back\"),",
        breaks: "the VMware self-test selector omits the consumed native isolation policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_vmware_selftest_transcript_is_complete_current_and_unambiguous",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "const SHIPPED_MARKERS: &[TranscriptMarker] = &[\n    TranscriptMarker::Exact(VMWARE_ISOLATION_MARKER),\n    TranscriptMarker::Nvme,",
        replace: "const SHIPPED_MARKERS: &[TranscriptMarker] = &[\n    TranscriptMarker::Nvme,",
        breaks: "the shipped VMware selector omits the consumed native isolation policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_shipped_vmware_transcript_proves_liveness_without_selftest_substitution",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            ensure!(\n                !contains_diagnostic_marker(&text).with_context(|| {\n                    format!(\n                        \"decoding literals while scanning {} for the reserved API marker\",\n                        relative.display()\n                    )\n                })?,\n                \"{} contains the API audit's reserved diagnostic marker\",\n                relative.display()\n            );",
        replace: "            let _unchecked_diagnostic_source = (&text, &relative);",
        breaks: "the API-audit implementation can author a compiler diagnostic identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::the_audit_implementation_cannot_author_a_diagnostic_id",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            ensure!(\n                instrumentation.ids.contains(&id),\n                \"an API diagnostic names item {id}, which was not instrumented in this compiler round\"\n            );",
        replace: "            let _unchecked_instrumentation = (&instrumentation.ids, id);",
        breaks: "a stale or authored diagnostic can root an item in another compiler round",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_diagnostic_id_must_be_instrumented_in_the_current_round",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "    let metadata = std::fs::symlink_metadata(child)\n        .with_context(|| format!(\"inspecting {}\", child.display()))?;\n    require_real_directory_metadata(child, &metadata, label)?;\n    require_canonical_child(parent, child, label)",
        replace: "    let metadata = std::fs::symlink_metadata(child)\n        .with_context(|| format!(\"inspecting {}\", child.display()))?;\n    let _unchecked_mutation_child = (metadata, label);\n    Ok(())",
        breaks: "a redirected mutation-state ancestor can reach recursive cleanup outside target",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::recursive_cleanup_revalidates_the_whole_mutation_state_ancestry",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    if count != 1 || summary != \"public API item(s) have no caller:\" {\n        return false;\n    }",
        replace: "    if count != 1 {\n        return false;\n    }",
        breaks: "API mutations accept an arbitrary summary as the structured dead-item verdict",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::api_audit_mutations_require_the_current_structured_dead_item_verdict",
        },
    },
    Mutation {
        file: "xtask/src/elf.rs",
        find: "    if stride != SECTION_HEADER_BYTES {",
        replace: "    if false && stride != SECTION_HEADER_BYTES {",
        breaks: "section headers can alias through a zero or non-ELF64 stride",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "elf::tests::a_section_header_stride_is_exactly_elf64s",
        },
    },
    Mutation {
        file: "libs/memory-map/src/lib.rs",
        find: "        matches!(self, Self::Usable)",
        replace: "        false",
        breaks: "usable memory loses its exact allocator policy",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::every_memory_kind_has_its_exact_policy_and_name",
        },
    },
    Mutation {
        file: "libs/memory-map/src/lib.rs",
        find: "                let overlaps = start < region.end() && region.base < end;",
        replace: "                let overlaps = start < region.end() && region.base <= end;",
        breaks: "a device extent overlaps a RAM region beginning at its exclusive end",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_disallowed_region_at_a_device_extents_exclusive_end_is_disjoint",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "        if u32::from(identification_version) != CURRENT_VERSION {",
        replace: "        if false && u32::from(identification_version) != CURRENT_VERSION {",
        breaks: "ELF accepts an invalid identification version",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "        if kind != EXECUTABLE {",
        replace: "        if false && kind != EXECUTABLE {",
        breaks: "the loader accepts a non-executable ELF object type",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "        if version != CURRENT_VERSION {",
        replace: "        if false && version != CURRENT_VERSION {",
        breaks: "ELF accepts an invalid file-header version",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "        if header_size as usize != HEADER_BYTES {",
        replace: "        if false && header_size as usize != HEADER_BYTES {",
        breaks: "ELF accepts a non-ELF64 file-header size",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    repository_authority(&location[..authority_end])",
        replace: "    true",
        breaks: "a hostless HTTPS repository satisfies compiler provenance",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::repository_provenance_requires_a_host_bearing_https_authority",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    repository_authority(&location[..authority_end])",
        replace: "    false",
        breaks: "every supported HTTPS repository authority is refused",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::repository_provenance_accepts_each_supported_host_form",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    if host.len() > 253 {",
        replace: "    if host.len() >= 253 {",
        breaks: "the maximum valid DNS host length is refused",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::repository_host_length_boundary_is_exact",
        },
    },
    Mutation {
        file: "xtask/src/bounded_process.rs",
        find: "        if exited.is_none() {\n            return Err(anyhow::anyhow!(\n                \"{error:#}; stopping {reason} {label}: {kill_error}\"\n            ));\n        }",
        replace: "        if false && exited.is_none() {\n            return Err(anyhow::anyhow!(\n                \"{error:#}; stopping {reason} {label}: {kill_error}\"\n            ));\n        }",
        breaks: "a live direct child after refused termination is reported as an ordinary timeout",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "bounded_process::tests::a_failed_timeout_kill_is_returned_before_a_blocking_wait",
        },
    },
    Mutation {
        file: "xtask/src/bounded_process.rs",
        find: "    } else if let Err(reap) = poll_terminated_child(&mut child, try_wait, POST_KILL_TIMEOUT, label)\n    {\n        return Err(anyhow::anyhow!(\"{error:#}; {reap:#}\"));\n    }",
        replace: "    } else {\n        let _unreaped_child = (&mut child, try_wait, label);\n    }",
        breaks: "successful direct-child termination returns without a bounded reap observation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "bounded_process::tests::a_successful_timeout_kill_is_reaped_without_a_blocking_wait",
        },
    },
    Mutation {
        file: "xtask/src/bounded_process.rs",
        find: "        ensure!(\n            bytes <= limit as u64,\n            \"bounded child output exceeded {limit} bytes\"\n        );",
        replace: "        let _unchecked_capture_size = (bytes, limit);",
        breaks: "a noisy tool can grow its regular-file capture past the polling resource cap",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "bounded_process::tests::capture_growth_is_refused_while_the_child_is_still_polled",
        },
    },
    Mutation {
        file: "xtask/src/debugger.rs",
        find: "    crate::bounded_process::output(command, timeout, \"GDB identity probe\")",
        replace: "    crate::bounded_process::output(command, timeout.saturating_mul(20), \"GDB identity probe\")",
        breaks: "the GDB identity probe outlives its named direct-child deadline",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "debugger::tests::the_gdb_identity_probe_has_a_direct_child_return_deadline",
        },
    },
    Mutation {
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
        file: "xtask/src/qualification.rs",
        find: "    let result = crate::bounded_process::output(command, timeout, &label)?;",
        replace: "    let result = crate::bounded_process::output(command, timeout.saturating_mul(20), &label)?;",
        breaks: "an exceptional qualification probe outlives its named direct-child deadline",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::an_external_qualification_probe_has_a_direct_child_return_deadline",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "        let path = require_confined_file(&self.snapshot, relative, \"mutation source file\")?;",
        replace: "        let path = checked_join(&self.snapshot, relative)?;",
        breaks: "a substituted snapshot parent can author the mutation input after session open",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::tracked_snapshot_and_workspace_redirects_are_refused_at_use",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "        let path =\n            require_confined_file(self.workspace.root(), relative, \"mutation workspace file\")?;",
        replace: "        let path = checked_join(self.workspace.root(), relative)?;",
        breaks: "a substituted workspace parent can redirect a mutation write after session open",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::tracked_snapshot_and_workspace_redirects_are_refused_at_use",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "    require_real_file_metadata(path, &metadata, label)?;\n    require_canonical_child(parent, path, label)?;",
        replace: "    let _unchecked_direct_file = (&metadata, label);",
        breaks: "a redirected tracked file can receive an isolated mutation outside the workspace",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::tracked_snapshot_and_workspace_redirects_are_refused_at_use",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "    fn require_directory(&self) -> Result<()> {\n        ensure!(\n            canonical_existing_real_directory(&self.directory, \"mutation journal directory\")?\n                == self.directory,\n            \"mutation journal directory changed after it was opened\"\n        );\n        Ok(())\n    }",
        replace: "    fn require_directory(&self) -> Result<()> { Ok(()) }",
        breaks: "a replaced journal directory can receive durable mutation state after store open",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::journal_paths_cannot_be_substituted_after_the_store_opens",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "            if require_direct_file_if_present(&self.directory, path, label)? {\n                std::fs::remove_file(path)\n                    .with_context(|| format!(\"removing old {label} {}\", path.display()))?;\n            }\n            std::fs::rename(&temporary, path).with_context(|| {\n                format!(\n                    \"publishing {label} {} as {}\",\n                    temporary.display(),\n                    path.display()\n                )\n            })?;",
        replace: "            let _unchecked_destination =\n                require_direct_file_if_present(&self.directory, path, label)?;\n            std::fs::write(path, bytes)\n                .with_context(|| format!(\"overwriting {label} {}\", path.display()))?;",
        breaks: "a hard-linked report target can be overwritten instead of atomically replaced",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::journal_paths_cannot_be_substituted_after_the_store_opens",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            prepare_contained_real_directory(\n                evidence_root,\n                parent,\n                \"qualification bundle directory\",\n            )?;",
        replace: "            std::fs::create_dir_all(parent)\n                .with_context(|| format!(\"creating {}\", parent.display()))?;\n            require_contained_real_directory(\n                evidence_root,\n                parent,\n                \"qualification bundle directory\",\n            )?;",
        breaks: "bundle preparation can create a directory through a redirected evidence parent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::bundle_directories_are_validated_before_any_redirected_creation",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            transcript.source.file_name().and_then(|name| name.to_str())\n                == Some(generation_transcript_name(generation)?.as_str()),",
        replace: "            true,",
        breaks: "a transcript path from another valid VMware generation enters the capture",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::one_immutable_vmware_generation_is_captured_before_publication",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            receipt == generation_record_from_digests(source, kind, generation, &digests)?,",
        replace: "            true,",
        breaks: "machine bytes from multiple VMware generations share one captured receipt",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::one_immutable_vmware_generation_is_captured_before_publication",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "                digest_file(&file.source)? == file.digest,",
        replace: "                true,",
        breaks: "a VMware source artifact can change after its immutable capture",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::one_immutable_vmware_generation_is_captured_before_publication",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            ensure!(\n                destination == file.captured && digest_file(&destination)? == file.digest,\n                \"captured VMware artifact changed during publication\"\n            );",
        replace: "            std::fs::copy(&file.source, &destination)\n                .with_context(|| format!(\"recopying {}\", file.source.display()))?;",
        breaks: "VMware publication reopens moving sources instead of using the immutable capture",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::one_immutable_vmware_generation_is_captured_before_publication",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/vector.rs",
        find: "        if vector < FIRST_PIC_VECTOR {",
        replace: "        if vector < FIRST_PIC_VECTOR && vector != 0x1f {",
        breaks: "external APIC vectors admit frame shapes owned by exception entry",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "vector::tests::every_cpu_vector_is_reserved",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/vector.rs",
        find: "            while other < index {",
        replace: "            while other < index.saturating_sub(1) {",
        breaks: "two APIC sources can be assigned one indistinguishable external vector",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "vector::tests::every_pair_is_accepted_exactly_when_assignable_and_distinct",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    identity == wanted\n        && location.rsplit_once(':').is_some_and(|(path, line)| {",
        replace: "    identity.contains(wanted)\n        && location.rsplit_once(':').is_some_and(|(path, line)| {",
        breaks: "an API mutation is credited by a longer dead-item name containing its target",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::api_audit_mutations_require_the_current_structured_dead_item_verdict",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    let output = run(command, QEMU_VERSION_DEADLINE, QEMU_VERSION_LABEL)?;",
        replace: "    let output = run(command, Duration::MAX, QEMU_VERSION_LABEL)?;",
        breaks: "the external QEMU identity probe has no finite deadline",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_parked_qemu_identity_probe_reaches_its_named_deadline",
        },
    },
    Mutation {
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
        file: "xtask/src/evidence.rs",
        find: "    let root = crate::mutation_workspace::prepare_real_directory_tree(\n        &evidence_root(paths),\n        \"evidence root\",\n    )?;",
        replace: "    let root = evidence_root(paths);\n    std::fs::create_dir_all(&root)\n        .with_context(|| format!(\"creating evidence directory {}\", root.display()))?;",
        breaks: "a redirected target receives evidence bytes before its ancestry is refused",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_redirected_target_is_refused_before_evidence_root_creation",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    let root = crate::mutation_workspace::prepare_real_directory_tree(\n        &qualification_root(paths),\n        \"qualification root\",\n    )?;",
        replace: "    let root = qualification_root(paths);\n    std::fs::create_dir_all(&root).with_context(|| format!(\"creating {}\", root.display()))?;",
        breaks: "a redirected target receives qualification bytes before its ancestry is refused",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_redirected_target_is_refused_before_qualification_root_creation",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        let target = crate::mutation_workspace::prepare_real_directory_tree(\n            paths.target_dir(),\n            \"API audit target directory\",\n        )?;",
        replace: "        let target = paths.target_dir().to_path_buf();\n        std::fs::create_dir_all(&target)\n            .with_context(|| format!(\"creating {}\", target.display()))?;",
        breaks: "a redirected target receives API audit state before its ancestry is refused",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::redirected_api_audit_paths_are_refused_without_touching_outside_bytes",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    match std::fs::symlink_metadata(&directory) {\n        Ok(_) => {\n            let canonical = crate::mutation_workspace::canonical_existing_real_directory(\n                &directory,\n                \"API audit working directory\",\n            )?;\n            ensure!(\n                canonical.parent() == Some(state),\n                \"API audit working directory escaped its canonical state root\"\n            );\n            std::fs::remove_dir_all(&canonical)\n                .with_context(|| format!(\"removing stale {}\", canonical.display()))?;\n        }\n        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}\n        Err(error) => {\n            return Err(error).with_context(|| format!(\"inspecting {}\", directory.display()));\n        }\n    }\n    crate::mutation_workspace::prepare_real_child(state, &directory, \"API audit working directory\")",
        replace: "    if directory.exists() {\n        std::fs::remove_dir_all(&directory)\n            .with_context(|| format!(\"removing stale {}\", directory.display()))?;\n    }\n    std::fs::create_dir_all(&directory)\n        .with_context(|| format!(\"creating {}\", directory.display()))?;\n    Ok(directory)",
        breaks: "a redirected API working directory can become a recursive reset target",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::redirected_api_audit_paths_are_refused_without_touching_outside_bytes",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "fn open_audit_lock(state: &Path) -> Result<File> {\n    let lock_path = state.join(\"LOCK\");\n    crate::mutation_workspace::require_direct_file_if_present(state, &lock_path, \"API audit lock\")?;\n    let mut lock_options = OpenOptions::new();\n    OpenOptions::create(&mut lock_options, true);\n    OpenOptions::truncate(&mut lock_options, false);\n    OpenOptions::read(&mut lock_options, true);\n    OpenOptions::write(&mut lock_options, true);\n    let lock = OpenOptions::open(&lock_options, &lock_path)\n        .with_context(|| format!(\"opening {}\", lock_path.display()))?;\n    crate::mutation_workspace::require_direct_file(state, &lock_path, \"API audit lock\")?;\n    lock.try_lock()\n        .context(\"another public API audit is already running\")?;\n    Ok(lock)\n}",
        replace: "fn open_audit_lock(state: &Path) -> Result<File> {\n    let lock_path = state.join(\"LOCK\");\n    let mut lock_options = OpenOptions::new();\n    OpenOptions::create(&mut lock_options, true);\n    OpenOptions::truncate(&mut lock_options, false);\n    OpenOptions::read(&mut lock_options, true);\n    OpenOptions::write(&mut lock_options, true);\n    let lock = OpenOptions::open(&lock_options, &lock_path)\n        .with_context(|| format!(\"opening {}\", lock_path.display()))?;\n    lock.try_lock()\n        .context(\"another public API audit is already running\")?;\n    Ok(lock)\n}",
        breaks: "an API audit lock path can redirect the exclusive lock to an external file",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::redirected_api_audit_paths_are_refused_without_touching_outside_bytes",
        },
    },
    Mutation {
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
        file: "xtask/src/mutation_workspace.rs",
        find: "fn write_source(destination: &Path, bytes: &[u8]) -> Result<()> {\n    write_source_with(destination, bytes, |from, to| std::fs::rename(from, to))\n}",
        replace: "fn write_source(destination: &Path, bytes: &[u8]) -> Result<()> {\n    std::fs::write(destination, bytes)\n        .with_context(|| format!(\"overwriting {}\", destination.display()))\n}",
        breaks: "a workspace mutation can overwrite an external hard-linked inode",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::workspace_replacement_never_writes_through_an_external_hardlink",
        },
    },
    Mutation {
        file: "xtask/src/bounded_process.rs",
        find: "    let mut file = retained\n        .try_clone()\n        .with_context(|| format!(\"cloning captured {label} {}\", path.display()))?;",
        replace: "    let _discarded_capture_handle = retained;\n    let mut file = File::open(path)\n        .with_context(|| format!(\"reopening captured {label} {}\", path.display()))?;",
        breaks: "a substituted capture path can forge external-tool output after execution",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "bounded_process::tests::collection_reads_the_original_capture_handle_not_a_substituted_path",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "    Ok(SweepLock { _file: file })",
        replace: "    file.set_len(0)\n        .with_context(|| format!(\"truncating mutation lock {}\", path.display()))?;\n    Ok(SweepLock { _file: file })",
        breaks: "a mutation lock can truncate an external hard-linked inode",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_mutation_lock_never_writes_through_an_external_hardlink",
        },
    },
    Mutation {
        file: "xtask/src/model.rs",
        find: "            ensure!(\n                canonical.parent() == Some(root.as_path()),\n                \"model-check scratch directory escaped its state root\"\n            );",
        replace: "            let _unchecked_scratch_parent = canonical.parent();",
        breaks: "a redirected TLC metadir can become an external recursive-delete target",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::a_redirected_model_scratch_is_refused_without_touching_outside_bytes",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "    crate::mutation_workspace::prepare_real_directory_tree(&root, \"VMware root\")?;",
        replace: "    std::fs::create_dir_all(&root)\n        .with_context(|| format!(\"creating VMware root {}\", root.display()))?;",
        breaks: "a redirected repository ancestor receives a VMware root before refusal",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::a_redirected_repository_ancestor_is_refused_before_vmware_root_creation",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "fn require_real_directory(path: &Path, label: &str) -> Result<()> {\n    crate::mutation_workspace::canonical_existing_real_directory(path, label)?;\n    Ok(())\n}",
        replace: "fn require_real_directory(path: &Path, label: &str) -> Result<()> {\n    let metadata = std::fs::symlink_metadata(path)\n        .with_context(|| format!(\"inspecting {}\", path.display()))?;\n    ensure!(\n        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),\n        \"{label} is not a real directory: {}\",\n        path.display()\n    );\n    Ok(())\n}",
        breaks: "an evidence root can hide a redirected ancestor behind a real final directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_substituted_evidence_root_is_never_recursively_reset",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "fn require_real_directory(path: &Path, label: &str) -> Result<()> {\n    crate::mutation_workspace::canonical_existing_real_directory(path, label)?;\n    Ok(())\n}",
        replace: "fn require_real_directory(path: &Path, label: &str) -> Result<()> {\n    let metadata = std::fs::symlink_metadata(path)\n        .with_context(|| format!(\"inspecting {}\", path.display()))?;\n    ensure!(\n        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),\n        \"{label} is not a real directory: {}\",\n        path.display()\n    );\n    Ok(())\n}",
        breaks: "a qualification root can hide a redirected ancestor behind a real final directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_qualification_root_cannot_be_substituted",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "fn require_canonical_child(parent: &Path, child: &Path, label: &str) -> Result<()> {\n    let canonical_parent =\n        crate::mutation_workspace::canonical_existing_real_directory(parent, label)?;\n    let canonical_child =\n        crate::mutation_workspace::canonical_existing_real_directory(child, label)?;\n    ensure!(\n        canonical_child.parent() == Some(canonical_parent.as_path()),\n        \"{label} escaped its canonical parent\"\n    );\n    Ok(())\n}",
        replace: "fn require_canonical_child(parent: &Path, child: &Path, label: &str) -> Result<()> {\n    let canonical_parent = std::fs::canonicalize(parent)\n        .with_context(|| format!(\"canonicalizing {}\", parent.display()))?;\n    let canonical_child = std::fs::canonicalize(child)\n        .with_context(|| format!(\"canonicalizing {}\", child.display()))?;\n    ensure!(\n        canonical_child.parent() == Some(canonical_parent.as_path()),\n        \"{label} escaped its canonical parent\"\n    );\n    Ok(())\n}",
        breaks: "a VMware selector can hide a redirected ancestor behind canonical siblings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::destructive_vmware_paths_require_one_canonical_parent",
        },
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "                && current == self.expected,",
        replace: "                && current != self.expected,",
        breaks: "refused file work can roll back a queued read it no longer owns",
        reddens: Caught::Panic {
            marker: "refused file work no longer owns its queued read",
            why: "the refusal path has no authority to remove a different pending read, so an ownership mismatch must stop before mutating the queue",
        },
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "                !entry.settling && *entry == self.work.expected,",
        replace: "                entry.settling && *entry == self.work.expected,",
        breaks: "running file work can start after its pending entry began settling",
        reddens: Caught::Panic {
            marker: "running file work no longer owns its read",
            why: "a settling entry has already transferred terminal authority, so starting its read would let two paths consume the same queued operation",
        },
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            entry.settling && current == entry,",
        replace: "            !entry.settling && current == entry,",
        breaks: "terminal file work can clear a pending entry before settlement owns it",
        reddens: Caught::Panic {
            marker: "terminal work no longer owns its pending entry",
            why: "only the exact settling snapshot carries terminal removal authority, so clearing any other state would discard or replace live work",
        },
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        if self.head >= WORK_DEPTH || self.tail >= WORK_DEPTH || self.len > WORK_DEPTH {",
        replace: "        if self.head > WORK_DEPTH || self.tail >= WORK_DEPTH || self.len > WORK_DEPTH {",
        breaks: "a work queue accepts its exclusive head bound as an index",
        reddens: Caught::Check("a cancelled operation and its cancellation complete separately"),
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            pending.checked_add(1)",
        replace: "            Some(pending.wrapping_add(1))",
        breaks: "reap request admission wraps its outstanding count through zero",
        reddens: Caught::Check("a cancelled operation and its cancellation complete separately"),
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            pending.checked_sub(1)",
        replace: "            Some(pending.wrapping_sub(1))",
        breaks: "an empty reap request counter wraps to the maximum",
        reddens: Caught::Check("a cancelled operation and its cancellation complete separately"),
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "        if self.reserved + self.completion.occupied() >= self.completion.capacity().entries() {",
        replace: "        if true || self.reserved + self.completion.occupied() >= self.completion.capacity().entries() {",
        breaks: "the scheduled ring proof can pass when production admission always refuses",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "scheduled_calls_preserve_the_production_reservation",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "        write_source(&path, mutated.as_bytes())\n            .with_context(|| format!(\"writing isolated mutation {}\", path.display()))",
        replace: "        std::fs::write(&path, mutated)\n            .with_context(|| format!(\"writing isolated mutation {}\", path.display()))",
        breaks: "an active mutation session can overwrite an external hard-linked inode",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::workspace_replacement_never_writes_through_an_external_hardlink",
        },
    },
    Mutation {
        file: "xtask/src/bounded_process.rs",
        find: "        let status = match try_wait(&mut child) {\n            Ok(status) => status,\n            Err(error) => {\n                let error = anyhow::anyhow!(\"checking {label}: {error}\");\n                return stop_after_observation_failure(child, try_wait, kill, error, label);\n            }\n        };",
        replace: "        let status = try_wait(&mut child)\n            .with_context(|| format!(\"checking {label}\"))?;",
        breaks: "a child polling failure can leave the direct child running",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "bounded_process::tests::a_poll_failure_stops_the_direct_child_before_returning",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        self.may_reach(virtual_base, len)?;\n        if let Some(pinned) = protected_stack_overlap(self.root(), virtual_base, len) {",
        replace: "        self.may_reach(virtual_base, len)?;\n        if let Some(pinned) = permanent_stack_overlap(self.root(), virtual_base, len) {",
        breaks: "unmap can clear a live task stack or its guard",
        reddens: Caught::Check("unmapping refuses what it does not own"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if current_slot == Some(slot) {",
        replace: "    if current_slot == Some(slot) && cpu::page_table_base() != active_root() {",
        breaks: "an active cleared leaf can remain cached until its frame is reused",
        reddens: Caught::Check("an active unmap exposes only its replacement"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if current_slot == Some(slot) {",
        replace: "    if cpu::page_table_base() == active_root() {",
        breaks: "root identity hides a shared higher-half leaf reached by the current CR3",
        reddens: Caught::Check("a shared kernel unmap evicts the live user root"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if current_slot == Some(slot) {",
        replace: "    if true {",
        breaks: "clearing an inactive lower-root leaf evicts an unrelated live translation",
        reddens: Caught::Check("an inactive lower-root unmap preserves the live observer"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "const NOT_THE_CALLERS: Flags = Flags::OWNED.union(Flags::HUGE).union(Flags::GLOBAL);",
        replace: "const NOT_THE_CALLERS: Flags = Flags::OWNED.union(Flags::HUGE);",
        breaks: "safe mappings can preserve a caller-supplied global translation",
        reddens: Caught::Check("caller global is removed from safe mappings"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "    assert!(\n        !page_table_context_ids_are_enabled(),\n        \"page-table switching requires CR4.PCIDE to remain clear\"\n    );",
        replace: "    assert!(\n        page_table_context_ids_are_enabled(),\n        \"page-table switching requires CR4.PCIDE to remain clear\"\n    );",
        breaks: "page-table switching accepts PCIDE and rejects the required clear state",
        reddens: Caught::Panic {
            marker: "page-table switching requires CR4.PCIDE to remain clear",
            why: "the CR3-flush contract requires PCID to remain disabled before any address-space switch",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "    if let Some(address) = arena.invalid_page() {",
        replace: "    if let Some(address) = arena.invalid_page().filter(|_| false) {",
        breaks: "task arena validation ignores an unmapped stack or guard",
        reddens: Caught::Check("thread stacks are guarded below"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "    let layout = match validate_arena(&arena) {",
        replace: "    let layout = match Ok::<stack_layout::StackLayout, TaskError>(*arena.layout()) {",
        breaks: "task initialization can publish and pin an arena it never validated",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::task_init_validates_the_arena_before_publication_and_pinning",
        },
    },
    Mutation {
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
        file: "xtask/src/qualification.rs",
        find: "            crate::model::verify_transcript(name, &bytes)?;",
        replace: "            let _unchecked_model_transcript = (name, &bytes);",
        breaks: "offline evidence can accept one valid TLC run donated under another identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::model_support_rejects_omitted_substituted_and_reordered_runs",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        relative: \"qualification/model/ring-clean.transcript\",",
        replace: "        relative: \"qualification/model/ring-clean.transcript-omitted\",",
        breaks: "the outer evidence bundle can omit a required TLC execution",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::qualification_records_cannot_be_omitted_or_changed_offline",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            let mut activated = false;\n            for (&(file, site), evidence) in &self.dispatches {",
        replace: "            let mut activated = false;\n            for activation in &inventory.trait_activations {\n                if activation.declaration.is_some()\n                    && live.contains(&activation.self_type)\n                    && live.insert(activation.implementation)\n                {\n                    pending.push_back(activation.implementation);\n                    activated = true;\n                }\n            }\n            for (&(file, site), evidence) in &self.dispatches {",
        breaks: "a reachable type can activate an uncalled trait member",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_reachable_type_does_not_activate_an_uncalled_trait_member",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        ensure!(
            !inventory.unresolved_trait_implementations.contains(&caller),
            \"an API diagnostic for {} originates in unresolved trait implementation {}\",
            inventory.nodes[id].label,
            inventory.nodes[caller].label
        );",
        replace: "        let _unchecked_trait_implementation = &inventory.unresolved_trait_implementations;",
        breaks: "an unresolved trait implementation can silently default to a caller",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::an_unresolved_trait_implementation_refuses_a_clean_verdict",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "fn prelude_external_trait(path: &str) -> bool {\n    matches!(",
        replace: "fn prelude_external_trait(path: &str) -> bool {\n    return false;\n    #[allow(unreachable_code)]\n    matches!(",
        breaks: "edition-2024 prelude traits lose their external authority classification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_edition_2024_prelude_trait_is_external",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if namespace == 0 || !Self::contiguous_extent(address, IDENTIFY_TRANSFER_BYTES, page_size) {",
        replace: "        if namespace == 0 {",
        breaks: "identify can bind a partial, unaligned or wrapping DMA page",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::identify_binds_one_complete_aligned_page",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "            || !Self::contiguous_extent(address, transfer.bytes(), page_size)",
        replace: "            || false",
        breaks: "read can bind bytes outside its validated contiguous DMA page",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::read_binds_its_validated_extent_to_one_page",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if queue == 0 || !Self::contiguous_extent(address, capacity.completion_bytes(), page_size) {",
        replace: "        if queue == 0 {",
        breaks: "completion-queue creation can bind an incomplete contiguous extent",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::queue_commands_bind_the_complete_contiguous_extent",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        self.state = DmaOwnership::InFlight;\n        Some(handoff(DmaPublication { _state: self }))",
        replace: "        let outcome = handoff(DmaPublication { _state: self });\n        self.state = DmaOwnership::InFlight;\n        Some(outcome)",
        breaks: "doorbell handoff observes DMA memory before publication closes cancellation",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::publication_closes_the_unpublished_cancellation_door",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if self.state != DmaOwnership::InFlight {",
        replace: "        if self.state == DmaOwnership::Idle {",
        breaks: "DMA completion accepts memory that was never published to the device",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::publication_closes_the_unpublished_cancellation_door",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "        return if another_collector {
            CollectOutcome::Retained
        } else {
            CollectOutcome::Empty
        };",
        replace: "        return if another_collector {
            CollectOutcome::Empty
        } else {
            CollectOutcome::Empty
        };",
        breaks: "an in-flight collector can lose the worker retry that owns its progress",
        reddens: Caught::Check("an in-flight collector keeps the worker retry live"),
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "impl Drop for USegment {\n    fn drop(&mut self) {\n        let Some(mut segment) = self.segment.take() else {\n            return;\n        };\n        if !segment.zero() {",
        replace: "impl Drop for USegment {\n    fn drop(&mut self) {\n        let Some(mut segment) = self.segment.take() else {\n            return;\n        };\n        if false {",
        breaks: "a failed external-byte zeroing attempt still returns the page to the allocator",
        reddens: Caught::Check("external user bytes are zeroed before free"),
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "impl Drop for USegment {",
        replace: "#[cfg(any())]\nimpl Drop for USegment {",
        breaks: "natural user-segment drop returns external bytes without zeroing them",
        reddens: Caught::Check("external user bytes are zeroed before free"),
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        if !segment.zero() {\n            #[cfg(feature = \"selftest\")]\n            let address = segment.address();\n            core::mem::forget(segment);\n            #[cfg(feature = \"selftest\")]\n            observe_external_run_withheld_for_test(address);\n        }",
        replace: "        if !segment.zero() {\n            #[cfg(feature = \"selftest\")]\n            let address = segment.address();\n            drop(segment);\n            #[cfg(feature = \"selftest\")]\n            observe_external_run_withheld_for_test(address);\n        }",
        breaks: "a refused external clear returns a dirty run after deciding to withhold it",
        reddens: Caught::Check("a refused external clear withholds its run"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "            if zombies.settling[slot] {\n                return None;\n            }",
        replace: "            if false && zombies.settling[slot] {\n                return None;\n            }",
        breaks: "the same zombie can acquire a second settlement collector",
        reddens: Caught::Check("one zombie has one settlement collector"),
    },
    Mutation {
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "pub const fn matches_permitted_configuration(configured: u64) -> bool {\n    configured == PERMITTED\n}",
        replace: "pub const fn matches_permitted_configuration(configured: u64) -> bool {\n    configured & PERMITTED == PERMITTED\n}",
        breaks: "extra live XCR0 components can escape the fixed save-area policy",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::the_live_configuration_is_exactly_the_fixed_policy",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    fn device_reclaim_error(&self) -> Option<ReclaimError> {\n        let mappings = self.devices.iter().flatten().count();\n        (mappings != 0).then_some(ReclaimError::Devices(mappings))\n    }",
        replace: "    fn device_reclaim_error(&self) -> Option<ReclaimError> {\n        let _mappings = self.devices.iter().flatten().count();\n        None\n    }",
        breaks: "a live device ledger no longer prevents its page tables from being reclaimed",
        reddens: Caught::Check("active device tables cannot be reclaimed"),
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "    for run in map.allocatable_runs() {\n        ignored += frames.insert(run.base(), run.bytes()).ignored;\n    }",
        replace: "    for region in map.regions().filter(|region| region.kind.is_allocatable()) {\n        ignored += frames.insert(region.base, region.len).ignored;\n    }",
        breaks: "raw usable claims seed frames that an overlapping reserved claim excludes",
        reddens: Caught::Check("the frame allocator admits only exclusively usable frames"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            Err(reason) => Err(UserMapError::Mapping { reason, run }),",
        replace: "            Err(_reason) => {\n                run.commit_mapping();\n                Ok(())\n            }",
        breaks: "a refused user mapping consumes dirty-page ownership as if the mapping committed",
        reddens: Caught::Check("external user bytes are zeroed before free"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !crate::maskable_interrupts_are_ready() {\n        warn!(\"user {which}: maskable interrupts are not admitted\");\n        refuse_cross(slot);\n    }",
        replace: "    if false {\n        warn!(\"user {which}: maskable interrupts are not admitted\");\n        refuse_cross(slot);\n    }",
        breaks: "aged interrupt-readiness evidence can cross the final user-transition door",
        reddens: Caught::Check("unadmitted interrupts cannot be enabled by a user transition"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "    if !disabled.were_enabled() {",
        replace: "    if false {",
        breaks: "a user return frame can enable interrupts that were not admitted before transition",
        reddens: Caught::Check("unadmitted interrupts cannot be enabled by a user transition"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "fn refuse_cross(slot: usize) -> ! {\n    let _disabled = interrupt::disable();\n    orphan(slot);\n    thread::exit()\n}",
        replace: "fn refuse_cross(slot: usize) -> ! {\n    let _disabled = interrupt::disable();\n    let _ = slot;\n    thread::exit()\n}",
        breaks: "a final user-transition refusal can abandon its committed process owner",
        reddens: Caught::Check("unadmitted interrupts cannot be enabled by a user transition"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "#[cfg(feature = \"selftest\")]\npub fn fault_window() -> (u64, u64) {",
        replace: "pub fn fault_window() -> (u64, u64) {",
        breaks: "the user-copy fault window reaches the shipped wost graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        file: "xtask/src/sha256.rs",
        find: "            if self.buffered != BLOCK_SIZE {",
        replace: "            if false && self.buffered != BLOCK_SIZE {",
        breaks: "an incremental SHA-256 update discards a partial block before it is complete",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "sha256::tests::incremental_hashing_matches_every_adversarial_block_split",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        initial <= maximum,\n        \"{label} is {initial} bytes, exceeding the {maximum}-byte allocation limit\"\n    );",
        replace: "    ensure!(\n        true,\n        \"{label} is {initial} bytes, exceeding the {maximum}-byte allocation limit\"\n    );",
        breaks: "a semantic qualification artifact can allocate from an already oversized length",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::semantic_artifacts_are_bounded_before_and_during_allocation",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        bytes.len() as u64 <= maximum,\n        \"{label} grew beyond the {maximum}-byte allocation limit while being read\"\n    );",
        replace: "    ensure!(\n        true,\n        \"{label} grew beyond the {maximum}-byte allocation limit while being read\"\n    );",
        breaks: "a semantic qualification artifact can grow past its allocation cap during the read",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::semantic_artifacts_are_bounded_before_and_during_allocation",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        ensure!(\n            length == maximum,\n            \"qualification artifact {name} is {length} bytes, expected exactly {maximum}\"\n        );",
        replace: "        ensure!(\n            length <= maximum,\n            \"qualification artifact {name} is {length} bytes, expected exactly {maximum}\"\n        );",
        breaks: "an undersized deterministic machine image can enter qualification evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            initial == maximum,\n            \"{label} is {initial} bytes, expected exactly {maximum}\"",
        replace: "            initial >= maximum,\n            \"{label} is {initial} bytes, expected exactly {maximum}\"",
        breaks: "an oversized sparse VMware image reaches the qualification capture stream",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware_capture_preflights_image_size_before_streaming",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    verify_artifacts_with_image_bytes(record, root, boot_image_bytes())",
        replace: "    verify_artifacts_with_image_bytes(\n        record,\n        root,\n        std::fs::metadata(root.join(ARTIFACTS[1].relative))?.len(),\n    )",
        breaks: "an undersized release image can satisfy the offline evidence verifier",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::artifact_bytes_must_match_the_manifest",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "    ensure!(\n        initial <= maximum,\n        \"{label} is {initial} bytes, exceeding the {maximum}-byte allocation limit\"\n    );",
        replace: "    ensure!(\n        true,\n        \"{label} is {initial} bytes, exceeding the {MAX_STATE_FILE_BYTES}-byte allocation limit\"\n    );",
        breaks: "an oversized mutation-state file can drive allocation before its cap is enforced",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::oversized_mutation_state_is_refused_before_payload_allocation",
        },
    },
    Mutation {
        file: "xtask/src/supply_chain.rs",
        find: "    let result = (|| {\n        let mut options = OpenOptions::new();",
        replace: "    std::fs::write(destination, bytes)\n        .with_context(|| format!(\"writing old SPDX document {}\", destination.display()))?;\n    let result = (|| {\n        let mut options = OpenOptions::new();",
        breaks: "SPDX publication writes through a preexisting hardlink before replacing its name",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::spdx_publication_never_writes_through_an_external_link",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/local_vector.rs",
        find: "        if vector.get() == spurious.get() {",
        replace: "        if vector.get() == spurious.get() && false {",
        breaks: "a configured APIC delivery can select the spurious vector",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "command::tests::a_spurious_self_delivery_performs_no_fake_register_write",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/timer.rs",
        find: "    if current_count != 0 {",
        replace: "    if false {",
        breaks: "a running local timer can be unmasked under a new vector",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "timer::tests::refused_one_shot_plans_perform_no_fake_register_write",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/in_service.rs",
        find: "    for (bank, bits) in registers.iter().enumerate().rev() {",
        replace: "    for (bank, bits) in registers.iter().enumerate() {",
        breaks: "EOI authority selects the lowest rather than highest in-service vector",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::the_highest_set_vector_is_the_dismissal_target",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "    if EOI_CLAIMED\n        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)\n        .is_err()\n    {",
        replace: "    if EOI_CLAIMED\n        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)\n        .is_err()\n        && false\n    {",
        breaks: "one APIC delivery can mint more than one completion authority",
        reddens: Caught::Check("one delivery mints one completion authority"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "#[derive(Debug)]\npub struct DisarmedTimer",
        replace: "#[derive(Debug, Clone, Copy)]\npub struct DisarmedTimer",
        breaks: "a timer-disarm proof can be duplicated",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_completion_and_disarm_authorities_are_linear",
        },
    },
    Mutation {
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
        file: "xtask/src/vmware.rs",
        find: "        let message = line.strip_prefix(\"[info ] \")?;",
        replace: "        let message = line.split_once(\"] \").map(|(_, message)| message)?;",
        breaks: "a VMware warning can forge a required canonical execution record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::vmware_evidence_requires_canonical_info_records",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "    OpenOptions::create_new(&mut options, true);",
        replace: "    OpenOptions::create(&mut options, true);\n    OpenOptions::truncate(&mut options, true);",
        breaks: "a generated VMware output can truncate a preexisting external link",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::vmware_generation_never_writes_through_preexisting_output_links",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    require_observed_vmx(&observed, &expected)\n}",
        replace: "    require_observed_vmx(&observed, &observed)\n}",
        breaks: "offline VMware evidence can substitute another absolute running VMX",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::offline_vmware_observer_is_bound_to_its_exact_selector",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    let mut output_options = OpenOptions::new();\n    OpenOptions::create_new(&mut output_options, true);\n    OpenOptions::write(&mut output_options, true);\n    let mut output = OpenOptions::open(&output_options, destination)",
        replace: "    let mut output_options = OpenOptions::new();\n    OpenOptions::create(&mut output_options, true);\n    OpenOptions::truncate(&mut output_options, true);\n    OpenOptions::write(&mut output_options, true);\n    let mut output = OpenOptions::open(&output_options, destination)",
        breaks: "the API-audit source copy can truncate a preexisting external link",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::api_audit_copy_never_writes_through_an_external_link",
        },
    },
    Mutation {
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
        file: "libs/x86-apic/src/in_service.rs",
        find: "pub fn owns(registers: &[u32; 8], vector: InterruptVector) -> bool {\n    highest(registers) == Some(vector.get())\n}",
        replace: "pub fn owns(registers: &[u32; 8], vector: InterruptVector) -> bool {\n    let _ = vector;\n    highest(registers).is_some()\n}",
        breaks: "a wrong-source in-service vector can authorize an EOI write",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::absent_and_wrong_sources_plan_no_fake_eoi_write",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/in_service.rs",
        find: "    matches!(interrupts, Interrupts::Excluded)",
        replace: "    true",
        breaks: "EOI completion can write while maskable interrupts are enabled",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::only_named_live_ownership_observations_authorize_completion",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/in_service.rs",
        find: "        && matches!(frame_liveness, FrameLiveness::Live)",
        replace: "        && true",
        breaks: "EOI completion accepts a token whose trap frame is no longer live",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::only_named_live_ownership_observations_authorize_completion",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/in_service.rs",
        find: "        && frame_vector == token_vector.get() as u64",
        replace: "        && true",
        breaks: "EOI completion accepts a token from another trap vector",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::only_named_live_ownership_observations_authorize_completion",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/in_service.rs",
        find: "        && matches!(dismissal, DismissalOwnership::Owned)",
        replace: "        && true",
        breaks: "EOI completion ignores which vector the APIC would dismiss",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::only_named_live_ownership_observations_authorize_completion",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    fn holds_function(&self, function: FunctionLease, lease: CommandLease) -> bool {\n        lease.function == function && self.slots.iter().flatten().any(|held| *held == lease)\n    }",
        replace: "    fn holds_function(&self, function: FunctionLease, lease: CommandLease) -> bool {\n        lease.function == function\n            && self.slots.iter().flatten().any(|held| {\n                held.function == lease.function && held.generation == lease.generation\n            })\n    }",
        breaks: "command state can be consumed through a different registry with the same function and generation",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_state_is_bound_to_its_registry_function_and_command_offset",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    pub fn holds(&self, authority: &FunctionAuthority) -> bool {\n        self.slots\n            .iter()\n            .flatten()\n            .any(|held| *held == authority.lease)\n    }",
        replace: "    pub fn holds(&self, authority: &FunctionAuthority) -> bool {\n        let _ = authority;\n        true\n    }",
        breaks: "a foreign function authority reaches configuration cycles through another registry",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::configuration_cycles_require_the_issuing_function_registry",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if !functions.holds(authority) {\n        return Err(EnableFailure::from(EnableError::Stale));\n    }\n    enable_memory_space_with_leases(&mut functions.commands, authority, access)",
        replace: "    if functions.holds(authority) {\n        return Err(EnableFailure::from(EnableError::Stale));\n    }\n    enable_memory_space_with_leases(&mut functions.commands, authority, access)",
        breaks: "memory decode reverses which function registry may reach configuration space",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::public_memory_decode_accepts_only_its_function_authority",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    admit_bus_master_with_leases(&mut functions.commands, authority.function, state, access)",
        replace: "    let _ = (&mut functions.commands, authority, state, access);\n    Ok(())",
        breaks: "bus-master admission reports success without changing the device or state",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::mapped_bus_master_admission_requires_its_bar_and_command_owner",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if !functions.holds_bar(authority) {\n        return Err(EnableError::Stale);\n    }\n    admit_bus_master_with_leases(&mut functions.commands, authority.function, state, access)",
        replace: "    if functions.holds_bar(authority) {\n        return Err(EnableError::Stale);\n    }\n    admit_bus_master_with_leases(&mut functions.commands, authority.function, state, access)",
        breaks: "bus-master admission rejects its live mapped BAR owner",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::mapped_bus_master_admission_requires_its_bar_and_command_owner",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if !functions.holds(authority) {\n        return Err(RestoreFailure {\n            reason: EnableError::Stale,\n            state,\n        });\n    }\n    commit_command_with_leases(&mut functions.commands, authority, state, access)",
        replace: "    if functions.holds(authority) {\n        return Err(RestoreFailure {\n            reason: EnableError::Stale,\n            state,\n        });\n    }\n    commit_command_with_leases(&mut functions.commands, authority, state, access)",
        breaks: "command commit rejects its live function authority before consuming the sublease",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::public_command_commit_consumes_the_live_sublease",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    retry_probe_with_leases(&mut functions.commands, authority, retry, access)",
        replace: "    let _ = (&mut functions.commands, authority, retry, access);\n    Ok(())",
        breaks: "BAR retry reports success without restoring hardware or settling its sublease",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::public_probe_retry_restores_hardware_and_releases_its_sublease",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if !functions.holds(authority) {\n        return Err(ProbeFailure::with_retry(\n            ProbeError::Unavailable(EnableError::Stale),\n            retry,\n        ));\n    }\n    retry_probe_with_leases(&mut functions.commands, authority, retry, access)",
        replace: "    if functions.holds(authority) {\n        return Err(ProbeFailure::with_retry(\n            ProbeError::Unavailable(EnableError::Stale),\n            retry,\n        ));\n    }\n    retry_probe_with_leases(&mut functions.commands, authority, retry, access)",
        breaks: "BAR retry rejects the issuing registry before restoring its retained hardware state",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::public_probe_retry_restores_hardware_and_releases_its_sublease",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    fn reserve(&mut self, authority: &FunctionAuthority) -> Result<CommandLease, EnableError> {
        if self
            .slots
            .iter()
            .flatten()
            .any(|lease| lease.function == authority.lease)
        {",
        replace: "    fn reserve(&mut self, authority: &FunctionAuthority) -> Result<CommandLease, EnableError> {
        if self
            .slots
            .iter()
            .flatten()
            .any(|lease| lease.function != authority.lease)
        {",
        breaks: "a second command transition can nest over an already leased function",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_leases_refuse_nesting_and_preserve_uncertain_retry_authority",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if current != state.installed {\n        return Err(RestoreFailure {\n            reason: EnableError::Changed(current),\n            state,\n        });\n    }\n    if leases.release(state.lease) {",
        replace: "    if leases.release(state.lease) {",
        breaks: "command commit can consume a lease after the installed word changed",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_consumption_requires_the_installed_word_and_live_generation",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    fn holds_function(&self, function: FunctionLease, lease: CommandLease) -> bool {\n        lease.function == function && self.slots.iter().flatten().any(|held| *held == lease)\n    }",
        replace: "    fn holds_function(&self, _function: FunctionLease, lease: CommandLease) -> bool {\n        self.slots.iter().flatten().any(|held| {\n            held.registry == lease.registry && held.generation == lease.generation\n        })\n    }",
        breaks: "command state can be consumed for another function in the same registry generation",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_state_is_bound_to_its_registry_function_and_command_offset",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        let Some(slot) = self.slots.iter_mut().find(|held| **held == Some(lease)) else {",
        replace: "        let Some(slot) = self.slots.iter_mut().find(|held| {\n            held.is_some_and(|held| {\n                held.at == lease.at && held.generation == lease.generation\n            })\n        }) else {",
        breaks: "a BAR probe token can release the matching generation from a different registry",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::bar_probe_reservations_are_registry_bound",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "            state: Some(state),",
        replace: "            state: None,",
        breaks: "an uncertain command transition loses the exclusive retry authority",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_leases_refuse_nesting_and_preserve_uncertain_retry_authority",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let wanted = current | command::MEMORY_SPACE | command::BUS_MASTER;\n    state.phase = CommandPhase::Restoring(wanted);\n    let result = change_command(at, current, wanted, &mut access);",
        replace: "    let wanted = current | command::MEMORY_SPACE | command::BUS_MASTER;\n    let result = change_command(at, current, wanted, &mut access);",
        breaks: "an interrupted command upgrade publishes no restoration phase before its write",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_interrupted_upgrade_records_retry_authority_before_the_write",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        Address(self.function.at.0 | header::COMMAND as u32)",
        replace: "        Address(self.function.at.0 | header::BAR0 as u32)",
        breaks: "a command lease binds a BAR offset instead of the function command register",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_state_is_bound_to_its_registry_function_and_command_offset",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        let Some(generation) = self.generation.checked_add(1) else {\n            return Err(EnableError::Exhausted);\n        };",
        replace: "        let generation = self.generation.wrapping_add(1);",
        breaks: "command generations wrap and replay instead of refusing exhaustion",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_generation_exhaustion_refuses_before_configuration_access",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "            Self::Cold => Ok(()),",
        replace: "            Self::Cold => Err(BeginError::Ready),",
        breaks: "a cold local APIC refuses its sole initialization transition",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::only_cold_hardware_can_begin",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "            Ok(()) => Ok(Self {\n                lifecycle: Lifecycle::Initializing,\n                next_stage: 0,\n            }),",
        replace: "            Ok(()) => Ok(Self {\n                lifecycle: Lifecycle::Ready,\n                next_stage: 0,\n            }),",
        breaks: "an APIC initialization transaction publishes readiness before its first hardware boundary",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::every_intermediate_observation_refuses_reentry_and_readiness",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "    pub fn refuse(&mut self) -> Lifecycle {\n        if matches!(self.lifecycle, Lifecycle::Initializing) {\n            self.lifecycle = Lifecycle::Poisoned;\n        }\n        self.lifecycle\n    }",
        replace: "    pub fn refuse(&mut self) -> Lifecycle {\n        if matches!(self.lifecycle, Lifecycle::Initializing) {\n            self.lifecycle = Lifecycle::Cold;\n        }\n        self.lifecycle\n    }",
        breaks: "a late APIC initialization refusal becomes retryable instead of terminal",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::every_late_refusal_is_unpublished_and_terminal",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "const fn progress_is_excluded(interrupts: Interrupts) -> bool {\n    matches!(interrupts, Interrupts::Excluded)\n}",
        replace: "const fn progress_is_excluded(interrupts: Interrupts) -> bool {\n    let _ = interrupts;\n    true\n}",
        breaks: "an APIC hardware stage or publication can continue after interrupt exclusion is lost",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::every_hardware_boundary_and_publication_require_interrupt_exclusion",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "        if InitStage::ALL.get(self.next_stage) != Some(&stage) {",
        replace: "        if false {",
        breaks: "APIC hardware initialization stages can run out of order",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::publication_requires_every_stage_in_exact_order",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "        if self.next_stage != InitStage::ALL.len() {",
        replace: "        if false {",
        breaks: "APIC readiness can publish before every hardware stage completes",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::publication_requires_every_stage_in_exact_order",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "            Lifecycle::Cold.bits(),\n            Lifecycle::Initializing.bits(),\n            Ordering::AcqRel,",
        replace: "            Lifecycle::Cold.bits(),\n            Lifecycle::Ready.bits(),\n            Ordering::AcqRel,",
        breaks: "the live APIC publishes readiness before its hardware transaction begins",
        reddens: Caught::Check(
            "local apic initialization stayed unpublished and interrupt-excluded",
        ),
    },
    Mutation {
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "                if let Some(eoi) = eoi {\n                    dismiss(eoi);\n                }",
        replace: "                if let Some(eoi) = eoi {\n                    drop(eoi);\n                }",
        breaks: "the ping interrupt arm drops its completion authority without issuing EOI",
        reddens: Caught::Check("the interrupt is dismissed, so the next one arrives"),
    },
    Mutation {
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "    drop(eoi);\n    let reclaimed = apic::claim_eoi(frame, vector::PING);",
        replace: "    core::mem::forget(eoi);\n    let reclaimed = apic::claim_eoi(frame, vector::PING);",
        breaks: "abandoning an EOI token can permanently strand its live-claim latch",
        reddens: Caught::Check("an abandoned completion can be reclaimed for the same delivery"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "impl Drop for EoiToken<'_> {\n    fn drop(&mut self) {\n        EOI_CLAIMED.store(false, Ordering::Release);",
        replace: "impl Drop for EoiToken<'_> {\n    fn drop(&mut self) {\n        EOI_CLAIMED.store(true, Ordering::Release);",
        breaks: "dropping an EOI token leaves its completion latch claimed",
        reddens: Caught::Panic {
            marker: "vector 0x41 has no matching APIC in-service authority",
            why: "the leaked EOI claim prevents the next ping from owning its in-service source",
        },
    },
    Mutation {
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            dismiss(eoi);\n            crate::thread::reschedule(frame);",
        replace: "            drop(eoi);\n            crate::thread::reschedule(frame);",
        breaks: "the timer interrupt arm drops its completion authority without issuing EOI",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::every_interrupt_arm_has_explicit_completion_ownership",
        },
    },
    Mutation {
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            let woke = crate::input::on_interrupt();\n            dismiss(eoi);",
        replace: "            let woke = crate::input::on_interrupt();\n            drop(eoi);",
        breaks: "the keyboard interrupt arm drops its completion authority without issuing EOI",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::every_interrupt_arm_has_explicit_completion_ownership",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/lib.rs",
        find: "        if is_canonical(address) {",
        replace: "        if true {",
        breaks: "a safe page-invalidation address can cross either noncanonical hole boundary",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::canonical_address_admission_covers_both_sides_of_the_hole",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/activation.rs",
        find: "            2 => Some(Self::Ready),",
        replace: "            2 => None,",
        breaks: "the published paging-ready byte has no protocol state",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "activation::tests::every_byte_decodes_to_one_state_or_refusal",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/activation.rs",
        find: "            Self::Initializing => Some(Self::Ready),",
        replace: "            Self::Initializing => None,",
        breaks: "paging activation cannot settle its initializing transaction as ready",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "activation::tests::only_the_protocol_edges_are_admitted",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let interrupts = Some(interrupt::disable());",
        replace: "        let interrupts = None;",
        breaks: "paging activation publishes across an interruptible partial transaction",
        reddens: Caught::Check("paging activation stays unpublished under interrupt exclusion"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            Ok(_) => {\n                let activation = Self {",
        replace: "            Ok(_) => {\n                state.store(ActivationState::Ready.bits(), Ordering::Release);\n                let activation = Self {",
        breaks: "paging activation publishes readiness before its CR3 transaction commits",
        reddens: Caught::Check("paging activation stays unpublished under interrupt exclusion"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        this.root.store(root, Ordering::Release);\n        after_root();\n        let Some(ready) = ActivationState::Initializing.complete() else {\n            panic!(\"paging initialization cannot complete\");\n        };\n        this.committed = true;\n        this.state.store(ready.bits(), Ordering::Release);",
        replace: "        let Some(ready) = ActivationState::Initializing.complete() else {\n            panic!(\"paging initialization cannot complete\");\n        };\n        this.state.store(ready.bits(), Ordering::Release);\n        this.root.store(root, Ordering::Release);\n        after_root();\n        this.committed = true;",
        breaks: "paging activation publishes readiness before its active root",
        reddens: Caught::Check("paging activation publishes its root before readiness"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    unsafe { cpu::set_page_table_base(root) };\n    activation.commit(root);",
        replace: "    activation.commit(root);\n    unsafe { cpu::set_page_table_base(root) };",
        breaks: "paging activation publishes readiness before loading its page-table root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::paging_activation_loads_cr3_before_publishing_ready",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if space.is_reclaimed() {",
        replace: "    if false && space.is_reclaimed() {",
        breaks: "paging activation reads a root from an already reclaimed address space",
        reddens: Caught::Check("a reclaimed space answers nothing"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "pub fn invalidate_page(address: CanonicalAddress) {\n    // SAFETY: construction of `CanonicalAddress` proves the instruction's\n    // operand is canonical. `invlpg` dereferences no memory and only evicts\n    // this CPU's cached translation.\n    unsafe {\n        core::arch::asm!(\"invlpg [{}]\", in(reg) address.get(), options(nostack, preserves_flags))\n    };\n    #[cfg(feature = \"selftest\")]\n    PAGE_INVALIDATIONS.fetch_add(1, Ordering::Relaxed);\n}",
        replace: "pub trait InvalidationAddress {\n    /// Expose the instruction operand.\n    fn raw(self) -> u64;\n}\n\nimpl InvalidationAddress for CanonicalAddress {\n    fn raw(self) -> u64 {\n        self.get()\n    }\n}\n\nimpl InvalidationAddress for u64 {\n    fn raw(self) -> u64 {\n        self\n    }\n}\n\n/// Drop any cached translation for an admitted address from this CPU's TLB.\npub fn invalidate_page(address: impl InvalidationAddress) {\n    let raw = address.raw();\n    // SAFETY: the weakened mutation admits this operand without validation.\n    unsafe {\n        core::arch::asm!(\"invlpg [{}]\", in(reg) raw, options(nostack, preserves_flags))\n    };\n    #[cfg(feature = \"selftest\")]\n    PAGE_INVALIDATIONS.fetch_add(1, Ordering::Relaxed);\n}",
        breaks: "the INVLPG door accepts a raw noncanonical address",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::invlpg_requires_canonical_address_authority",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: "    let raw = an_admitted_canonical_page_can_be_invalidated as *const () as u64;",
        replace: "    let raw = 0x0000_8000_0000_0000;",
        breaks: "the production page-invalidation wrapper receives a noncanonical raw operand",
        reddens: Caught::Check("an admitted canonical page can be invalidated"),
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "            Some(restore) => Err(ProbeFailure {\n                reason: failure.reason,\n                retry: Some(ProbeRetry { probe, restore }),\n            }),",
        replace: "            Some(restore) => Err(ProbeFailure {\n                reason: failure.reason,\n                retry: None,\n            }),",
        breaks: "an uncertain BAR restoration returns no exclusive retry authority",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_uncertain_bar_restore_retains_exclusion_and_retries_exactly",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "                Ok(bits) if bits == restore.original[index] => {}\n                Ok(_) => return Err((ProbeError::RestoreChanged, restore)),",
        replace: "                Ok(_) => {}",
        breaks: "BAR restoration accepts a write the device ignored",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::ignored_bar_and_command_restore_writes_retain_retry_authority",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        Ok(bits) if bits as u16 == restore.command => Ok(()),\n        Ok(_) => Err((ProbeError::RestoreChanged, restore)),",
        replace: "        Ok(_) => Ok(()),",
        breaks: "command restoration accepts a write the device ignored",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::ignored_bar_and_command_restore_writes_retain_retry_authority",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "            Ok(bits) if bits as u16 == decode_off => {}\n            Ok(_) => return Err((ProbeError::RestoreChanged, restore)),",
        replace: "            Ok(_) => {}",
        breaks: "a BAR retry restores windows without reproving that decode stayed off",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_retry_reproves_decode_is_off_before_restoring_bars",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        Ok(bits) if bits as u16 == decode_off => {}\n        Ok(bits) => {\n            return Err(after_probe_error(\n                ProbeError::Refused(bits as u16),\n                &mut access,\n                command_restore,\n            ));\n        }",
        replace: "        Ok(_) => {}",
        breaks: "a BAR probe accesses windows after the device ignored decode-off",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_ignored_decode_off_write_is_refused_before_any_bar_access",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "            Some(restore) => Err(ProbeFailure {\n                reason: failure.reason,\n                retry: Some(ProbeRetry { probe, restore }),\n            }),",
        replace: "            Some(restore) => {\n                let _released = leases.release(probe.0);\n                Err(ProbeFailure {\n                    reason: failure.reason,\n                    retry: Some(ProbeRetry { probe, restore }),\n                })\n            }",
        breaks: "an uncertain BAR retry token releases its exclusion lease before return",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_uncertain_bar_restore_retains_exclusion_and_retries_exactly",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/pci.rs",
        find: "    let NvmeBusMasterAuthority { bar, _observed: _ } = authority;\n    let mut configuration = CONFIGURATION.lock_leaf();",
        replace: "    let NvmeBusMasterAuthority { bar, _observed: _ } = authority;\n    let mut configuration = CONFIGURATION\n        .try_lock_leaf()\n        .expect(\"the configuration port pair is available\");",
        breaks: "one PCI command transaction does not hold the configuration port pair exactly once",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_command_transactions_hold_the_port_pair_once",
        },
    },
    Mutation {
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
        file: "xtask/src/tasks.rs",
        find: r#"    let canonical_directory = crate::mutation_workspace::canonical_existing_real_directory(
        directory,
        "VMware cleanup directory",
    )?;"#,
        replace: r#"    let canonical_directory = std::fs::canonicalize(directory)
        .with_context(|| format!("canonicalizing {}", directory.display()))?;"#,
        breaks: "recursive cleanup follows a redirected VMware directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::recursive_vm_cleanup_refuses_redirected_paths_without_touching_the_target",
        },
    },
    Mutation {
        file: "xtask/src/tasks.rs",
        find: r#"    let canonical_directory = crate::mutation_workspace::canonical_existing_real_directory(
        directory,
        "VMware cleanup directory",
    )?;"#,
        replace: r#"    let metadata = std::fs::symlink_metadata(directory)
        .with_context(|| format!("inspecting {}", directory.display()))?;
    ensure!(
        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
        "VMware cleanup directory is not a real directory"
    );
    let canonical_directory = std::fs::canonicalize(directory)
        .with_context(|| format!("canonicalizing {}", directory.display()))?;"#,
        breaks: "recursive cleanup accepts a real final directory behind a redirected repository ancestor",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tasks::tests::recursive_vm_cleanup_refuses_redirected_paths_without_touching_the_target",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        let directory = crate::image::qemu_run_directory(target)?;",
        replace: r#"        let directory = target.join(crate::paths::QEMU_RUN_DIRECTORY);
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("creating {}", directory.display()))?;"#,
        breaks: "private firmware can be created through a redirected QEMU run root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::private_firmware_refuses_a_redirected_run_root_before_writing",
        },
    },
    Mutation {
        file: "xtask/src/image/mod.rs",
        find: r#"pub(crate) fn qemu_run_directory(target: &Path) -> Result<PathBuf> {
    crate::mutation_workspace::prepare_real_directory_tree(
        &target.join(paths::QEMU_RUN_DIRECTORY),
        "QEMU private run directory",
    )
}"#,
        replace: r#"pub(crate) fn qemu_run_directory(target: &Path) -> Result<PathBuf> {
    let directory = target.join(paths::QEMU_RUN_DIRECTORY);
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("creating {}", directory.display()))?;
    Ok(directory)
}"#,
        breaks: "private boot images can be created through a redirected QEMU run root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::private_qemu_images_refuse_a_redirected_run_root_before_writing",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"    crate::mutation_workspace::prepare_real_directory_tree(&parent, "QEMU storage directory")?;"#,
        replace: r#"    std::fs::create_dir_all(&parent)
        .with_context(|| format!("creating {}", parent.display()))?;"#,
        breaks: "QEMU storage can be created through a redirected generated-state parent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::storage_creation_refuses_a_redirected_parent_before_writing",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"        crate::mutation_workspace::publish_staged_file(
            parent,
            &staging,
            backing,
            "NVMe image",
            |published| {
                let digest =
                    crate::sha256::digest_reader(File::open(published).with_context(|| {
                        format!("opening published NVMe image {}", published.display())
                    })?)
                    .with_context(|| {
                        format!("hashing published NVMe image {}", published.display())
                    })?;
                ensure!(
                    digest == expected,
                    "published NVMe image differs from its staged bytes"
                );
                Ok(())
            },
        )?;"#,
        replace: r#"        let _unchecked_digest = expected;
        std::fs::copy(&staging, backing)
            .with_context(|| format!("overwriting {}", backing.display()))?;"#,
        breaks: "NVMe publication writes staged bytes through an external hardlink",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::storage_publication_never_writes_through_an_external_link",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"        let mut options = OpenOptions::new();
        OpenOptions::create_new(&mut options, true);
        OpenOptions::read(&mut options, true);
        OpenOptions::write(&mut options, true);
        let mut file = OpenOptions::open(&options, &staging)"#,
        replace: r#"        let mut options = OpenOptions::new();
        OpenOptions::create_new(&mut options, true);
        OpenOptions::write(&mut options, true);
        let mut file = OpenOptions::open(&options, &staging)"#,
        breaks: "the production NVMe staging handle cannot support its read-modify-write formatter",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::storage_staging_supports_read_modify_write_builders",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"fn require_storage_lock_path(parent: &Path, path: &Path, must_exist: bool) -> Result<()> {
    ensure!(
        path == parent.join("nvme.lock"),
        "storage build lock is not the exact allowlisted path"
    );
    let present = crate::mutation_workspace::require_direct_file_if_present(
        parent,
        path,
        "storage build lock",
    )?;
    ensure!(
        !must_exist || present,
        "storage build lock disappeared after it was opened: {}",
        path.display()
    );
    Ok(())
}"#,
        replace: r#"fn require_storage_lock_path(parent: &Path, path: &Path, must_exist: bool) -> Result<()> {
    let _unchecked_storage_lock = (parent, path, must_exist);
    Ok(())
}"#,
        breaks: "a QEMU storage lock can resolve to an external linked inode",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::storage_publication_never_writes_through_an_external_link",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    after_open(path);\n    require_storage_lock_path(parent, path, true)?;",
        replace: "    after_open(path);\n    require_storage_lock_path(parent, path, false)?;",
        breaks: "an opened QEMU storage lock remains named before serialization begins",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::an_open_storage_lock_must_remain_at_its_name",
        },
    },
    Mutation {
        file: "xtask/src/image/mod.rs",
        find: r#"    let canonical_parent = crate::mutation_workspace::prepare_real_directory_tree(
        parent,
        "boot-image target directory",
    )?;"#,
        replace: r#"    let canonical_parent = std::fs::canonicalize(parent)
        .with_context(|| format!("canonicalizing {}", parent.display()))?;"#,
        breaks: "an authoritative boot image can be published through a redirected target directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::authoritative_images_refuse_a_redirected_target_before_writing",
        },
    },
    Mutation {
        file: "xtask/src/image/mod.rs",
        find: r#"        crate::mutation_workspace::publish_staged_file(
            &canonical_parent,
            &staging,
            &published_path,
            "boot image",
            |published| {
                let digest =
                    crate::sha256::digest_reader(File::open(published).with_context(|| {
                        format!("opening boot image {}", destination.display())
                    })?)
                    .with_context(|| format!("hashing boot image {}", destination.display()))?;
                ensure!(
                    digest == staged,
                    "published boot image differs from its complete stage"
                );
                Ok(())
            },
        )?;"#,
        replace: r#"        let _unchecked_digest = staged;
        std::fs::copy(&staging, &published_path)
            .with_context(|| format!("overwriting {}", destination.display()))?;"#,
        breaks: "boot-image publication writes staged bytes through an external hardlink",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::authoritative_image_publication_never_writes_through_an_external_link",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: r#"        let (_, destination) =
            prepare_confined_destination(&root, relative, "mutation copy destination")?;"#,
        replace: r#"        let destination = root.join(relative);
        std::fs::create_dir_all(
            destination.parent().context("mutation copy destination has no parent")?,
        )?;"#,
        breaks: "initial mutation copy creates nested parents before proving confinement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::initial_copy_refuses_a_nested_redirect_before_creating_outside_paths",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
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
            name: "mutation_workspace::tests::restore_refuses_same_byte_external_state_before_its_fast_path",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
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
            name: "mutation_workspace::tests::a_failed_source_publication_restores_the_previous_authority",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
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
            name: "mutation_workspace::tests::failed_stage_cleanup_rejects_a_hostile_stage_replacement",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
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
            name: "mutation_workspace::tests::failed_stage_cleanup_reports_both_operation_and_removal_failures",
        },
    },
    Mutation {
        file: "xtask/src/supply_chain.rs",
        find: r#"                digest: package.checksum.clone().with_context(|| {
                    format!(
                        "registry package {} {} has no validated checksum",
                        package.name, package.version
                    )
                })?,"#,
        replace: "                digest: package.checksum.clone().unwrap_or_default(),",
        breaks: "a registry package without a validated checksum enters the inventory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::registry_inventory_projection_refuses_a_missing_checksum",
        },
    },
    Mutation {
        file: "xtask/src/bounded_process.rs",
        find: "        reservation.track(path);",
        replace: "        let _untracked_capture = (reservation, path);",
        breaks: "a second capture-stream failure leaks the first file and its reservation directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "bounded_process::tests::a_second_stream_failure_rolls_back_the_first_capture_and_directory",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"        return Err(error).context("terminating QEMU");"#,
        replace: "        return Ok(());",
        breaks: "a live QEMU child after refused termination is accepted as stopped",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::qemu_stop_refuses_timeout_and_a_live_child_after_kill_refusal",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"        if exited(child).context("polling terminated QEMU")? {
            return Ok(());
        }"#,
        replace: r#"        if false && exited(child).context("polling terminated QEMU")? {
            return Ok(());
        }"#,
        breaks: "QEMU teardown never observes the terminated direct child",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::qemu_stop_polls_until_a_terminated_child_is_reaped",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"            bail!(
                "QEMU did not exit within {} ms after termination",
                timeout.as_millis()
            );"#,
        replace: "            return Ok(());",
        breaks: "QEMU teardown reports success when the terminated child never exits",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::qemu_stop_refuses_timeout_and_a_live_child_after_kill_refusal",
        },
    },
    Mutation {
        file: "xtask/src/mutate.rs",
        find: "            if node.method == \"check\" && report_receiver {",
        replace: "            if node.method == \"check\" {",
        breaks: "a decoy method call can forge a current machine-check identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::machine_check_inventory_rejects_decoys_and_nonliteral_names",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let Some(current) = current_mapping_root(self.root) else {\n            return false;\n        };",
        replace: "        if !is_active() || active_root() != self.root {\n            return false;\n        }\n        let current = self.root;",
        breaks: "a device token authenticates its recorded root instead of the root actually loaded",
        reddens: Caught::Check("a stale shared root cannot authenticate a device mapping"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        current_mapping_root(self.identity.root)\n            .is_some_and(|current| self.identity.invalid_page_in(current).is_none())",
        replace: "        is_active()\n            && active_root() == self.identity.root\n            && self.identity.invalid_page_in(self.identity.root).is_none()",
        breaks: "a stack token authenticates its recorded root instead of the root actually loaded",
        reddens: Caught::Check("a stale shared root cannot authenticate a stack"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let mut stack = self.slot.stack_base();",
        replace: "        let mut stack = self.slot.stack_base() + PAGE_SIZE;",
        breaks: "a retained stack identity skips its first leaf",
        reddens: Caught::Check("the boot stack rejects a lower-leaf substitution"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    pub(crate) fn stack_top(&self, index: usize) -> Option<KernelStackTop<'_>> {\n        let identity = self.stacks.get(index)?;\n        if !is_active() {\n            return None;\n        }\n        let current = cpu::page_table_base();\n        identity\n            .invalid_page_in(current)\n            .is_none()\n            .then_some(KernelStackTop(identity))\n    }",
        replace: "    pub(crate) fn stack_top(&self, index: usize) -> Option<KernelStackTop<'_>> {\n        self.stacks.get(index).map(KernelStackTop)\n    }",
        breaks: "a hardware-use stack pointer is derived without rewalking its retained identity",
        reddens: Caught::Check("a noncurrent task stack rejects a lower-leaf substitution"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            match walk(root, stack) {\n                Some((found, flags, span))\n                    if page_identity_matches(found, flags, span, physical, expected_flags) => {}\n                _ => return Some(stack),\n            }",
        replace: "            if offset != PAGE_SIZE {\n                match walk(root, stack) {\n                    Some((found, flags, span))\n                        if page_identity_matches(found, flags, span, physical, expected_flags) => {}\n                    _ => return Some(stack),\n                }\n            }",
        breaks: "a retained stack identity skips its middle leaf",
        reddens: Caught::Check("a stack checks its middle leaf"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        while stack < self.slot.stack_top() {",
        replace: "        while stack < self.slot.stack_top() - PAGE_SIZE {",
        breaks: "a retained stack identity skips its last leaf",
        reddens: Caught::Check("a stack checks its last leaf"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let mut offset = 0u64;",
        replace: "        let mut offset = PAGE_SIZE;",
        breaks: "a retained device identity skips its first leaf",
        reddens: Caught::Check("a device mapping checks its first leaf"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            let Some((mapped, flags, span)) = walk(current, virtual_address) else {\n                return false;\n            };\n            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {",
        replace: "            let Some((mapped, flags, span)) = walk(current, virtual_address) else {\n                return false;\n            };\n            let expected = if offset == PAGE_SIZE {\n                mapped\n            } else {\n                expected\n            };\n            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {",
        breaks: "a retained device identity trusts its middle physical leaf",
        reddens: Caught::Check("a device mapping checks its middle leaf"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            let Some((mapped, flags, span)) = walk(current, virtual_address) else {\n                return false;\n            };\n            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {",
        replace: "            let Some((mapped, flags, span)) = walk(current, virtual_address) else {\n                return false;\n            };\n            let expected = if offset.checked_add(PAGE_SIZE) == Some(self.pages) {\n                mapped\n            } else {\n                expected\n            };\n            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {",
        breaks: "a retained device identity trusts its last physical leaf",
        reddens: Caught::Check("a device mapping checks its last leaf"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {",
        replace: "            if !page_identity_matches(mapped, flags, span, expected, flags) {",
        breaks: "a retained device identity trusts the cache policy it observes",
        reddens: Caught::Check("a device mapping checks its cache policy"),
    },
    Mutation {
        file: "libs/x86-paging/src/lib.rs",
        find: "        && span == PAGE_SIZE",
        replace: "        && span != 0",
        breaks: "page identity accepts a non-page leaf span",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::page_identity_ignores_only_hardware_observation_bits",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "    unsafe { extended.write_bytes(0, xstate::CEILING) };",
        replace: "    unsafe { extended.write_bytes(0, 0) };",
        breaks: "a recycled task slot retains its previous extended-state bytes",
        reddens: Caught::Check("an exited slot is reused"),
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: "    let written = xstate::write_probe(REUSED_SLOT_SECRET);",
        replace: "    let written = false;",
        breaks: "the task-slot reuse witness accepts a secret that was never planted",
        reddens: Caught::Check("an exited slot is reused"),
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\npub struct DmaState {",
        replace: "#[derive(Debug, Clone, PartialEq, Eq)]\npub struct DmaState {",
        breaks: "DMA ownership authority can be duplicated before publication or completion",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "            if end > bytes {",
        replace: "            if end >= bytes {",
        breaks: "a publication word ending exactly at the allocation boundary is refused",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::publication_words_exhaust_every_small_offset",
        },
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "            let word_end = word + PUBLICATION_WORD_BYTES;",
        replace: "            let word_end = word;",
        breaks: "a payload byte can overlap the beginning of a publication word",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::payload_exhausts_touching_overlapping_and_outside_ranges",
        },
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "            if !at.is_multiple_of(align_of::<u32>()) {",
        replace: "            if false && !at.is_multiple_of(align_of::<u32>()) {",
        breaks: "a publication word can begin at a misaligned byte offset",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::malformed_publication_partitions_are_refused_exactly",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "impl Shared {\n    /// Consume the allocation into volatile byte-only access.",
        replace: "impl Shared {\n    /// Write through the unpartitioned shared owner.\n    #[must_use]\n    pub fn write_volatile_at(&mut self, offset: usize, bytes: &[u8]) -> bool {\n        write_shared_volatile(&self.0, offset, bytes)\n    }\n\n    /// Consume the allocation into volatile byte-only access.",
        breaks: "an unpartitioned shared allocation retains a raw byte-write door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::shared_memory_access_requires_one_partition",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        if !self.layout.contains_payload(offset, bytes.len()) {",
        replace: "        if false && !self.layout.contains_payload(offset, bytes.len()) {",
        breaks: "a shared ring payload write can enter its publication-word partition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::every_shared_payload_copy_is_volatile",
        },
    },
    Mutation {
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        write(&mut self.registers, low, entry::MASKED)?;\n        let (observed_low, observed_high) = self.entry_words(line)?;\n        if observed_low != entry::MASKED {\n            return Err(MaskError::Refused {\n                low: observed_low,\n                high: observed_high,\n            });\n        }\n        write(&mut self.registers, low + 1, 0)?;",
        replace: "        write(&mut self.registers, low, entry::MASKED)?;\n        write(&mut self.registers, low + 1, 0)?;",
        breaks: "an unproven takeover mask permits a destination write",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        if low != entry::MASKED || high != 0 {\n            return Err(MaskError::Refused { low, high });\n        }",
        replace: "        let _ = (low, high);",
        breaks: "an ignored takeover mask write is accepted",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "            Ok((low, high)) if low == expected_low && high == destination => Ok(()),\n            Ok((low, high)) => self.refuse_route(line, expected_low, destination, low, high),",
        replace: "            Ok((_low, _high)) => Ok(()),",
        breaks: "an ignored unmask write is accepted",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        match self.entry_words(line) {\n            Ok((low, high)) if low == entry::MASKED && high == destination => {}\n            Ok((low, high)) => {\n                return self.refuse_route(line, expected_low, destination, low, high);\n            }\n            Err(error) => {\n                self.mask_line_and_verify(line)\n                    .map_err(RoutingError::from)?;\n                return Err(RoutingError::Registers(error));\n            }\n        }",
        replace: "        let _ = self.entry_words(line);",
        breaks: "an IOAPIC destination is unmasked before its readback",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        file: "kernel/src/pci.rs",
        find: "        Ok(()) => release_clean_function(function, reason),",
        replace: "        Ok(()) => {\n            drop(function);\n            reason\n        },",
        breaks: "a clean post-claim PCI refusal releases its function authority",
        reddens: Caught::Check("a device is claimed once and a refusal strands nothing"),
    },
    Mutation {
        file: "libs/pixel/src/lib.rs",
        find: "        Some(RowCopyPlan {\n            source_word,\n            destination_word,\n            destination_offset,\n            row_words: self.width,\n            stride_words,",
        replace: "        Some(RowCopyPlan {\n            source_word,\n            destination_word,\n            destination_offset,\n            row_words: stride_words,\n            stride_words,",
        breaks: "a padded row copy publishes the row padding as visible pixels",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::row_copy_uses_padded_starts_without_publishing_padding",
        },
    },
    Mutation {
        file: "libs/pixel/src/lib.rs",
        find: "        if to_y > from_y {",
        replace: "        if to_y < from_y {",
        breaks: "a framebuffer row-copy plan admits downward overlap and refuses upward movement",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::row_copy_refuses_downward_and_extreme_empty_ranges",
        },
    },
    Mutation {
        file: "libs/pixel/src/lib.rs",
        find: "        self.source_end <= len && self.destination_end <= len",
        replace: "        self.destination_end <= len",
        breaks: "a row copy can read beyond a short framebuffer shadow",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::row_copy_refuses_a_short_shadow_without_side_effects",
        },
    },
    Mutation {
        file: "libs/pixel/src/lib.rs",
        find: "            self.destination_offset += self.pitch;",
        replace: "            self.destination_offset += self.row_words * WORD_SIZE;",
        breaks: "a padded row copy advances device writes by visible width instead of pitch",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::every_small_row_copy_stays_within_its_padded_layout",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "/// One ordered boundary in the hardware transaction.\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum InitStage {",
        replace: "impl Interrupts {\n    pub const fn from_enabled(enabled: bool) -> Self {\n        if enabled { Self::Enabled } else { Self::Excluded }\n    }\n}\n\n/// One ordered boundary in the hardware transaction.\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum InitStage {",
        breaks: "APIC interrupt-exclusion policy regains a boolean constructor",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_policy_observations_are_typed",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/in_service.rs",
        find: "pub const fn may_complete(\n    interrupts: Interrupts,\n    frame_liveness: FrameLiveness,\n    frame_vector: u64,\n    token_vector: InterruptVector,\n    dismissal: DismissalOwnership,\n) -> bool {\n    matches!(interrupts, Interrupts::Excluded)\n        && matches!(frame_liveness, FrameLiveness::Live)\n        && frame_vector == token_vector.get() as u64\n        && matches!(dismissal, DismissalOwnership::Owned)\n}",
        replace: "pub const fn may_complete(\n    interrupts_excluded: bool,\n    frame_is_live: bool,\n    frame_vector: u64,\n    token_vector: InterruptVector,\n    dismissal_owned: bool,\n) -> bool {\n    interrupts_excluded\n        && frame_is_live\n        && frame_vector == token_vector.get() as u64\n        && dismissal_owned\n}",
        breaks: "APIC completion collapses three independent observations into positional booleans",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_policy_observations_are_typed",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "    fn published_vector(self, source: &AtomicU8) -> Option<InterruptVector> {\n        if !self.is_enabled() {\n            return None;\n        }\n        InterruptVector::new(source.load(Ordering::Relaxed))",
        replace: "    fn published_vector(self, source: &AtomicU8) -> Option<InterruptVector> {\n        let vector = source.load(Ordering::Relaxed);\n        if !self.is_enabled() {\n            return None;\n        }\n        InterruptVector::new(vector)",
        breaks: "an APIC consumer reads a dependent vector before acquiring Ready",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_ready_publication_precedes_dependent_vector_reads",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "        let Some(vector) = self.published_vector(self.self_interrupt_vector) else {\n            return Err(DeliveryError::Unavailable);\n        };",
        replace: "        let vector = self.self_interrupt_vector.load(Ordering::Relaxed);",
        breaks: "self-interrupt delivery reads its vector outside ordered Ready publication",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_ready_publication_precedes_dependent_vector_reads",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "        self.controller.state.store(ready.bits(), Ordering::Release);\n        if let Some(interrupts) = self.interrupts.take() {\n            interrupts.restore();\n        }",
        replace: "        if let Some(interrupts) = self.interrupts.take() {\n            interrupts.restore();\n        }\n        self.controller.state.store(ready.bits(), Ordering::Release);",
        breaks: "APIC readiness publishes after initialization interrupt exclusion unwinds",
        reddens: Caught::Check("apic refusal defers a live interrupt until exclusion unwinds"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "            self.controller\n                .state\n                .store(self.transaction.refuse().bits(), Ordering::Release);",
        replace: "            self.controller\n                .state\n                .store(Lifecycle::Cold.bits(), Ordering::Release);",
        breaks: "a failed live APIC transaction becomes retryable after its owner drops",
        reddens: Caught::Check(
            "every late or ignored-write apic refusal stays unavailable and terminal",
        ),
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "    InitializationRegisters::write(\n        &registers,\n        register::SPURIOUS_INTERRUPT_VECTOR,\n        spurious.bits().into(),\n    );\n    if InitializationRegisters::read(&registers, register::SPURIOUS_INTERRUPT_VECTOR)\n        != u64::from(spurious.bits())\n    {\n        return Err(ApicError::NotResponding);\n    }",
        replace: "    InitializationRegisters::write(\n        &registers,\n        register::SPURIOUS_INTERRUPT_VECTOR,\n        spurious.bits().into(),\n    );",
        breaks: "an ignored APIC software-enable write can publish readiness",
        reddens: Caught::Check(
            "every late or ignored-write apic refusal stays unavailable and terminal",
        ),
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "        self.lifecycle().is_ready().then_some(Registers)",
        replace: "        matches!(self.lifecycle(), Lifecycle::Initializing | Lifecycle::Ready)\n            .then_some(Registers)",
        breaks: "the central APIC register gate admits an unpublished initializing controller",
        reddens: Caught::Check(
            "local apic initialization stayed unpublished and interrupt-excluded",
        ),
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        let Ok(range) = AccessRange::repeated(offset, bytes.len(), 1, self.len) else {",
        replace: "        let Ok(range) =\n            AccessRange::repeated(offset, bytes.len(), 1, self.len.saturating_sub(1))\n        else {",
        breaks: "DMA byte writes refuse the exact final byte of their owned run",
        reddens: Caught::Check("dma byte access is exact and bounded"),
    },
    Mutation {
        file: "libs/trap-cause/src/lib.rs",
        find: "        self.0 & !0b11 == 0",
        replace: "        self.0 & !0b111 == 0",
        breaks: "a user-origin page fault is classified as supervisor data access",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::only_exact_supervisor_data_faults_are_admitted",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/lib.rs",
        find: "    pub const ZERO: Self = Self(0);",
        replace: "    pub const ZERO: Self = Self(1);",
        breaks: "the canonical zero-address authority names a nonzero byte",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::canonical_address_admission_covers_both_sides_of_the_hole",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "    pub const fn none() -> Self {\n        Self(0)\n    }",
        replace: "    pub const fn none() -> Self {\n        Self(1 << 11)\n    }",
        breaks: "an empty CR4 protection update silently enables UMIP",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::cr4_updates_are_add_only_and_exact",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "        Self(self.0 | 1 << 20)",
        replace: "        Self(self.0 | 1 << 21)",
        breaks: "the SMEP builder enables SMAP instead of execution prevention",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::cr4_updates_are_add_only_and_exact",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "        Self(self.0 | 1 << 21)",
        replace: "        Self(self.0 | 1 << 20)",
        breaks: "the SMAP builder enables SMEP instead of access prevention",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::cr4_updates_are_add_only_and_exact",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "        Self(self.0 | 1 << 11)",
        replace: "        Self(self.0 | 1 << 21)",
        breaks: "the UMIP builder enables SMAP instead of instruction prevention",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::cr4_updates_are_add_only_and_exact",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "#[cfg(feature = \"selftest\")]\n#[must_use = \"a refused resume changed no trap frame\"]\npub(super) fn resume_write_protection(",
        replace: "#[cfg(feature = \"selftest\")]\n#[must_use = \"a refused write-protection resume changed no trap frame\"]\npub(super) fn resume_write_protection(",
        breaks: "the write-protection recovery door loses its pinned must-use contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::fallible_operation_bools_stay_must_use",
        },
    },
    Mutation {
        file: "docs/017-pre-wizzy-hostile-surface-audit.md",
        find: "| `x86-pit` | PIT counter/status samples and watchdog observations |",
        replace: "| `x86-pit-omitted` | PIT counter/status samples and watchdog observations |",
        breaks: "the hostile-surface audit omits a host foundation library",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "boundary::tests::every_foundation_library_has_an_explicit_boundary_policy",
        },
    },
    Mutation {
        file: "libs/trap-cause/src/lib.rs",
        find: "        self.vector == u64::from(exception::PAGE_FAULT)",
        replace: "        self.vector <= u64::from(exception::PAGE_FAULT)",
        breaks: "controlled recovery accepts a non-page-fault vector",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::recovery_requires_the_page_fault_vector",
        },
    },
    Mutation {
        file: "libs/trap-cause/src/lib.rs",
        find: "            && self.frame_error_code == self.page_fault.bits()",
        replace: "            && self.frame_error_code == self.frame_error_code",
        breaks: "controlled recovery does not bind the saved error word to its decoded fault",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::recovery_requires_the_error_code_the_dispatcher_decoded",
        },
    },
    Mutation {
        file: "libs/trap-cause/src/lib.rs",
        find: "            && self.code_selector & 0b11 == 0",
        replace: "            && self.code_selector & 0b11 != 3",
        breaks: "controlled recovery admits ring-one and ring-two trap frames",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::recovery_requires_a_ring_zero_frame",
        },
    },
    Mutation {
        file: "libs/trap-cause/src/lib.rs",
        find: "            RecoveryFault::SupervisorData => self.page_fault.is_supervisor_data_access(),",
        replace: "            RecoveryFault::SupervisorData => true,",
        breaks: "user-copy recovery accepts every page-fault class",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::user_copy_recovery_accepts_only_supervisor_data_faults",
        },
    },
    Mutation {
        file: "libs/trap-cause/src/lib.rs",
        find: "            RecoveryFault::SupervisorWriteProtection => self.page_fault.bits() == 0b11,",
        replace: "            RecoveryFault::SupervisorWriteProtection => {\n                self.page_fault.is_supervisor_data_access()\n            }",
        breaks: "write-protection recovery accepts missing and read faults",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::write_protection_recovery_requires_the_exact_protection_write",
        },
    },
    Mutation {
        file: "libs/trap-cause/src/lib.rs",
        find: "            && rule.fault_addresses.contains(&self.faulting_address)",
        replace: "            && self.faulting_address < rule.fault_addresses.end",
        breaks: "controlled recovery ignores the lower bound and emptiness of its CR2 interval",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::recovery_requires_cr2_inside_the_owned_interval",
        },
    },
    Mutation {
        file: "libs/trap-cause/src/lib.rs",
        find: "            && rule\n                .instruction_addresses\n                .contains(&self.instruction_pointer)",
        replace: "            && self.instruction_pointer < rule.instruction_addresses.end",
        breaks: "controlled recovery ignores the lower bound and emptiness of its RIP interval",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::recovery_requires_rip_inside_the_owned_interval",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "        frame_error_code: frame.error_code(),",
        replace: "        frame_error_code: frame.vector(),",
        breaks: "the user-copy adapter substitutes the trap vector for the saved error word",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::each_recovery_adapter_binds_the_live_architectural_values",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "    let at = frame.instruction_pointer();",
        replace: "    let mut at = frame.instruction_pointer();",
        breaks: "the user-copy adapter can rewrite its saved live instruction pointer",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::each_recovery_adapter_binds_the_live_architectural_values",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "    let start = core::ptr::addr_of!(wost_copy_fault_start) as u64;",
        replace: "    let start = core::ptr::addr_of!(wost_copy_fault_end) as u64;",
        breaks: "the user-copy recovery window starts at its exclusive end label",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::each_recovery_adapter_binds_the_live_architectural_values",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "        faulting_address: faulting_address(),",
        replace: "        faulting_address: target,",
        breaks: "the write-protection adapter substitutes its expected target for live CR2",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::each_recovery_adapter_binds_the_live_architectural_values",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "target.checked_add(1)",
        replace: "target.wrapping_add(1)",
        breaks: "the write-protection recovery target end can wrap through zero",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::each_recovery_adapter_binds_the_live_architectural_values",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    // SAFETY: as above. Only `usercopy` can construct the target, from its\n    // in-crate fixup label, so `iretq` cannot be redirected by another caller.\n    unsafe { core::ptr::write(slot, target.address()) };",
        replace: "    // SAFETY: this mutation writes a value already read from the live frame.\n    unsafe { core::ptr::write(slot, frame.instruction_pointer()) };",
        breaks: "the user-copy resume writer ignores its opaque fixup authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::every_instruction_pointer_writer_requires_its_opaque_authority",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    LIVE_FRAME.load(Ordering::Relaxed) == frame.origin",
        replace: "    LIVE_FRAME.load(Ordering::Relaxed) != frame.origin",
        breaks: "live-frame authority authenticates every origin except the active one",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::every_instruction_pointer_writer_refuses_a_non_live_frame_first",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "pub(super) fn resume_usercopy(frame: &TrapFrame, target: super::usercopy::RecoveryTarget) -> bool {\n    if !is_live_frame(frame) {\n        return false;\n    }",
        replace: "pub(super) fn resume_usercopy(frame: &TrapFrame, target: super::usercopy::RecoveryTarget) -> bool {\n    if false && !is_live_frame(frame) {\n        return false;\n    }",
        breaks: "the user-copy resume writer can reach a stale trap frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::every_instruction_pointer_writer_refuses_a_non_live_frame_first",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "pub(super) fn resume_write_protection(\n    frame: &TrapFrame,\n    target: super::cpu::WriteProtectionRecoveryTarget,\n) -> bool {\n    if !is_live_frame(frame) {\n        return false;\n    }",
        replace: "pub(super) fn resume_write_protection(\n    frame: &TrapFrame,\n    target: super::cpu::WriteProtectionRecoveryTarget,\n) -> bool {\n    if false && !is_live_frame(frame) {\n        return false;\n    }",
        breaks: "the write-protection resume writer can reach a stale trap frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::every_instruction_pointer_writer_refuses_a_non_live_frame_first",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "pub(super) struct RecoveryTarget(u64);",
        replace: "pub(super) struct RecoveryTarget(pub(super) u64);",
        breaks: "the user-copy fixup address field becomes constructible across its parent module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "pub(super) struct RecoveryTarget(u64);",
        replace: "#[derive(Clone)]\npub(super) struct RecoveryTarget(u64);",
        breaks: "user-copy recovery authority gains derived duplication",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "impl RecoveryTarget {\n    fn fixup() -> Self {",
        replace: "impl RecoveryTarget {\n    const ZERO: Self = Self(0);\n\n    fn fixup() -> Self {",
        breaks: "user-copy recovery authority gains an associated raw-address constructor",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "pub(super) struct RecoveryTarget(u64);\n\nimpl RecoveryTarget {",
        replace: "pub(super) struct RecoveryTarget(u64);\n\nconst FORGED_RECOVERY_TARGET: RecoveryTarget = unsafe { core::mem::transmute(0u64) };\n\nimpl RecoveryTarget {",
        breaks: "user-copy recovery authority gains a module-level typed forgery surface",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "pub(super) struct RecoveryTarget(u64);\n\nimpl RecoveryTarget {",
        replace: "pub(super) struct RecoveryTarget(u64);\n\nuse self::RecoveryTarget as ForgedRecoveryTarget;\n\nimpl RecoveryTarget {",
        breaks: "user-copy recovery authority gains an imported constructor alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "pub(super) struct WriteProtectionRecoveryTarget(u64);",
        replace: "pub(super) struct WriteProtectionRecoveryTarget(pub(super) u64);",
        breaks: "the write-protection fixup field becomes constructible across its parent module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)",
        replace: "        Self(core::ptr::addr_of!(wost_copy_fault_end) as u64)",
        breaks: "user-copy recovery authority is minted from the end of the fault window instead of the linker fixup",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "    pub(super) const fn address(&self) -> u64 {\n        self.0\n    }",
        replace: "    pub(super) const fn address(&self) -> u64 {\n        0\n    }",
        breaks: "user-copy recovery authority observes a value other than its private address",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "impl RecoveryTarget {\n    fn fixup() -> Self {\n        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)\n    }\n\n    pub(super) const fn address(&self) -> u64 {\n        self.0\n    }\n}",
        replace: "impl RecoveryTarget {\n    fn fixup() -> Self {\n        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)\n    }\n\n    pub(super) const fn address(&self) -> u64 {\n        self.0\n    }\n}\n\nimpl self::RecoveryTarget {\n    fn from_raw(address: u64) -> Self {\n        Self(address)\n    }\n}",
        breaks: "a qualified inherent impl restores a raw user-copy recovery constructor",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "impl RecoveryTarget {\n    fn fixup() -> Self {\n        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)\n    }\n\n    pub(super) const fn address(&self) -> u64 {\n        self.0\n    }\n}",
        replace: "impl RecoveryTarget {\n    fn fixup() -> Self {\n        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)\n    }\n\n    pub(super) const fn address(&self) -> u64 {\n        self.0\n    }\n}\n\nmod hostile_nested_recovery_constructor {\n    fn from_raw(value: u64) -> super::RecoveryTarget {\n        super::RecoveryTarget(value)\n    }\n}",
        breaks: "a nested module restores a raw user-copy recovery constructor",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: r#"impl RecoveryTarget {
    fn fixup() -> Self {
        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)
    }

    pub(super) const fn address(&self) -> u64 {
        self.0
    }
}

/// Resume a faulting user copy at its fixup, if that is what faulted.
///
/// A mapping may disappear between the walk and copy. Nothing outside the
/// private two-instruction interval is recoverable. `false` leaves the frame
/// unchanged.
#[must_use = "a refused user-copy recovery changed no trap frame"]
pub fn recover(frame: &interrupt::TrapFrame, fault: trap_cause::PageFault) -> bool {
    let at = frame.instruction_pointer();
    let start = core::ptr::addr_of!(wost_copy_fault_start) as u64;
    let end = core::ptr::addr_of!(wost_copy_fault_end) as u64;
    let attempt = trap_cause::RecoveryAttempt {
        vector: frame.vector(),
        frame_error_code: frame.error_code(),
        code_selector: frame.code_selector(),
        instruction_pointer: at,
        page_fault: fault,
        faulting_address: cpu::faulting_address(),
    };
    let rule = trap_cause::RecoveryRule {
        fault: trap_cause::RecoveryFault::SupervisorData,
        fault_addresses: 0..USER_END,
        instruction_addresses: start..end,
    };
    if !attempt.is_admitted_by(&rule) {
        return false;
    }
    interrupt::resume_usercopy(frame, RecoveryTarget::fixup())
}"#,
        replace: r#"impl RecoveryTarget {
    fn fixup() -> Self {
        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)
    }

    pub(super) const fn address(&self) -> u64 {
        self.0
    }
}

fn hostile_resume(frame: &interrupt::TrapFrame) -> bool {
    interrupt::resume_usercopy(frame, RecoveryTarget::fixup())
}

/// Resume a faulting user copy at its fixup, if that is what faulted.
///
/// A mapping may disappear between the walk and copy. Nothing outside the
/// private two-instruction interval is recoverable. `false` leaves the frame
/// unchanged.
#[must_use = "a refused user-copy recovery changed no trap frame"]
pub fn recover(frame: &interrupt::TrapFrame, fault: trap_cause::PageFault) -> bool {
    let at = frame.instruction_pointer();
    let start = core::ptr::addr_of!(wost_copy_fault_start) as u64;
    let end = core::ptr::addr_of!(wost_copy_fault_end) as u64;
    let attempt = trap_cause::RecoveryAttempt {
        vector: frame.vector(),
        frame_error_code: frame.error_code(),
        code_selector: frame.code_selector(),
        instruction_pointer: at,
        page_fault: fault,
        faulting_address: cpu::faulting_address(),
    };
    let rule = trap_cause::RecoveryRule {
        fault: trap_cause::RecoveryFault::SupervisorData,
        fault_addresses: 0..USER_END,
        instruction_addresses: start..end,
    };
    if !attempt.is_admitted_by(&rule) {
        return false;
    }
    hostile_resume(frame)
}"#,
        breaks: "recovery fixup authority escapes its validated adapter through a private helper",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        file: "xtask/src/bounded_process.rs",
        find: "        capture.require_within_limit(capture_limit)",
        replace: "        capture.require_within_limit(MAX_CAPTURE_BYTES)",
        breaks: "child polling ignores its caller-specific capture ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "bounded_process::tests::a_caller_specific_capture_limit_is_polled_while_the_child_is_live",
        },
    },
    Mutation {
        file: "xtask/src/bounded_process.rs",
        find: "    capture.collect_with_limit(status, capture_limit)",
        replace: "    capture.collect_with_limit(status, MAX_CAPTURE_BYTES)",
        breaks: "completed child collection ignores its caller-specific capture ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "bounded_process::tests::captured_output_cannot_grow_collection_memory_past_its_limit",
        },
    },
    Mutation {
        file: "xtask/src/bounded_process.rs",
        find: "    for name in GIT_REDIRECT_ENVIRONMENT {\n        command.env_remove(name);\n    }",
        replace: "    for name in GIT_REDIRECT_ENVIRONMENT {\n        if *name != \"GIT_DIR\" {\n            command.env_remove(name);\n        }\n    }",
        breaks: "Git children can inherit an attacker-selected repository directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "bounded_process::tests::git_children_cannot_inherit_repository_or_pathspec_redirection",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        initial <= maximum,\n        \"{} is {initial} bytes, exceeding its {maximum}-byte digest limit\",\n        path.display()\n    );",
        replace: "    ensure!(\n        true,\n        \"{} is {initial} bytes, exceeding its {maximum}-byte digest limit\",\n        path.display()\n    );",
        breaks: "a generic qualification digest allocates from an oversized initial file",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::generic_qualification_digests_have_a_streaming_ceiling",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        bytes <= maximum,\n        \"{} grew beyond its {maximum}-byte digest limit while hashing\",\n        path.display()\n    );",
        replace: "    ensure!(\n        true,\n        \"{} grew beyond its {maximum}-byte digest limit while hashing\",\n        path.display()\n    );",
        breaks: "a qualification digest can grow beyond its semantic ceiling after preflight",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::generic_qualification_digest_growth_is_refused_after_measurement",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            length <= MAX_MODEL_FILE_BYTES,",
        replace: "            true,",
        breaks: "one oversized tracked model input reaches the hashing stream",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::tracked_model_files_are_size_checked_before_streaming",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            total <= MAX_MODEL_TREE_BYTES,",
        replace: "            true,",
        breaks: "the tracked model tree has no aggregate byte ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::tracked_model_tree_has_an_independent_total_size_bound",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "            .update(&(relative.len() as u64).to_le_bytes())",
        replace: "            .update(&0u64.to_le_bytes())",
        breaks: "the model-tree digest omits its path-length frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::streamed_model_tree_digest_preserves_the_length_delimited_schema",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    let output = run(command, PROBE_TIMEOUT, &command_label, maximum)?;",
        replace: "    let output = run(\n        command,\n        PROBE_TIMEOUT,\n        &command_label,\n        crate::bounded_process::MAX_CAPTURE_BYTES,\n    )?;",
        breaks: "qualification Git queries discard their caller's semantic output limit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::git_queries_enforce_the_callers_semantic_capture_limit",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        before == after,\n        \"qualification source identity changed while it was captured\"\n    );",
        replace: "    ensure!(\n        before == before,\n        \"qualification source identity changed while it was captured\"\n    );",
        breaks: "qualification accepts two different clean source identities",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::source_capture_requires_two_clean_views_of_one_identity",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    ensure!(\n        initial == expected,\n        \"{label} is {initial} bytes, expected exactly {expected}\"\n    );",
        replace: "    ensure!(\n        true,\n        \"{label} is {initial} bytes, expected exactly {expected}\"\n    );",
        breaks: "release evidence hashes a deterministic artifact before checking its exact size",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::deterministic_artifact_size_is_checked_before_hashing_payload_bytes",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    ensure!(\n        before == after,\n        \"release source identity changed while it was captured\"\n    );",
        replace: "    ensure!(\n        before == before,\n        \"release source identity changed while it was captured\"\n    );",
        breaks: "release evidence accepts two different clean source identities",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::release_source_capture_requires_two_clean_views_of_one_identity",
        },
    },
    Mutation {
        file: "xtask/src/supply_chain.rs",
        find: "        bytes.len() as u64 <= MAX_SPDX_BYTES,",
        replace: "        bytes.len() as u64 <= MAX_SPDX_BYTES + 1,",
        breaks: "an SPDX rendering one byte beyond its publication ceiling is staged",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::oversized_spdx_renderings_are_refused_before_staging",
        },
    },
    Mutation {
        file: "xtask/src/supply_chain.rs",
        find: "            after_publish(published)",
        replace: "            let _ = published;\n            Ok(())",
        breaks: "SPDX publication skips its late verifier and cannot roll back its refusal",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::a_late_spdx_publication_refusal_restores_the_previous_document",
        },
    },
    Mutation {
        file: "xtask/src/supply_chain.rs",
        find: "    ensure!(\n        initial <= maximum,\n        \"{label} is {initial} bytes, exceeding its {maximum}-byte allocation limit\"\n    );",
        replace: "    ensure!(\n        true,\n        \"{label} is {initial} bytes, exceeding its {maximum}-byte allocation limit\"\n    );",
        breaks: "supply-chain text allocates from an already oversized file",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::oversized_supply_chain_text_is_refused_before_allocation",
        },
    },
    Mutation {
        file: "xtask/src/supply_chain.rs",
        find: "    ensure!(\n        bytes.len() as u64 <= maximum,\n        \"{label} grew beyond its {maximum}-byte allocation limit\"\n    );",
        replace: "    ensure!(\n        true,\n        \"{label} grew beyond its {maximum}-byte allocation limit\"\n    );",
        breaks: "supply-chain text can grow beyond its allocation ceiling during the read",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::supply_chain_text_growth_is_refused_after_measurement",
        },
    },
    Mutation {
        file: "xtask/src/supply_chain.rs",
        find: "    crate::mutation_workspace::require_direct_file(parent, &path, label)?;\n    let mut file =",
        replace: "    let mut file =",
        breaks: "supply-chain text follows a redirected file before opening it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::supply_chain_text_rejects_a_redirect_before_opening",
        },
    },
    Mutation {
        file: "xtask/src/supply_chain.rs",
        find: "    crate::mutation_workspace::require_direct_file(parent, &path, label)?;\n    String::from_utf8(bytes)",
        replace: "    String::from_utf8(bytes)",
        breaks: "supply-chain text does not revalidate its direct file after reading",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::supply_chain_text_revalidates_its_path_after_reading",
        },
    },
    Mutation {
        file: "xtask/src/vendor.rs",
        find: "            matches!(\n                core::iter::Iterator::next(&mut components),\n                Some(Component::Normal(_))\n            ) && core::iter::Iterator::next(&mut components).is_none()\n                && !name.contains(['/', '\\\\'])\n                && !name.chars().any(char::is_control),",
        replace: "            true,",
        breaks: "a vendor checksum entry can name an indirect or control-bearing path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::checksum_names_are_unique_direct_children",
        },
    },
    Mutation {
        file: "xtask/src/vendor.rs",
        find: "            names.insert(name.to_string()),",
        replace: "            names.insert(name.to_string()) || names.contains(name),",
        breaks: "a vendor checksum listing accepts the same payload name twice",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::checksum_names_are_unique_direct_children",
        },
    },
    Mutation {
        file: "xtask/src/vendor.rs",
        find: "            entries.len() < MAX_DIRECTORY_ENTRIES,",
        replace: "            true,",
        breaks: "a vendor checksum listing has no independent entry-count ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::checksum_entry_count_is_bounded_independently_of_text_size",
        },
    },
    Mutation {
        file: "xtask/src/vendor.rs",
        find: "        initial <= MAX_LISTING_BYTES,",
        replace: "        true,",
        breaks: "a vendor checksum listing allocates from an already oversized file",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::oversized_checksum_listings_are_refused_before_allocation",
        },
    },
    Mutation {
        file: "xtask/src/vendor.rs",
        find: "        bytes.len() as u64 <= MAX_LISTING_BYTES,",
        replace: "        true,",
        breaks: "a vendor checksum listing can grow beyond its allocation ceiling while reading",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::checksum_listing_growth_is_refused_after_measurement",
        },
    },
    Mutation {
        file: "xtask/src/vendor.rs",
        find: "            length <= MAX_PAYLOAD_BYTES,",
        replace: "            true,",
        breaks: "an oversized vendored payload reaches the hashing stream",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::oversized_vendored_payloads_are_refused_before_hashing",
        },
    },
    Mutation {
        file: "xtask/src/vendor.rs",
        find: "            total <= MAX_DIRECTORY_PAYLOAD_BYTES,",
        replace: "            true,",
        breaks: "a vendored directory has no aggregate payload-byte ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::vendored_directories_have_an_independent_total_payload_bound",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        || (text.len() > 1 && text.starts_with('0'))\n",
        replace: "",
        breaks: "VMware verdict counts accept noncanonical leading-zero decimals",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::selftest_verdict_counts_use_one_canonical_decimal_grammar",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;",
        replace: "const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;",
        breaks: "VMware source parsing accepts twice its allocation ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::oversized_source_counts_are_refused_before_text_allocation",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "            length <= MAX_TRACKED_FILE_BYTES,",
        replace: "            true,",
        breaks: "one oversized tracked mutation source reaches the hashing stream",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::oversized_tracked_sources_are_refused_before_streaming",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "            total <= MAX_TRACKED_TREE_BYTES,",
        replace: "            true,",
        breaks: "the tracked mutation source tree has no aggregate byte ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::tracked_source_trees_have_an_independent_total_size_bound",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "            .update(&(name.len() as u64).to_le_bytes())",
        replace: "            .update(&0u64.to_le_bytes())",
        breaks: "the tracked-source digest omits its path-length frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::streamed_tracked_source_digest_preserves_the_framed_schema",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "pub const fn memory_window_size(low: u32, high: u32, width: MemoryWidth) -> Option<u64> {",
        replace: "pub const fn memory_window_size(low: u32, high: u32, width: bool) -> Option<u64> {",
        breaks: "PCI memory-window width becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "pub const fn doorbell(cap: Capabilities, queue: u16, doorbell: Doorbell) -> Option<usize> {",
        replace: "pub const fn doorbell(cap: Capabilities, queue: u16, doorbell: bool) -> Option<usize> {",
        breaks: "NVMe submission/completion doorbell selection becomes positional boolean state",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        replace: "    /// Recover hardware quiescence from an unnamed observation.\n    pub fn quiesce(&mut self, verified: bool) {\n        if verified {\n            self.close();\n        }\n    }\n\n    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        breaks: "pure NVMe DMA state regains boolean hardware-quiescence authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        replace: "    /// Recover retained hardware ownership from an unnamed observation.\n    pub fn retain_unquiesced(&mut self, retained: bool) {\n        if retained {\n            self.close();\n        }\n    }\n\n    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        breaks: "pure NVMe DMA state regains boolean retention authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "    pub fn close(&mut self) {",
        replace: "    pub fn close(&mut self, _closed: bool) {",
        breaks: "NVMe DMA closure regains a redundant unnamed state boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "    pub const fn new(size: u32, alignment: Alignment) -> Self {",
        replace: "    pub const fn new(size: u32, alignment: bool) -> Self {",
        breaks: "xstate component alignment becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "wost/src/sync.rs",
        find: "    fn acquire(&self, #[cfg(feature = \"selftest\")] kind: LockKind) -> Option<SpinLockGuard<'_, T>> {",
        replace: "    fn acquire(&self, #[cfg(feature = \"selftest\")] kind: bool) -> Option<SpinLockGuard<'_, T>> {",
        breaks: "spin-lock discipline becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/percpu.rs",
        find: "        #[cfg(feature = \"selftest\")] kind: LockKind,\n        #[cfg(feature = \"selftest\")] at: &'static core::panic::Location<'static>,",
        replace: "        #[cfg(feature = \"selftest\")] kind: bool,\n        #[cfg(feature = \"selftest\")] at: &'static core::panic::Location<'static>,",
        breaks: "per-CPU lock acquisition accounting becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/percpu.rs",
        find: "    pub(crate) fn released_lock(&self, #[cfg(feature = \"selftest\")] kind: LockKind) {",
        replace: "    pub(crate) fn released_lock(&self, #[cfg(feature = \"selftest\")] kind: bool) {",
        breaks: "per-CPU lock release accounting becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/pci.rs",
        find: "fn claim_as(at: Address, lifetime: ClaimLifetime) -> Result<Device, ClaimError> {",
        replace: "fn claim_as(at: Address, lifetime: bool) -> Result<Device, ClaimError> {",
        breaks: "PCI claim lifetime becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {",
        replace: "fn start_interrupts(external_controllers: bool) {",
        breaks: "interrupt startup receives controller-mask proof as an unnamed boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "const fn external_interrupts_are_admitted(mask: Option<&mem::ExternalControllersMasked>) -> bool {",
        replace: "const fn external_interrupts_are_admitted(mask: bool) -> bool {",
        breaks: "external-interrupt admission receives controller state as an unnamed boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "fn route_keyboard(routing: Option<mem::ExternalControllersMasked>) {",
        replace: "fn route_keyboard(routing: Option<bool>) {",
        breaks: "keyboard routing receives unnamed boolean external-controller authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/input.rs",
        find: "pub(crate) fn set_translation(translation: crate::arch::current::i8042::Translation) -> bool {",
        replace: "pub(crate) fn set_translation(translation: bool) -> bool {",
        breaks: "input translation policy becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    pub(crate) fn set_translation(&mut self, translation: Translation) -> bool {",
        replace: "    pub(crate) fn set_translation(&mut self, translation: bool) -> bool {",
        breaks: "i8042 translation policy becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    wanted: Readiness,",
        replace: "    wanted: bool,",
        breaks: "NVMe ready-state polling becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/thread.rs",
        find: "    fn choose(&self, current: usize, eligibility: CurrentEligibility) -> Option<usize> {",
        replace: "    fn choose(&self, current: usize, eligibility: bool) -> Option<usize> {",
        breaks: "scheduler current-thread eligibility becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/console.rs",
        find: "    fn show_cursor(&mut self) {",
        replace: "    fn show_cursor(&mut self, _visible: bool) {",
        breaks: "cursor display regains a redundant unnamed visibility boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/console.rs",
        find: "    fn hide_cursor(&mut self) {",
        replace: "    fn hide_cursor(&mut self, _visible: bool) {",
        breaks: "cursor hiding regains a redundant unnamed visibility boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/console.rs",
        find: "    fn show_cursor(&mut self) {",
        replace: "    fn set_cursor(&mut self, visible: bool) {\n        let colour = if visible {\n            self.palette.foreground\n        } else {\n            self.palette.background\n        };\n        self.paint_cursor(colour);\n    }\n\n    fn show_cursor(&mut self) {",
        breaks: "cursor visibility regrows one boolean-shaped operation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "#[derive(Debug)]\npub(crate) struct ExternalControllersMasked {",
        replace: "#[derive(Debug, Clone)]\npub(crate) struct ExternalControllersMasked {",
        breaks: "external-controller mask authority can be duplicated after one hardware proof",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "pub(crate) struct ExternalControllersMasked {\n    routing: Option<Routing>,\n}",
        replace: "pub(crate) struct ExternalControllersMasked {\n    pub(crate) routing: Option<Routing>,\n}",
        breaks: "kernel peers can forge external-controller mask authority through a visible field",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "pub(crate) struct ExternalControllersMasked {\n    routing: Option<Routing>,\n}\n\nstruct ControllersReleased {",
        replace: "pub(crate) struct ExternalControllersMasked {\n    routing: Option<Routing>,\n}\n\nuse self::ExternalControllersMasked as ForgedMask;\n\nstruct ControllersReleased {",
        breaks: "external-controller mask authority gains an imported constructor alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "impl ExternalControllersMasked {\n    /// The selected keyboard controller, when the proven inventory has one.\n    pub(crate) fn into_routing(self) -> Option<Routing> {\n        self.routing\n    }\n}",
        replace: "impl ExternalControllersMasked {\n    /// The selected keyboard controller, when the proven inventory has one.\n    pub(crate) fn into_routing(self) -> Option<Routing> {\n        self.routing\n    }\n}\n\nfn forged_mask_authority() -> ExternalControllersMasked {\n    ExternalControllersMasked { routing: None }\n}",
        breaks: "external-controller mask authority gains a mint outside its checked transition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "            controllers_masked: false,",
        replace: "            controllers_masked: true,",
        breaks: "an unproven external-controller inventory manufactures mask authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "struct ControllersReleased {\n    _private: (),\n}",
        replace: "#[derive(Clone)]\nstruct ControllersReleased {\n    _private: (),\n}",
        breaks: "one exact controller cleanup can duplicate its release evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "impl RoutingPreparation {\n    fn ready(routing: Routing) -> Self {",
        replace: "impl RoutingPreparation {\n    const fn polled_without_release() -> Self {\n        Self {\n            routing: None,\n            controllers_masked: true,\n        }\n    }\n\n    fn ready(routing: Routing) -> Self {",
        breaks: "polled routing can manufacture mask authority without controller-release evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "impl RoutingPreparation {\n    fn ready(routing: Routing) -> Self {",
        replace: "fn forged_routing_preparation() -> RoutingPreparation {\n    RoutingPreparation {\n        routing: None,\n        controllers_masked: true,\n    }\n}\n\nimpl RoutingPreparation {\n    fn ready(routing: Routing) -> Self {",
        breaks: "routing preparation gains a raw constructor outside its audited transitions",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "struct ControllersReleased {\n    _private: (),\n}\n\nimpl RoutingPreparation {",
        replace: "struct ControllersReleased {\n    _private: (),\n}\n\nfn forged_controller_release() -> ControllersReleased {\n    ControllersReleased { _private: () }\n}\n\nimpl RoutingPreparation {",
        breaks: "controller-release evidence gains a mint outside exact cleanup",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "            retain_routing_registers(registers, core::mem::take(controllers));\n            return None;",
        replace: "            retain_routing_registers(registers, core::mem::take(controllers));\n            return Some(ControllersReleased { _private: () });",
        breaks: "failed controller cleanup still mints release evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "            warn!(\"ioapic: candidate mapping remained owned during cleanup\");\n            retain_routing_registers(registers, core::mem::take(controllers));",
        replace: "            assert!(controllers.is_empty());\n            retain_routing_registers(registers, core::mem::take(controllers));",
        breaks: "routing cleanup can panic before retaining a refused device owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "        Some(routing) if interrupt::are_enabled() => routing,",
        replace: "        Some(routing) if true => routing,",
        breaks: "keyboard routing proceeds without observing enabled maskable interrupts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "        Some(routing) if interrupt::are_enabled() => routing,",
        replace: "        Some(routing) if { let _ = interrupt::are_enabled(); true } => routing,",
        breaks: "keyboard routing observes interrupt state but ignores its value",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        replace: "fn enable_before_mask_proof() {\n    interrupt::enable();\n}\n\nfn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    enable_before_mask_proof();\n    if let Err(reason) = pic::silence() {",
        breaks: "an interrupt-enable helper bypasses external-controller mask admission",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        replace: "fn retain_interrupt_enable() {\n    let _ = interrupt::enable;\n}\n\nfn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        breaks: "interrupt enable gains an indirect function reference outside its audited calls",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        replace: "use wost::arch::current::interrupt::enable as alias_enable;\n\nfn call_aliased_interrupt_enable() {\n    alias_enable();\n}\n\nfn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        breaks: "interrupt enable gains an imported alias outside mask admission",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "    mask.is_some()",
        replace: "    mask.is_none()",
        breaks: "missing external-controller mask authority is admitted while real authority is refused",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        file: "xtask/src/typed_arguments.rs",
        find: "        visit::visit_item_impl(self, implementation);",
        replace: "        let _ = implementation;",
        breaks: "nested inherent methods disappear from typed API-contract discovery",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::nested_inherent_methods_remain_visible_to_api_contracts",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        self.state = DmaOwnership::Closed;",
        replace: "        self.state = DmaOwnership::Idle;",
        breaks: "closing pure DMA state reopens publication instead of making it terminal",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::closed_dma_state_cannot_publish_or_complete",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    wait_for_ready(registers, *capabilities, Readiness::NotReady)?;",
        replace: "    wait_for_ready(registers, *capabilities, Readiness::Ready)?;",
        breaks: "controller-stop evidence is minted after observing the ready bit set",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    let cc = read(registers, register::CC)?;",
        replace: "    let cc = 0;",
        breaks: "controller-stop evidence substitutes a constant for its fresh configuration read",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    let csts = read(registers, register::CSTS)?;",
        replace: "    let csts = 0;",
        breaks: "controller-stop evidence substitutes a constant for its fresh status read",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    let observed = DisabledRegisters::from_bits(cc, csts).map_err(|_| BringUpError::NotReady)?;",
        replace: "    let observed = DisabledRegisters::from_bits(0, 0).map_err(|_| BringUpError::NotReady)?;",
        breaks: "controller-stop evidence classifies constants instead of its fresh register pair",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "            self.dma.disposition = DmaDisposition::Quiesced;",
        replace: "            self.dma.disposition = DmaDisposition::Retained;",
        breaks: "proven controller-stop evidence retains recyclable DMA ownership",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::proven_stop_consumes_evidence_before_recycling",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    dma: &'controller mut DmaState,",
        replace: "    dma: &'controller DmaState,",
        breaks: "controller-stop evidence loses mutable authority over the exact DMA state it retires",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    fn quiesce(self) {",
        replace: "    fn quiesce(&mut self) {",
        breaks: "one controller-stop observation can authorize more than one DMA recycle transition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::proven_stop_consumes_evidence_before_recycling",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: r#"    Ok(ControllerStopped {
        _registers: registers,
        _observed: observed,
        dma,
    })
}"#,
        replace: r#"    Ok(mint_stopped(registers, observed, dma))
}

fn mint_stopped<'controller>(
    registers: &'controller MappedBar,
    observed: DisabledRegisters,
    dma: &'controller mut DmaState,
) -> ControllerStopped<'controller> {
    ControllerStopped {
        _registers: registers,
        _observed: observed,
        dma,
    }
}"#,
        breaks: "stop evidence can bind a hardware observation and DMA state from different controllers",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        file: "xtask/src/typed_arguments.rs",
        find: "        if implementation.trait_.is_none() && type_is(&implementation.self_ty, self.owner) {",
        replace: "        if type_is(&implementation.self_ty, self.owner) {",
        breaks: "a cosmetic trait method can satisfy an inherent typed-API contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::trait_methods_do_not_satisfy_inherent_api_contracts",
        },
    },
    Mutation {
        file: "xtask/src/dma_contracts.rs",
        find: "    methods.visit_file(syntax);",
        replace: "    for item in &syntax.items {\n        if let Item::Impl(implementation) = item {\n            methods.visit_item_impl(implementation);\n        }\n    }",
        breaks: "a nested inherent DMA-authority method can hide outside the top-level item list",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::nested_inherent_methods_cannot_hide_from_dma_authority_contracts",
        },
    },
    Mutation {
        file: "xtask/src/dma_contracts.rs",
        find: "    find_method(syntax, owner, name, false)",
        replace: "    find_method(syntax, owner, name, true)",
        breaks: "a trait implementation can hide forbidden pure DMA authority from the contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::trait_methods_do_not_satisfy_dma_authority_contracts",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "#[derive(Debug)]\nstruct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}",
        replace: "#[derive(Debug)]\nstruct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nconst _: () = {\n    impl Clone for ControllerStopped<'_> {\n        fn clone(&self) -> Self {\n            unreachable!()\n        }\n    }\n};",
        breaks: "a nested trait implementation can duplicate one observed controller stop into multiple recycle authorities",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\nenum DmaDisposition {",
        replace: "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\nenum DmaDisposition {",
        breaks: "a controller's DMA retirement disposition becomes freely copyable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "impl ControllerStopped<'_> {\n    fn quiesce(self) {",
        replace: "impl ControllerStopped<'_> {\n    fn duplicate(self) -> Self {\n        self\n    }\n\n    fn quiesce(self) {",
        breaks: "controller-stop evidence gains an extra inherent transfer path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nimpl ControllerStopped<'_> {",
        replace: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\ntype ControllerStoppedAlias<'controller> = ControllerStopped<'controller>;\n\nimpl ControllerStopped<'_> {",
        breaks: "controller-stop evidence gains a constructor alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nimpl ControllerStopped<'_> {",
        replace: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nuse self::ControllerStopped as ForgedStopped;\n\nimpl ControllerStopped<'_> {",
        breaks: "controller-stop evidence gains an imported constructor alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    const fn is_settled(&self) -> bool {\n        !matches!(&self.disposition, DmaDisposition::Active)\n    }",
        replace: "    const fn is_settled(&self) -> bool {\n        true\n    }",
        breaks: "active DMA is reported settled before hardware quiescence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        if self.dma.disposition == DmaDisposition::Active {",
        replace: "        if true {",
        breaks: "consumed stop evidence can rewrite an already terminal DMA disposition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::proven_stop_consumes_evidence_before_recycling",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        if self.dma.must_retain() {\n            self.admin.retain();\n            if let Some(queue) = self.io.as_mut() {\n                queue.retain();\n            }\n            if let Some(segment) = data {\n                segment.retain();\n            }\n        }",
        replace: "",
        breaks: "a failed controller stop retains none of its reachable DMA runs",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::failed_stop_retention_and_retirement_sinks_are_exact",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        self.completion.retain();",
        replace: "",
        breaks: "failed controller cleanup recycles the completion queue while hardware may reach it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::failed_stop_retention_and_retirement_sinks_are_exact",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        if !self.dma.is_quiesced() {\n            return quarantine_controller(self);\n        }",
        replace: "        if !self.dma.is_quiesced() {\n            return ControllerRetirement::Retired;\n        }",
        breaks: "an unverified controller stop drops its complete quarantine owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_retirement_returns_every_cleanup_owner",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        mem::MappedPciBarRetirement::Quarantined => PrepareFailure::memory_quarantine(reason),",
        replace: "        mem::MappedPciBarRetirement::Quarantined => PrepareFailure::Retired(reason),",
        breaks: "memory-owned BAR quarantine is misreported as successful retirement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::mapped_bar_cleanup_quarantine_cannot_be_reported_retired",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    *retained = Some(RetainedController {\n        _controller: controller,\n        device: None,\n    });",
        replace: "    drop(controller);",
        breaks: "controller quarantine mints its token after dropping the controller owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_retirement_returns_every_cleanup_owner",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "impl Drop for Controller {\n    fn drop(&mut self) {\n        self.stop_dma(None);\n    }\n}",
        replace: "impl Drop for Controller {\n    fn drop(&mut self) {}\n}",
        breaks: "dropping a controller skips the final DMA stop-or-retain boundary",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::failed_stop_retention_and_retirement_sinks_are_exact",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        if observed == wanted {",
        replace: "        if observed == wanted || wanted == Readiness::NotReady {",
        breaks: "a disable wait succeeds before observing the controller ready bit clear",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_waits_for_the_requested_hardware_state",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        let observed = if bits & status::READY != 0 {",
        replace: "        let observed = if bits & status::READY == 0 {",
        breaks: "the readiness poll reverses the controller's live ready-bit observation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_waits_for_the_requested_hardware_state",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        replace: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\nfn swap_dma_states(left: &mut Controller, right: &mut Controller) {\n    core::mem::swap(&mut left.dma, &mut right.dma);\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        breaks: "DMA retirement state can be swapped between two controllers",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        replace: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\nfn alias_dma_state(controller: &Controller) {\n    let _ = &controller.dma;\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        breaks: "DMA state gains an unaudited whole-state access alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "    pub fn close(&mut self) {",
        replace: "    pub fn quiesce(&mut self) {",
        breaks: "pure DMA state renames terminal closure into a hardware-recycling claim",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            WORKER_IS_PAUSED.store(true, Ordering::Release);",
        replace: "            WORKER_IS_PAUSED.store(false, Ordering::Release);",
        breaks: "the queue worker never acknowledges its release-race pause",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    fn map_device_range(\n        &mut self,\n        admitted: &AdmittedDeviceRange,\n    ) -> Result<DeviceMapping, MapError> {",
        replace: "    fn map_device_range(\n        &mut self,\n        admitted: &AdmittedDeviceRange,\n    ) -> Result<DeviceMapping, MapError> {\n        let _fixed_projection = [None::<DeviceExtent>; MAX_DEVICE_MAPPINGS];",
        breaks: "device mapping rebuilds a maximum-sized ownership projection on the kernel stack",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::an_address_space_keeps_its_bounded_ledger_off_the_stack",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let virtual_extent =\n            AddressRange::new(virtual_base, pages).ok_or(MapError::InvalidDeviceRange(physical))?;\n\n        let expected_flags = Flags::PRESENT\n            .union(Flags::WRITABLE)\n            .union(Flags::NO_EXECUTE)\n            .union(Flags::NO_CACHE);\n        self.map_range(\n            virtual_base,\n            base,\n            pages,\n            expected_flags.difference(Flags::PRESENT),\n            Grain::Pages,\n        )?;",
        replace: "        let expected_flags = Flags::PRESENT\n            .union(Flags::WRITABLE)\n            .union(Flags::NO_EXECUTE)\n            .union(Flags::NO_CACHE);\n        self.map_range(\n            virtual_base,\n            base,\n            pages,\n            expected_flags.difference(Flags::PRESENT),\n            Grain::Pages,\n        )?;\n        let virtual_extent =\n            AddressRange::new(virtual_base, pages).ok_or(MapError::InvalidDeviceRange(physical))?;",
        breaks: "device mapping keeps fallible extent construction after page-table commit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::an_address_space_keeps_its_bounded_ledger_off_the_stack",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "pub(crate) struct TaskStackPin {\n    stacks: Vec<StackIdentity>,\n}\n\n/// A task-stack top borrowed from a pinned arena identity.",
        replace: "pub(crate) struct TaskStackPin {\n    stacks: Vec<StackIdentity>,\n}\n\nimpl Drop for TaskStackPin {\n    fn drop(&mut self) {\n        let mut pinned = TASK_STACK_PIN.lock_leaf();\n        *pinned = None;\n    }\n}\n\n/// A task-stack top borrowed from a pinned arena identity.",
        breaks: "dropping the task stack owner restores unmap authority over hardware-retained stacks",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::a_kernel_stack_top_cannot_outlive_its_pin",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "pub(crate) struct KernelStackTop<'pin>(&'pin StackIdentity);\n\nimpl KernelStackTop<'_> {\n    /// The address hardware may retain in `RSP0`.\n    pub(crate) const fn address(self) -> u64 {\n        self.0.slot.stack_top()\n    }\n}\n\nimpl TaskStackPin {\n    /// The pinned stack at `index`, as retained-hardware authority.\n    pub(crate) fn stack_top(&self, index: usize) -> Option<KernelStackTop<'_>> {\n        let identity = self.stacks.get(index)?;\n        if !is_active() {\n            return None;\n        }\n        let current = cpu::page_table_base();\n        identity\n            .invalid_page_in(current)\n            .is_none()\n            .then_some(KernelStackTop(identity))\n    }",
        replace: "pub(crate) struct KernelStackTop<'pin> {\n    address: u64,\n    _pin: core::marker::PhantomData<&'pin TaskStackPin>,\n}\n\nimpl KernelStackTop<'_> {\n    /// The address hardware may retain in `RSP0`.\n    pub(crate) const fn address(self) -> u64 {\n        self.address\n    }\n}\n\nimpl TaskStackPin {\n    /// The pinned stack at `index`, as retained-hardware authority.\n    pub(crate) fn stack_top(&self, index: usize) -> Option<KernelStackTop<'_>> {\n        let identity = self.stacks.get(index)?;\n        if !is_active() {\n            return None;\n        }\n        let current = cpu::page_table_base();\n        identity\n            .invalid_page_in(current)\n            .is_none()\n            .then_some(KernelStackTop {\n                address: identity.slot.stack_top(),\n                _pin: core::marker::PhantomData,\n            })\n    }",
        breaks: "kernel stack-top authority carries only an address and phantom pin instead of borrowing its identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::a_kernel_stack_top_cannot_outlive_its_pin",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: r#"            None => {
                self.devices
                    .try_reserve(1)
                    .map_err(|_| MapError::OutOfMemory)?;
                self.devices.len()
            }
        };

        let Some(device_window) =
            AddressRange::new(DEVICE_WINDOW_START, DEVICE_WINDOW_END - DEVICE_WINDOW_START)
        else {
            return Err(MapError::OutOfMemory);
        };
        let virtual_base = first_fit_by(device_window, pages, PAGE_SIZE, &self.devices, |extent| {
            extent.map(|extent| extent.virtual_range)
        })
        .ok_or(MapError::OutOfMemory)?;
        let virtual_extent =
            AddressRange::new(virtual_base, pages).ok_or(MapError::InvalidDeviceRange(physical))?;

        let expected_flags = Flags::PRESENT
            .union(Flags::WRITABLE)
            .union(Flags::NO_EXECUTE)
            .union(Flags::NO_CACHE);
        self.map_range(
            virtual_base,
            base,
            pages,
            expected_flags.difference(Flags::PRESENT),
            Grain::Pages,
        )?;"#,
        replace: r#"            None => self.devices.len(),
        };

        let Some(device_window) =
            AddressRange::new(DEVICE_WINDOW_START, DEVICE_WINDOW_END - DEVICE_WINDOW_START)
        else {
            return Err(MapError::OutOfMemory);
        };
        let virtual_base = first_fit_by(device_window, pages, PAGE_SIZE, &self.devices, |extent| {
            extent.map(|extent| extent.virtual_range)
        })
        .ok_or(MapError::OutOfMemory)?;
        let virtual_extent =
            AddressRange::new(virtual_base, pages).ok_or(MapError::InvalidDeviceRange(physical))?;

        let expected_flags = Flags::PRESENT
            .union(Flags::WRITABLE)
            .union(Flags::NO_EXECUTE)
            .union(Flags::NO_CACHE);
        self.map_range(
            virtual_base,
            base,
            pages,
            expected_flags.difference(Flags::PRESENT),
            Grain::Pages,
        )?;
        if device_slot == self.devices.len() {
            self.devices
                .try_reserve(1)
                .map_err(|_| MapError::OutOfMemory)?;
        }"#,
        breaks: "device-ledger allocation can fail only after page-table mapping commits",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::an_address_space_keeps_its_bounded_ledger_off_the_stack",
        },
    },
    Mutation {
        file: "kernel/src/fs.rs",
        find: "pub(crate) const VOLUME_SECTOR: u64 = 2048;",
        replace: "pub(crate) const VOLUME_SECTOR: u64 = 2048;\n\nfn with_volume() {}",
        breaks: "filesystem volume ownership regrows a callback authority door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "const COMMAND_TIMEOUT_MS: u64 = 5_000;",
        replace: "const COMMAND_TIMEOUT_MS: u64 = 5_000;\n\nfn with_controller() {}",
        breaks: "NVMe controller ownership regrows a callback authority door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "kernel/src/tmpfs.rs",
        find: "pub(crate) const ROOT: usize = 0;",
        replace: "pub(crate) const ROOT: usize = 0;\n\nfn walk() {}",
        breaks: "tmpfs regrows a speculative internal pathname walker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "fn checksum_is_valid(bytes: &[u8]) -> bool {",
        replace: "pub fn checksum_is_valid(bytes: &[u8]) -> bool {",
        breaks: "ACPI checksum validation regrows as speculative public surface",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "#[cfg(test)]\nfn fix_checksum(bytes: &mut [u8], index: usize) {",
        replace: "pub fn fix_checksum(bytes: &mut [u8], index: usize) {",
        breaks: "the test-only ACPI checksum forger becomes a production API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "    const fn sector_of(self, cluster: u32) -> Result<u64, FatError> {",
        replace: "    pub const fn sector_of(self, cluster: u32) -> Result<u64, FatError> {",
        breaks: "FAT cluster-to-sector internals regrow as speculative public arithmetic",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "    const fn table_position(self, cluster: u32) -> (u64, usize) {",
        replace: "    pub const fn table_position(self, cluster: u32) -> (u64, usize) {",
        breaks: "FAT table-position internals regrow as speculative public arithmetic",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "const fn decode_entry(raw: u32) -> Next {",
        replace: "pub const fn decode_entry(raw: u32) -> Next {",
        breaks: "raw FAT entry decoding regrows as a speculative public policy door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "const fn decode_slot(slot: &[u8; ENTRY_BYTES]) -> Slot {",
        replace: "pub const fn decode_slot(slot: &[u8; ENTRY_BYTES]) -> Slot {",
        breaks: "raw FAT directory-slot decoding regrows as a speculative public policy door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/memory-map/src/lib.rs",
        find: "pub const MAX_REGIONS: usize = 128;",
        replace: "pub const MAX_REGIONS: usize = 128;\n\npub fn allocatable_bytes() -> u64 {\n    0\n}",
        breaks: "memory-map allocation policy regrows a speculative aggregate API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/path-walk/src/lib.rs",
        find: "fn is_a_name(component: &[u8]) -> bool {",
        replace: "pub fn is_a_name(component: &[u8]) -> bool {",
        breaks: "pathname component policy regrows as a speculative public predicate",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "const fn io_window_size(mask: u32) -> Option<u16> {",
        replace: "pub const fn io_window_size(mask: u32) -> Option<u16> {",
        breaks: "PCI I/O-window decoding regrows as speculative public arithmetic",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "pub const BARS: usize = 6;",
        replace: "pub const BARS: usize = 6;\n\nfn subordinate_bus() -> u8 {\n    0\n}",
        breaks: "PCI bridge traversal regrows a speculative subordinate-bus helper",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/stack-layout/src/lib.rs",
        find: "pub const PAGE_SIZE: u64 = 4096;",
        replace: "pub const PAGE_SIZE: u64 = 4096;\n\nfn stack_overlaps() -> bool {\n    false\n}",
        breaks: "stack layout regrows a second speculative overlap policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/pci.rs",
        find: "const FUNCTION_CLAIMS: usize = 8;",
        replace: "const FUNCTION_CLAIMS: usize = 8;\n\nfn write_command() {}",
        breaks: "the PCI adapter regrows an unchecked command-register write helper",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "pub const CEILING: u64 = 16 << 30;",
        replace: "pub const CEILING: u64 = 16 << 30;\n\nfn first_region() {}",
        breaks: "physical-memory admission regrows a speculative first-region shortcut",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "pub const CEILING: u64 = 16 << 30;",
        replace: "pub const CEILING: u64 = 16 << 30;\n\nfn past_every_region() {}",
        breaks: "physical-memory admission regrows a speculative region-tail shortcut",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\n#[must_use = \"dropping function authority permanently retains its claim\"]\npub struct FunctionAuthority {",
        replace: "#[derive(Debug, Clone, PartialEq, Eq)]\n#[must_use = \"dropping function authority permanently retains its claim\"]\npub struct FunctionAuthority {",
        breaks: "pure PCI function authority can be cloned",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\n#[must_use = \"dropping function authority permanently retains its claim\"]\npub struct FunctionAuthority {",
        replace: "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n#[must_use = \"dropping function authority permanently retains its claim\"]\npub struct FunctionAuthority {",
        breaks: "pure PCI function authority can be copied",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/pci.rs",
        find: "#[derive(Debug)]\n#[must_use = \"dropping PCI function authority permanently retains its claim\"]\npub struct FunctionClaim {",
        replace: "#[derive(Debug, Clone)]\n#[must_use = \"dropping PCI function authority permanently retains its claim\"]\npub struct FunctionClaim {",
        breaks: "hardware PCI function authority can be cloned",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/pci.rs",
        find: "#[derive(Debug)]\n#[must_use = \"dropping PCI function authority permanently retains its claim\"]\npub struct FunctionClaim {",
        replace: "#[derive(Debug, Clone, Copy)]\n#[must_use = \"dropping PCI function authority permanently retains its claim\"]\npub struct FunctionClaim {",
        breaks: "hardware PCI function authority can be copied",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "struct CommandLeases<const N: usize> {",
        replace: "pub struct CommandLeases<const N: usize> {",
        breaks: "callers can substitute a parallel PCI command-lease registry",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    fn release(&mut self, authority: FunctionAuthority) -> Result<(), FunctionAuthority> {",
        replace: "    pub fn release(&mut self, authority: FunctionAuthority) -> Result<(), FunctionAuthority> {",
        breaks: "callers can release PCI function authority without settling its live subleases",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "xtask/src/contracts.rs",
        find: "let _ = pci::enable_memory_space(at);",
        replace: "let _ = pci::read(at);",
        breaks: "the raw-address compile probe stops exercising PCI memory-decode mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "xtask/src/contracts.rs",
        find: "let _ = pci::enable_bus_master(at);",
        replace: "let _ = pci::read(at);",
        breaks: "the raw-address compile probe stops exercising PCI bus-master mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "xtask/src/contracts.rs",
        find: "let _ = pci::admit_bus_master(at, state);",
        replace: "let _ = pci::read(at);",
        breaks: "the raw-address compile probe stops exercising PCI bus-master upgrades",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "xtask/src/contracts.rs",
        find: "let _ = pci::restore_command(at, state);",
        replace: "let _ = pci::read(at);",
        breaks: "the raw-address compile probe stops exercising PCI command restoration",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "xtask/src/contracts.rs",
        find: "let _ = pci::commit_command(at, state);",
        replace: "let _ = pci::read(at);",
        breaks: "the raw-address compile probe stops exercising PCI command commit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "xtask/src/contracts.rs",
        find: "let _ = pci::probe_bars(at);",
        replace: "let _ = pci::read(at);",
        breaks: "the raw-address compile probe stops exercising PCI BAR mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "xtask/src/contracts.rs",
        find: "let _ = pci::retry_probe(at, retry);",
        replace: "let _ = pci::read(at);",
        breaks: "the raw-address compile probe stops exercising PCI BAR-restoration retry",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "xtask/src/contracts.rs",
        find: "let _ = pci::release_function(at);",
        replace: "let _ = pci::read(at);",
        breaks: "the raw-address compile probe stops exercising PCI authority release",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/pci.rs",
        find: "pub fn read(at: Address) -> Option<u32> {",
        replace: "pub(crate) fn read(at: Address) -> Option<u32> {",
        breaks: "read-only PCI enumeration disappears behind crate visibility",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/pci.rs",
        find: "pub fn claim_function(at: Address) -> Result<FunctionClaim, ClaimError> {",
        replace: "pub(crate) fn claim_function(at: Address) -> Result<FunctionClaim, ClaimError> {",
        breaks: "the sole public PCI function-authority claim door disappears",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "xtask/src/recovery_contracts.rs",
        find: "impl<'ast> Visit<'ast> for WriterFunctions<'ast> {",
        replace: "impl<'ast> Visit<'ast> for WriterFunctions<'ast> {\n    fn visit_item_mod(&mut self, _module: &'ast syn::ItemMod) {}",
        breaks: "instruction-pointer writers hidden in nested modules disappear from recovery-contract discovery",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::nested_and_associated_instruction_pointer_writers_remain_visible",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        Some(\"std\" | \"core\" | \"alloc\") => ImportOrigin::Standard,",
        replace: "        Some(\"std\" | \"core\" | \"alloc\") => ImportOrigin::Other,",
        breaks: "imported standard trait bodies are misclassified as unresolved",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::standard_trait_imports_have_external_authority",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    #[cfg(test)]\n    fn observe(\n        &mut self,\n        graph: usize,\n        kind: RootKind,\n        inventory: &Inventory,\n        root: &Path,\n        output: &str,\n        instrumentation: &Instrumentation,\n    ) -> Result<()> {\n        if self.graphs.len() <= graph {\n            self.graphs.resize_with(graph + 1, Uses::default);\n        }",
        replace: "    #[cfg(test)]\n    fn observe(\n        &mut self,\n        graph: usize,\n        kind: RootKind,\n        inventory: &Inventory,\n        root: &Path,\n        output: &str,\n        instrumentation: &Instrumentation,\n    ) -> Result<()> {\n        self.graphs.resize_with(graph + 1, Uses::default);",
        breaks: "a later compiler round erases every higher graph's accumulated API evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_later_round_preserves_higher_graph_evidence",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            uses.observe_cargo(\n                compilation.graph,\n                compilation.kind,\n                &inventory,\n                &workspace.root,\n                &compilation.diagnostics,\n                &insertions,\n            )?;",
        replace: "            if compilation.kind == RootKind::Production {\n                uses.observe_cargo(\n                    compilation.graph,\n                    compilation.kind,\n                    &inventory,\n                    &workspace.root,\n                    &compilation.diagnostics,\n                    &insertions,\n                )?;\n            }",
        breaks: "host tests and examples stop contributing legitimate compiler-resolved API callers",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::authoritative_verdict_includes_every_compiled_graph",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            CargoTargetKind::Library | CargoTargetKind::Test | CargoTargetKind::Bench => false,",
        replace: "            CargoTargetKind::Library => true,\n            CargoTargetKind::Test | CargoTargetKind::Bench => false,",
        breaks: "Cargo metadata for a library grants a conventional-filename executable root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cargo_metadata_not_a_conventional_filename_defines_a_target_root",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            RootKind::TestHarness => self.verification,",
        replace: "            RootKind::TestHarness => self.production || self.verification,",
        breaks: "a test-harness graph roots the target program's production main",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::test_harness_graphs_do_not_root_the_target_program_main",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        ensure!(\n            test_artifact,\n            \"Cargo target `{}:{}` uses a custom test harness, which the API audit cannot classify\",\n            selected.package,\n            selected.name\n        );",
        replace: "        let _unchecked_test_artifact = test_artifact;",
        breaks: "a custom test harness is accepted as standard-harness compiler evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cargo_test_artifacts_must_prove_the_standard_harness",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                        CargoTargetKind::Library\n                        | CargoTargetKind::Bin\n                        | CargoTargetKind::Example => target.test,",
        replace: "                        CargoTargetKind::Library => false,\n                        CargoTargetKind::Bin | CargoTargetKind::Example => target.test,",
        breaks: "a host library's test harness receives no independent compiler graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_host_target_mode_receives_its_own_compiler_graph",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                        CargoTargetKind::Test | CargoTargetKind::Bench => true,",
        replace: "                        CargoTargetKind::Test | CargoTargetKind::Bench => target.test,",
        breaks: "an explicit test or benchmark target with metadata test=false receives no standard harness graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_host_target_mode_receives_its_own_compiler_graph",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    let mut args = vec![command.to_string()];\n    if matches!(plan.mode, CompilationMode::TestHarness(_)) {",
        replace: "    let mut args = vec![command.to_string()];\n    if matches!(plan.mode, CompilationMode::RegisteredPass) {\n        args.push(\"--all-targets\".to_string());\n    }\n    if matches!(plan.mode, CompilationMode::TestHarness(_)) {",
        breaks: "one registered pass merges mutually exclusive Cargo targets into one evidence graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::compiler_graphs_never_merge_distinct_cargo_targets",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    if core::iter::Iterator::next(&mut lines).is_some() {\n        return false;\n    }\n    let Some((identity, location)) = row.rsplit_once(\" at \") else {",
        replace: "    let Some((identity, location)) = row.rsplit_once(\" at \") else {",
        breaks: "an API-audit mutation can be credited despite an additional dead item",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::api_audit_mutations_require_the_current_structured_dead_item_verdict",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "fn graph_build_directory(build: &Path, graph: usize) -> Result<PathBuf> {\n    let directory = build.join(format!(\"graph-{graph}\"));\n    let directory = crate::mutation_workspace::prepare_real_child(\n        build,\n        &directory,\n        \"API audit graph build directory\",\n    )?;\n    Ok(crate::cargo::child_process_path(&directory))\n}",
        replace: "fn graph_build_directory(build: &Path, _graph: usize) -> Result<PathBuf> {\n    Ok(crate::cargo::child_process_path(build))\n}",
        breaks: "mutually exclusive compiler graphs share incremental artifacts and lose cached diagnostics",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::compiler_graphs_have_distinct_stable_build_directories",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        \"--locked\".to_string(),",
        replace: "",
        breaks: "the copied API-audit lock graph may be rewritten during compilation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::compiler_graphs_are_offline_and_lockfile_frozen",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    args.extend([\"--target\".to_string(), plan.target.clone()]);",
        replace: "    let _implicit_host_target = &plan.target;",
        breaks: "a host compiler graph relies on Cargo's ambient target instead of its pinned explicit target",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::compiler_graphs_are_offline_and_lockfile_frozen",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if current == *expected {\n            return Ok(());\n        }",
        replace: "        if true || current == *expected {\n            return Ok(());\n        }",
        breaks: "a compiler or build script can rewrite the instrumented API-audit copy without invalidating its verdict",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::compiler_graphs_cannot_change_the_instrumented_copy",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    workspace.require_copy_unchanged(expected)?;\n    workspace.require_source_unchanged(&workspace.source_root)?;\n    compiler.require_unchanged()?;\n    if !result.status.success() {",
        replace: "    workspace.require_source_unchanged(&workspace.source_root)?;\n    compiler.require_unchanged()?;\n    if !result.status.success() {",
        breaks: "the copied API-audit input is not rebound after each compiler graph completes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::copied_source_is_bound_after_instrumentation_and_after_each_graph",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    workspace.require_copy_unchanged(expected)?;\n    workspace.require_source_unchanged(&workspace.source_root)?;\n    compiler.require_unchanged()?;\n    if !result.status.success() {",
        replace: "    workspace.require_copy_unchanged(expected)?;\n    compiler.require_unchanged()?;\n    if !result.status.success() {",
        breaks: "the original API-audit source is not rebound after each compiler graph completes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::api_audit_commands_rebind_original_and_copied_sources_before_verdicts",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        workspace.require_copy_unchanged(expected)?;\n        workspace.require_source_unchanged(&workspace.source_root)?;\n        workspace.require_cargo_inputs_unchanged()?;\n        compiler.require_unchanged()?;\n        ensure!(",
        replace: "        workspace.require_source_unchanged(&workspace.source_root)?;\n        workspace.require_cargo_inputs_unchanged()?;\n        compiler.require_unchanged()?;\n        ensure!(",
        breaks: "Cargo metadata can rewrite the copied API-audit graph before its output is trusted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::api_audit_commands_rebind_original_and_copied_sources_before_verdicts",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            .context(\"spawning Cargo metadata for the API audit\")?;\n        workspace.require_copy_unchanged(expected)?;\n        workspace.require_source_unchanged(&workspace.source_root)?;\n        workspace.require_cargo_inputs_unchanged()?;\n        compiler.require_unchanged()?;",
        replace: "            .context(\"spawning Cargo metadata for the API audit\")?;\n        workspace.require_copy_unchanged(expected)?;\n        workspace.require_cargo_inputs_unchanged()?;\n        compiler.require_unchanged()?;",
        breaks: "Cargo metadata can outlive the original API-audit source snapshot before its output is trusted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::api_audit_commands_rebind_original_and_copied_sources_before_verdicts",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    }\n    workspace.require_source_unchanged(paths.root())?;\n    plan.compiler.require_input_authority_unchanged()?;\n\n    uses.validate_dispatches(&inventory)?;\n    let dead = uses.dead(&inventory);",
        replace: "    }\n    plan.compiler.require_input_authority_unchanged()?;\n\n    uses.validate_dispatches(&inventory)?;\n    let dead = uses.dead(&inventory);",
        breaks: "the API-audit verdict can outlive the source snapshot it compiled",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::source_freshness_is_checked_after_compilation_and_before_the_verdict",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        let catalog = catalog.context(\"the API audit has no registered Cargo graph\")?;\n        compiler.require_input_authority_unchanged()?;\n        Self::from_catalog(catalog, compiler)",
        replace: "        let catalog = catalog.context(\"the API audit has no registered Cargo graph\")?;\n        Self::from_catalog(catalog, compiler)",
        breaks: "completed Cargo metadata is trusted without rechecking compiler-input authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::source_freshness_is_checked_after_compilation_and_before_the_verdict",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    workspace.require_cargo_inputs_unchanged()?;\n    plan.compiler.require_input_authority_unchanged()?;\n    let pristine_build_authority = (0..plan.compilations.len())",
        replace: "    workspace.require_cargo_inputs_unchanged()?;\n    let pristine_build_authority = (0..plan.compilations.len())",
        breaks: "the pristine compiler pass is accepted before its complete input authority is rechecked",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::source_freshness_is_checked_after_compilation_and_before_the_verdict",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        workspace.require_cargo_inputs_unchanged()?;\n        plan.compiler.require_target_libraries_unchanged()?;\n    }\n    workspace.require_source_unchanged(paths.root())?;",
        replace: "        workspace.require_cargo_inputs_unchanged()?;\n    }\n    workspace.require_source_unchanged(paths.root())?;",
        breaks: "instrumented compiler rounds are observed without rechecking target-library authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::source_freshness_is_checked_after_compilation_and_before_the_verdict",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    workspace.require_source_unchanged(paths.root())?;\n    plan.compiler.require_input_authority_unchanged()?;\n\n    uses.validate_dispatches(&inventory)?;",
        replace: "    workspace.require_source_unchanged(paths.root())?;\n\n    uses.validate_dispatches(&inventory)?;",
        breaks: "the final API verdict is issued without rechecking complete compiler-input authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::source_freshness_is_checked_after_compilation_and_before_the_verdict",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            return self.self_type == exact_self_type;",
        replace: "            return true;",
        breaks: "one static trait dispatch cross-products its member with the argument's self type",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::type_qualified_static_dispatch_uses_callee_not_argument_zero",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            let mut activated = false;\n            for (&(file, site), evidence) in &self.dispatches {",
        replace: "            let mut activated = false;\n            for activation in &inventory.trait_activations {\n                if activation.declaration.is_none()\n                    && live.contains(&activation.self_type)\n                    && live.insert(activation.implementation)\n                {\n                    pending.push_back(activation.implementation);\n                    activated = true;\n                }\n            }\n            for (&(file, site), evidence) in &self.dispatches {",
        breaks: "constructing a type activates its external trait body without a dispatch",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::constructing_a_type_does_not_activate_its_external_trait_body",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                        self.language_main && self.scope.is_empty() && self.lexical_item_depth == 0,",
        replace: "                        true,",
        breaks: "a module-local function named main becomes a production runtime root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_module_local_main_is_not_a_runtime_root",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            availability: context.availability.for_item(&module.attrs),",
        replace: "            availability: context.availability,",
        breaks: "an out-of-line module loses its parent's verification-only availability",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::out_of_line_modules_inherit_scope_and_parent_availability",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        scope.push(module.ident.to_string());",
        replace: "",
        breaks: "an out-of-line module loses its declared lexical scope",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::literal_path_modules_use_the_declared_scope",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            existing == &context,",
        replace: "            true || existing == &context,",
        breaks: "one physical source file can impersonate two module identities",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::one_source_file_cannot_have_conflicting_module_contexts",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                candidates.len() != 2,",
        replace: "                true || candidates.len() != 2,",
        breaks: "two default Rust module routes are resolved by filesystem order",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::ambiguous_default_module_routes_are_refused",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        let declared_path = literal_module_path(&module.attrs, source, module.ident.span())?;",
        replace: "        let declared_path = None;",
        breaks: "declared module path authority is ignored during source routing",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::unsafe_or_nonliteral_path_routes_are_refused",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                        self.language_main && self.scope.is_empty() && self.lexical_item_depth == 0,",
        replace: "                        self.language_main && self.scope.is_empty(),",
        breaks: "a nested function named main inherits its target's language-root authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::only_a_top_level_target_main_is_a_language_root",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "files[candidate.file].origin == files[reference.file].origin\n                    && candidate.symbol == name",
        replace: "candidate.symbol == name",
        breaks: "same-named types in other source files make a concrete trait implementation appear ambiguous",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::same_named_types_in_other_crates_do_not_ambiguate_trait_dispatch",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        Some(_) => ImportOrigin::Dependency,",
        replace: "        Some(_) => ImportOrigin::Other,",
        breaks: "external dependency trait imports lose their external authority classification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::dependency_trait_imports_do_not_authorize_local_aliases",
        },
    },
    Mutation {
        file: "kernel/src/mem.rs",
        find: "#[derive(Debug)]\npub(crate) struct ExternalControllersMasked {",
        replace: "#[derive(Debug, Default)]\npub(crate) struct ExternalControllersMasked {",
        breaks: "default construction forges external-controller mask authority without a hardware proof",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        && declaration_crate == crate_name",
        replace: "        && declaration_crate == \"not_a_crate\"",
        breaks: "qualified workspace trait implementations lose their declaring-crate binding",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::workspace_qualified_trait_implementations_bind_to_the_declaring_crate",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if prelude_external_trait(path) {\n            ImportOrigin::Standard",
        replace: "        if false && prelude_external_trait(path) {\n            ImportOrigin::Standard",
        breaks: "prelude trait recognition disappears beneath local glob imports",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::prelude_traits_remain_resolved_beneath_local_globs",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        imports.collect_names(items, &mut scope.to_vec());",
        replace: "",
        breaks: "local declarations cannot shadow same-named prelude traits",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::local_declarations_shadow_prelude_trait_names",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        } else if self.local_name(scope, first) {\n            ImportOrigin::Other",
        replace: "        } else if self.local_name(scope, first) {\n            ImportOrigin::Dependency",
        breaks: "crate-local modules and alias chains are classified as dependency traits",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::crate_modules_and_alias_chains_are_not_external_traits",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            || self.glob_binds_local(scope, first, &mut BTreeSet::new())",
        replace: "            || false",
        breaks: "local glob imports cannot shadow same-named prelude traits",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::local_globs_can_shadow_prelude_trait_names",
        },
    },
    Mutation {
        file: "xtask/src/cargo.rs",
        find: "            Prefix::VerbatimDisk(drive) => PathBuf::from(format!(\"{}:\\\\\", char::from(drive))),",
        replace: "            Prefix::VerbatimDisk(_) => return path.to_path_buf(),",
        breaks: "Cargo receives a Windows verbatim target path it cannot use",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::a_cargo_command_sheds_the_windows_verbatim_target_prefix",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    let path = self\n                        .trait_imports\n                        .resolve_path(&self.scope, &path)\n                        .unwrap_or(path);",
        replace: "                    let path = path;",
        breaks: "imported self-type aliases no longer resolve before trait-implementation binding",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::workspace_qualified_trait_implementations_bind_to_the_declaring_crate",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "source_item_path(&files[candidate.file], &candidate.symbol) == name",
        replace: "source_item_path(&files[candidate.file], &candidate.symbol) == \"not_a_type\"",
        breaks: "qualified concrete-type identity is discarded while binding workspace trait implementations",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::workspace_qualified_trait_implementations_bind_to_the_declaring_crate",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            .find(|(id, _)| active.contains(*id))",
        replace: "            .find(|_| false)",
        breaks: "an active procedural-macro dependency enters the API audit's syntactic trust boundary",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::active_procedural_macros_are_outside_the_audit_trust_boundary",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        _ => bail!(\"unsupported attribute `{name}`\"),",
        replace: "        _ => {}",
        breaks: "an unknown procedural attribute is accepted before API inventory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::procedural_attribute_and_derive_syntax_is_refused_before_inventory",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                let derive = single_path_name(&derive).context(\"qualified derive path\")?;",
        replace: "                let _procedural_derive = derive;\n                let derive = \"Clone\".to_string();",
        breaks: "a qualified procedural derive is accepted as a built-in derive",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::procedural_attribute_and_derive_syntax_is_refused_before_inventory",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        entry.mac.path.is_ident(\"macro_rules\")",
        replace: "        entry.mac.path.is_ident(\"not_macro_rules\")",
        breaks: "the exact exported wost entry declaration is no longer recognized",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::wost_entry_contract_binds_the_handler_to_the_linker_entry",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        expanded.to_string() == expected.to_string(),",
        replace: "        true,",
        breaks: "a changed wost entry expansion is accepted after its reviewed fragments still match",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::wost_entry_contract_binds_the_handler_to_the_linker_entry",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        entry_contract_use(&generated, \"boot::BootInfo\", \"wost::boot::BootInfo\")?,",
        replace: "        entry_contract_use(\n            &generated,\n            \"boot::limine_glue::base_revision\",\n            \"wost::boot::BootInfo\",\n        )?,",
        breaks: "the finite entry-contract use specification substitutes base_revision for BootInfo",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            || handler.segments[0].ident != \"kernel_main\"",
        replace: "",
        breaks: "a different kernel entry macro argument is trusted as kernel_main",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::exact_kernel_entry_reaches_its_handler_without_trusting_other_macro_tokens",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if transparent_macro(expression, self.unqualified_standard) {",
        replace: "        if false && transparent_macro(expression, self.unqualified_standard) {",
        breaks: "evaluated arguments of a guarded standard macro remain opaque to caller evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::macro_arguments_cannot_hide_dormant_lexical_callers",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    fn visit_item(&mut self, _item: &'ast Item) {\n        self.opaque = true;\n    }",
        replace: "    fn visit_item(&mut self, _item: &'ast Item) {}",
        breaks: "a nested item inside a standard macro argument manufactures lexical caller evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::macro_arguments_cannot_hide_dormant_lexical_callers",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                && unqualified_standard\n                && transparent_macro_name(name) =>",
        replace: "                && unqualified_standard =>",
        breaks: "arguments of an unknown unqualified macro are trusted as evaluated caller evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::macro_arguments_cannot_hide_dormant_lexical_callers",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            Some(name) if trusted_unqualified_statement_macro_name(&name.to_string()) => {",
        replace: "            Some(name) if false && trusted_unqualified_statement_macro_name(&name.to_string()) => {",
        breaks: "a locally defined standard macro name retains unqualified standard transparency",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::locally_shadowed_standard_macros_are_not_transparent_callers",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            None if !approved_item_macro_for_shadow_policy(item, self.context, self.relative) => {",
        replace: "            None if false && !approved_item_macro_for_shadow_policy(item, self.context, self.relative) => {",
        breaks: "an unknown item-position macro cannot shadow an unqualified standard macro across its target",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::an_item_macro_disables_unqualified_transparency_across_its_caller_target",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if !trusted_standard_statement_macro(&statement.mac) {",
        replace: "        if false && !trusted_standard_statement_macro(&statement.mac) {",
        breaks: "an unknown statement-position macro cannot shadow a later block-scoped standard macro name",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_statement_macro_can_manufacture_a_later_block_scoped_shadow",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    if name.starts_with(\"CARGO_\") {",
        replace: "    if false && name.starts_with(\"CARGO_\") {",
        breaks: "Cargo compiler-semantic environment variables escape the API audit boundary",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::compiler_semantic_environment_is_refused_before_the_audit",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        \"RUSTC\"\n            | \"RUSTC_WRAPPER\"",
        replace: "        \"IGNORED_RUSTC\"\n            | \"RUSTC_WRAPPER\"",
        breaks: "the selected Rust compiler can be replaced through the API audit environment",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::compiler_semantic_environment_is_refused_before_the_audit",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    ImplicitTraitTrigger::ConstructedType(ty) => self\n                        .constructions\n                        .iter()\n                        .any(|(caller, types)| live.contains(caller) && types.contains(&ty)),",
        replace: "                    ImplicitTraitTrigger::ConstructedType(ty) => live.contains(&ty),",
        breaks: "a type-only or associated-item use activates that type's standard Drop body",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::reviewed_nominal_drop_types_activate_only_from_exact_constructor_spans",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                        .any(|(caller, types)| live.contains(caller) && types.contains(&ty)),",
        replace: "                        .any(|(_, types)| types.contains(&ty)),",
        breaks: "a construction under a dormant caller activates the constructed type's Drop body",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::reviewed_nominal_drop_types_activate_only_from_exact_constructor_spans",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                        .filter(|(_, allocator_type)| *allocator_type == self_type)",
        replace: "                        .filter(|_| true)",
        breaks: "a global allocator static activates GlobalAlloc bodies for unrelated nominal types",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_global_allocator_activates_only_its_exact_global_alloc_implementation",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    Lit::Str(value) => bytes_contain_diagnostic_marker(value.value().as_bytes()),",
        replace: "                    Lit::Str(value) => {\n                        let _unchecked_decoded_literal = value;\n                        false\n                    }",
        breaks: "an escaped Rust literal can author a reserved API-audit diagnostic identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::decoded_literals_cannot_forge_a_compiler_diagnostic_marker",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            TokenTree::Group(group) => {\n                if token_stream_contains_diagnostic_marker(group.stream())? {\n                    return Ok(true);\n                }\n            }",
        replace: "            TokenTree::Group(group) => {\n                if group.delimiter() != Delimiter::Brace\n                    && token_stream_contains_diagnostic_marker(group.stream())?\n                {\n                    return Ok(true);\n                }\n            }",
        breaks: "an escaped reserved diagnostic identity can hide inside a macro body",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::decoded_literals_cannot_forge_a_compiler_diagnostic_marker",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        reject_authority_diagnostic_marker(\n            &directory.join(relative),\n            &bytes,\n            \"authenticated registry compiler input\",\n        )?;",
        replace: "        let _unchecked_registry_marker = (&bytes, relative);",
        breaks: "checksum-authenticated registry source can mint a reserved compiler diagnostic marker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::authenticated_registry_source_cannot_mint_a_diagnostic_marker",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                snapshot_marker_free_file(\n                    root,\n                    &entry.path(),\n                    snapshot,\n                    \"build-script compiler input\",\n                )?;",
        replace: "                snapshot_file(root, &entry.path(), snapshot)?;",
        breaks: "pristine build-script output is SHA-bound without rejecting a reserved diagnostic marker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::pristine_generated_source_cannot_mint_a_diagnostic_marker",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        validate_active_external_package_roots(registry_source, package_values, &members, &active)?;",
        replace: "        let _unchecked_external_roots =\n            (registry_source, package_values, &members, &active);",
        breaks: "an active path package outside the checksum-bound registry enters compiler authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::active_path_packages_are_outside_the_compiler_input_authority",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    snapshot_compiler_target_runtime(sysroot, targets, &mut snapshot)?;\n    Ok(snapshot)",
        replace: "    Ok(snapshot)",
        breaks: "selected compiler target runtime and codegen backends are omitted from marker and digest authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::compiler_binaries_and_target_metadata_cannot_mint_a_diagnostic_marker",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    reject_authority_diagnostic_marker(path, &bytes, label)?;",
        replace: "    if label != \"compiler target library authority\" {\n        reject_authority_diagnostic_marker(path, &bytes, label)?;\n    }",
        breaks: "precompiled target-library metadata is digested without rejecting a reserved diagnostic marker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::compiler_binaries_and_target_metadata_cannot_mint_a_diagnostic_marker",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    if current == expected {\n        return Ok(());\n    }",
        replace: "    if true {\n        return Ok(());\n    }",
        breaks: "changed compiler runtime or target-library bytes retain their captured authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::compiler_binaries_and_target_metadata_cannot_mint_a_diagnostic_marker",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            !kernel_item_binds_wost_root(item),",
        replace: "            true,",
        breaks: "a kernel-local item can rebind the trusted wost entry dependency root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::exact_kernel_entry_reaches_its_handler_without_trusting_other_macro_tokens",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            && !exact_kernel_entry_invocation(item)",
        replace: "            && false\n            && !exact_kernel_entry_invocation(item)",
        breaks: "an opaque kernel root item macro can manufacture the trusted wost binding",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::exact_kernel_entry_reaches_its_handler_without_trusting_other_macro_tokens",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            if enabled && RESERVED_LANGUAGE_ROOTS.contains(&dependency.name.as_str()) {",
        replace: "            if false && enabled && RESERVED_LANGUAGE_ROOTS.contains(&dependency.name.as_str()) {",
        breaks: "an active Cargo dependency can replace a reserved language crate root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::active_dependencies_cannot_replace_language_crate_roots",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            || reserved_language_root(&binding) && !reserved_language_root(&source)",
        replace: "            || reserved_language_root(&binding)\n                && false\n                && !reserved_language_root(&source)",
        breaks: "an extern crate alias can impersonate a reserved language root for macro resolution",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::noncanonical_language_root_bindings_disable_macro_transparency",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        UseTree::Rename(rename) => {\n            let binding = rename.rename.to_string();\n            let source = rename.ident.to_string();\n            let source_root = root.unwrap_or(&source);\n            if (trusted_unqualified_statement_macro_name(&binding)\n                || reserved_language_root(&binding))\n                && !reserved_language_root(source_root)",
        replace: "        UseTree::Rename(rename) => {\n            let binding = rename.rename.to_string();\n            let source = rename.ident.to_string();\n            let source_root = root.unwrap_or(&source);\n            if (trusted_unqualified_statement_macro_name(&binding)\n                || reserved_language_root(&binding))\n                && false\n                && !reserved_language_root(source_root)",
        breaks: "a use rename can impersonate a reserved language root for macro resolution",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::noncanonical_language_root_bindings_disable_macro_transparency",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                        _ => ImportOrigin::Dependency,",
        replace: "                        _ => ImportOrigin::Standard,",
        breaks: "a dependency crate alias can impersonate an implicit standard trait root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::dependency_roots_cannot_forge_implicit_standard_traits",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    let column = line\n        .char_indices()\n        .map(|(offset, _)| offset)",
        replace: "    let column = line\n        .bytes()\n        .enumerate()\n        .map(|(offset, _)| offset)",
        breaks: "a compiler character column is mistaken for a UTF-8 byte column during instrumentation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::unicode_columns_instrument_the_exact_nested_item",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            !nodes[candidates[0]].type_alias,",
        replace: "            true,",
        breaks: "a global allocator type alias enters implicit trait activation without exact nominal identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_global_allocator_type_alias_is_refused_without_exact_impl_identity",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    REVIEWED_IMPLICIT_DROP_TYPES\n                        .iter()\n                        .any(|(relative, symbol)| {\n                            file.relative == Path::new(relative) && declaration.symbol == *symbol\n                        }),",
        replace: "                    true,",
        breaks: "an unreviewed nominal type enters the API audit's implicit Drop boundary",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_new_implicit_drop_contract_is_refused_until_reviewed",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            self.sites.push(ValueConstruction {\n                range: range(segment.ident.span()),\n            });",
        replace: "            self.sites.push(ValueConstruction {\n                range: range(path.span()),\n            });",
        breaks: "a type qualifier on an associated item is accepted as a nominal value construction",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::reviewed_nominal_drop_types_activate_only_from_exact_constructor_spans",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    fn visit_item_static(&mut self, _item: &'ast syn::ItemStatic) {}",
        replace: "    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {\n        visit::visit_item_static(self, item);\n    }",
        breaks: "a static initializer is mistaken for a live construction of its implicit Drop type",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::reviewed_nominal_drop_types_activate_only_from_exact_constructor_spans",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        package == crate::paths::KERNEL_PACKAGE && target.relative == Path::new(\"kernel/build.rs\"),",
        replace: "        false && package == crate::paths::KERNEL_PACKAGE\n            && target.relative == Path::new(\"kernel/build.rs\"),",
        breaks: "the exact first-party kernel linker build script falls outside its only audited admission",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::only_the_exact_kernel_linker_build_script_is_trusted",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        actual == expected,\n        \"kernel build script performs work outside its exact linker contract\"",
        replace: "        actual != expected,\n        \"kernel build script performs work outside its exact linker contract\"",
        breaks: "the exact kernel linker build-script token contract is no longer the admitted program",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::only_the_exact_kernel_linker_build_script_is_trusted",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            !text.contains(\"harness\") && !text.contains(\"\\\\u\") && !text.contains(\"\\\\U\"),",
        replace: "            true,",
        breaks: "a manifest can select a custom harness outside the API audit's target model",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_macro_generated_custom_harness_main_is_refused_from_its_manifest",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    !conditional_semantic_root(attribute),",
        replace: "                    true,",
        breaks: "cfg_attr can conditionally hide a semantic runtime root from inventory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cfg_attr_cannot_hide_a_semantic_runtime_root",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            && (at.line < self.end.line || at.line == self.end.line && at.column < self.end.column)",
        replace: "            && (at.line < self.end.line || at.line == self.end.line && at.column <= self.end.column)",
        breaks: "a half-open source range absorbs the first column after its nested item",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::an_adjacent_call_is_outside_a_half_open_nested_item_span",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    let active_scripts = scripts\n        .iter()\n        .filter(|(id, _)| active.contains(*id))\n        .map(|(_, script)| script)\n        .collect::<Vec<_>>();",
        replace: "    let active_scripts: Vec<&DependencyBuildScript> = Vec::new();",
        breaks: "active dependency build scripts disappear from API-audit authority review",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::unreviewed_dependency_build_scripts_are_outside_the_audit_authority",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        build_script_authority_snapshot(root)? == *expected,",
        replace: "        build_script_authority_snapshot(root)? == *expected || true,",
        breaks: "instrumented compiler rounds can change build-script output authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::instrumented_rounds_cannot_change_build_script_output_authority",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        diagnostic.origin.package_id == expected.package_id\n            && diagnostic.origin.target_name == expected.name\n            && diagnostic.origin.target_kind == expected.kind\n            && reported_source == expected_source,",
        replace: "        true,",
        breaks: "a compiler diagnostic from another Cargo package and target can donate API liveness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cargo_diagnostic_origins_bind_markers_to_the_exact_package_and_target",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    inventory.is_opaque_macro_use(expansion_file, at),",
        replace: "                    true,",
        breaks: "a diagnostic at a non-macro expansion callsite is discarded as opaque",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::core_offset_of_diagnostics_are_discarded_at_the_terminal_opaque_callsite",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    expansion.terminal,\n                    \"an API diagnostic for item {} has an external primary span with a nested expansion chain\",",
        replace: "                    true,\n                    \"an API diagnostic for item {} has an external primary span with a nested expansion chain\",",
        breaks: "an external primary diagnostic traverses a nonterminal nested expansion chain",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::core_offset_of_diagnostics_are_discarded_at_the_terminal_opaque_callsite",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                        crate::json::JsonValue::Object(_) => false,",
        replace: "                        crate::json::JsonValue::Object(_) => bail!(\"primary API diagnostic has a nested expansion chain\"),",
        breaks: "a classified primary diagnostic is rejected for retaining its lexical nested expansion policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::core_offset_of_diagnostics_are_discarded_at_the_terminal_opaque_callsite",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            .find(|contract| contract.id == used)?;",
        replace: "            .find(|contract| contract.primary.contains(primary_at))?;",
        breaks: "a non-contract entry diagnostic can borrow the reviewed cross-crate origin bridge",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        (contract.primary.contains(primary_at)\n            && declaration.file == primary_file",
        replace: "        (true\n            && declaration.file == primary_file",
        breaks: "an entry-contract identity can borrow another reviewed identity's primary use site",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            && expansion.terminal\n            && expansion.macro_decl_name == \"::wost::entry!\"",
        replace: "            && true\n            && expansion.macro_decl_name == \"::wost::entry!\"",
        breaks: "a nonterminal entry expansion chain receives cross-crate bridge authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            && expansion.macro_decl_name == \"::wost::entry!\"",
        replace: "            && true",
        breaks: "an unrelated macro expansion impersonates the reviewed wost entry bridge",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            && expansion_file == entry.file\n            && expansion_at == entry.invocation.start)",
        replace: "            && true\n            && expansion_at == entry.invocation.start)",
        breaks: "an entry expansion in another source file inherits the reviewed invocation identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            && expansion_at == entry.invocation.start)\n            .then_some(entry.invocation_node)",
        replace: "            && true)\n            .then_some(entry.invocation_node)",
        breaks: "an entry expansion at another callsite inherits the reviewed invocation identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    validate_cargo_diagnostic_origin(\n                        diagnostic,\n                        inventory,\n                        root,\n                        expansion_file,\n                        expansion_source,\n                    )?;\n                    Some(caller)",
        replace: "                    let _unchecked_entry_origin =\n                        (diagnostic, inventory, root, expansion_file, expansion_source);\n                    Some(caller)",
        breaks: "an authorized entry path accepts a compiler origin unrelated to its expansion target",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    fn exact_entry_invocation_caller(\n        &self,\n        file: usize,\n        at: LineColumn,\n        used: usize,\n    ) -> Option<usize> {\n        let entry = self.entry?;",
        replace: "    fn exact_entry_invocation_caller(\n        &self,\n        file: usize,\n        at: LineColumn,\n        used: usize,\n    ) -> Option<usize> {\n        let entry = self.entry?;\n        let declaration = &self.nodes[entry.macro_declaration];\n        if declaration.file == file && declaration.range.contains(at) {\n            return Some(entry.invocation_node);\n        }",
        breaks: "a same-wost macro-definition diagnostic receives generic declaration-side entry credit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    fn discover_with_targets(root: &Path, targets: &[CargoTarget]) -> Result<Self> {\n        Self::discover_with_targets_and_entry_authority(root, targets, None)\n    }",
        replace: "    fn discover_with_targets(root: &Path, targets: &[CargoTarget]) -> Result<Self> {\n        let authority = EntryContractAuthority {\n            uses: validate_wost_entry_contract(root)?,\n        };\n        Self::discover_with_targets_and_entry_authority(root, targets, Some(&authority))\n    }",
        breaks: "raw target discovery gains declaration-side entry authority without the production catalog",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                        macro_decl_name: macro_decl_name.to_string(),",
        replace: "                        macro_decl_name: \"::wost::entry!\".to_string(),",
        breaks: "structured diagnostics rewrite every expansion's macro identity as wost entry",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::core_offset_of_diagnostics_are_discarded_at_the_terminal_opaque_callsite",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            && expansion_at == entry.invocation.start)\n            .then_some(entry.invocation_node)\n    }\n\n    fn is_opaque_macro_use",
        replace: "            && expansion_at == entry.invocation.start)\n            .then_some(entry.invocation_node)\n            .filter(|_| false)\n    }\n\n    fn is_opaque_macro_use",
        breaks: "the exact compiler-proven cross-crate entry bridge refuses its reviewed fixture",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            policy.reserved_root_rebinding.is_none(),",
        replace: "            true,",
        breaks: "a rebound core root can forge the otherwise approved global_asm boundary",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_rebound_core_root_cannot_forge_the_approved_global_asm_macro",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    trait_qualifier: path.qself.as_ref().and(trait_qualifier),",
        replace: "                    trait_qualifier: trait_qualifier.or_else(|| {\n                        direct_path_prefix(\n                            &path.path,\n                            path.path.segments.len().saturating_sub(1),\n                        )\n                    }),",
        breaks: "a plain type-qualified associated call is mistaken for an explicit trait dispatch",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::type_qualified_static_dispatch_uses_callee_not_argument_zero",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        match matching.as_slice() {\n            [activation] => DispatchResolution::Implementation(activation.implementation),\n            _ => DispatchResolution::Failure(matching.len()),\n        }",
        replace: "        match matching.as_slice() {\n            [activation, ..] => DispatchResolution::Implementation(activation.implementation),\n            [] => DispatchResolution::Failure(0),\n        }",
        breaks: "an ambiguous live trait dispatch is accepted when any implementation matches",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::ambiguous_variable_receiver_dispatch_fails_closed",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            if let DispatchResolution::Failure(implementations) =\n                self.dispatch_resolution(inventory, dispatch, evidence)\n            {\n                failures.insert(DispatchFailure {\n                    file,\n                    site,\n                    implementations,\n                });\n            }",
        replace: "            if let DispatchResolution::Failure(implementations) =\n                self.dispatch_resolution(inventory, dispatch, evidence)\n                && failures.is_empty()\n            {\n                failures.insert(DispatchFailure {\n                    file,\n                    site,\n                    implementations,\n                });\n            }",
        breaks: "only the first invalid dispatch in a compiler graph reaches the refusal report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_invalid_dispatch_in_one_graph_is_reported",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        for (graph_index, graph) in self.graphs.iter().enumerate() {",
        replace: "        for (graph_index, graph) in self.graphs.iter().take(1).enumerate() {",
        breaks: "an earlier compiler graph prevents later invalid dispatches from reaching the refusal report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_later_compiler_graph_cannot_hide_its_invalid_dispatch",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if !self.call_compatible(site, callee_nodes) || !self.exact_self_type {",
        replace: "        if !self.call_compatible(site, callee_nodes) {",
        breaks: "an erased generic self type can activate an unrelated trait specialization",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_generic_variable_receiver_dispatch_fails_closed",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if has_trait_declaration_evidence\n            && !self\n                .declaration\n                .is_some_and(|declaration| callee_nodes.contains(&declaration))\n        {",
        replace: "        if false\n            && has_trait_declaration_evidence\n            && !self\n                .declaration\n                .is_some_and(|declaration| callee_nodes.contains(&declaration))\n        {",
        breaks: "an explicitly named trait dispatch activates a same-member implementation from another trait",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::unique_external_method_dispatch_activates_only_its_exact_trait",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    uses.validate_dispatches(&inventory)?;\n    let dead = uses.dead(&inventory);",
        replace: "    let dead = uses.dead(&inventory);",
        breaks: "the authoritative API verdict never validates its recorded trait dispatches",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::authoritative_verdict_includes_every_compiled_graph",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    implicit_activations.push(ImplicitTraitActivation {\n                        trigger: ImplicitTraitTrigger::LiveNode(producer_node),\n                        implementation: node.id,\n                    });",
        replace: "                    let _unchecked_reviewed_producer_trigger = (producer_node, node.id);",
        breaks: "a live reviewed owned-Self producer no longer activates its result type's Drop body",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_reviewed_owned_self_producer_activates_drop_without_type_use_shortcuts",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                            recorded.reviewed_drop_producer =\n                                implementing_type.as_deref().and_then(|owner| {\n                                    reviewed_drop_producer(\n                                        self.relative,\n                                        owner,\n                                        &member.sig.ident.to_string(),\n                                        implemented_trait.as_deref(),\n                                    )\n                                });",
        replace: "                            recorded.reviewed_drop_producer = None;",
        breaks: "reviewed Drop producers no longer resolve to their current production nodes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_reviewed_drop_producer_resolves_in_the_current_source",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    candidate.direct_self_construction\n                        && candidate.reviewed_drop_producer.is_none()\n                        && matching_type_nodes(nodes, files, candidate, owner).contains(&self_type)",
        replace: "                    candidate.direct_self_construction\n                        && candidate.reviewed_drop_producer.is_none()\n                        && candidate.file == declaration.file\n                        && matching_type_nodes(nodes, files, candidate, owner).contains(&self_type)",
        breaks: "an unreviewed owned-Self Drop producer escapes review when declared in another source file",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_cross_file_owned_self_drop_producer_is_refused_until_reviewed",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                    unreviewed_producer.is_none(),",
        replace: "                    true,",
        breaks: "a new same-file owned-Self Drop producer enters the implicit activation boundary without review",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_new_owned_self_drop_producer_is_refused_until_reviewed",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            matching_type_nodes(nodes, files, producers[0], producer_owner) == vec![produced_type],",
        replace: "            true,",
        breaks: "a reviewed Drop producer can belong to a different nominal result type",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_reviewed_drop_producer_must_belong_to_its_exact_result_type",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    if type_path.qself.is_some()\n        || type_path\n            .path\n            .segments\n            .iter()\n            .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))\n    {\n        return None;\n    }",
        replace: "    if type_path.qself.is_some() {\n        return None;\n    }",
        breaks: "generic GlobalAlloc implementations and their allocator static collapse to one nominal identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::generic_global_allocator_identities_are_refused_before_type_arguments_can_collapse",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    let compiler_name = if cfg!(windows) { \"rustc.exe\" } else { \"rustc\" };",
        replace: "    let compiler_name = if cfg!(windows) { \"cargo.exe\" } else { \"cargo\" };",
        breaks: "Cargo is mistaken for the exact adjacent Rust compiler used by the API audit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_cargo_subprocess_pins_the_adjacent_compiler_and_disables_wrappers",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if normalized.starts_with(\"CARGO_\")\n            || normalized.starts_with(\"RUST\")\n            || normalized == \"TARGET\"\n        {",
        replace: "        if false\n            && (normalized.starts_with(\"CARGO_\")\n                || normalized.starts_with(\"RUST\")\n                || normalized == \"TARGET\")\n        {",
        breaks: "ambient Cargo and Rust variables survive into a supposedly isolated subprocess",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::cargo_commands_remove_ambient_rust_and_cargo_semantics_before_pinning_their_own",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        .env(\"RUSTC\", &compiler.path)",
        replace: "        .env(\"RUSTC\", env!(\"CARGO\"))",
        breaks: "an audited Cargo subprocess invokes a compiler other than its authenticated sibling rustc",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_cargo_subprocess_pins_the_adjacent_compiler_and_disables_wrappers",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        .env(\"RUSTC_WRAPPER\", \"\")\n        .env(\"RUSTC_WORKSPACE_WRAPPER\", \"\")",
        replace: "        .env(\"RUSTC_WRAPPER\", \"cargo\")\n        .env(\"RUSTC_WORKSPACE_WRAPPER\", \"cargo\")",
        breaks: "audited Cargo subprocesses retain compiler wrappers capable of replacing rustc",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_cargo_subprocess_pins_the_adjacent_compiler_and_disables_wrappers",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        .env(\"CARGO_HOME\", cargo_home)",
        replace: "        .env(\"CARGO_HOME\", execution_root)",
        breaks: "an audited Cargo subprocess escapes its private configuration home",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_cargo_subprocess_pins_the_adjacent_compiler_and_disables_wrappers",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        .env(\"PATH\", &compiler.tool_path)",
        replace: "        .env(\"PATH\", execution_root)",
        breaks: "an audited Cargo subprocess searches outside the authenticated compiler tool directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_cargo_subprocess_pins_the_adjacent_compiler_and_disables_wrappers",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            compiler.host_linker.to_string_lossy()",
        replace: "            compiler.path.to_string_lossy()",
        breaks: "host Cargo graphs receive rustc itself instead of the authenticated host linker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_cargo_subprocess_pins_the_adjacent_compiler_and_disables_wrappers",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        .arg(format!(\"source.api-audit-registry.directory={registry}\"));",
        replace: "        .arg(format!(\"source.untrusted-registry.directory={registry}\"));",
        breaks: "the audited Cargo source replacement no longer binds its private materialized registry",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::every_cargo_subprocess_pins_the_adjacent_compiler_and_disables_wrappers",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            crate::sha256::hex(&crate::sha256::digest(&bytes)) == digest,\n            \"materialized registry file `{relative}` differs from its authenticated archive\"",
        replace: "            true,\n            \"materialized registry file `{relative}` differs from its authenticated archive\"",
        breaks: "tampered materialized registry bytes retain checksum-authenticated Cargo authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::checksum_verified_archives_form_an_immutable_isolated_cargo_source",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if listed > MAX_TABLES {",
        replace: "        if false {",
        breaks: "an over-budget ACPI root table can hide a tail pointer",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_root_table_above_the_cap_cannot_hide_a_tail_madt",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if listed > MAX_TABLES {",
        replace: "        if listed >= MAX_TABLES {",
        breaks: "an ACPI root table at the exact pointer budget is refused",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_root_table_at_the_cap_is_walked_whole",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "            8 => u64_at(self.body, self.offset)?,",
        replace: "            8 => u32_at(self.body, self.offset)? as u64,",
        breaks: "XSDT iteration truncates every table pointer to thirty-two bits",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::an_xsdt_pointer_keeps_its_high_thirty_two_bits",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if self.revision >= 2 && self.xsdt != 0 {",
        replace: "        if self.revision >= 2 || self.xsdt != 0 {",
        breaks: "an XSDT pointer is published without both revision and nonzero-address proof",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::an_xsdt_requires_both_a_supporting_revision_and_a_nonzero_address",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "                enabled: u32_at(entry, 4).unwrap_or(0) & 1 != 0,",
        replace: "                enabled: u32_at(entry, 4).unwrap_or(0) & 1 == 0,",
        breaks: "a disabled local APIC is reported enabled and an enabled one disabled",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_local_apic_without_the_enabled_bit_is_disabled",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    if bus != 0 {",
        replace: "                    if false {",
        breaks: "a non-ISA interrupt override enters the ISA routing inventory",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::interrupt_overrides_are_unique_isa_sources_with_defined_flags",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    let Some(slot) = model.overrides.get_mut(source as usize) else {",
        replace: "                    let Some(slot) = model.overrides.get_mut((source as usize).min(15)) else {",
        breaks: "an interrupt override outside the ISA source domain aliases a valid source",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::interrupt_overrides_are_unique_isa_sources_with_defined_flags",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    if flags & !0xf != 0 || polarity == 0b10 || trigger == 0b10 {",
        replace: "                    if false {",
        breaks: "reserved interrupt polarity and trigger encodings enter the routing inventory",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::interrupt_overrides_are_unique_isa_sources_with_defined_flags",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    if slot.is_some() {",
        replace: "                    if false {",
        breaks: "a later interrupt override silently replaces the same ISA source",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::interrupt_overrides_are_unique_isa_sources_with_defined_flags",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "pub(super) unsafe fn set_page_table_base(address: u64) {",
        replace: "pub(crate) unsafe fn set_page_table_base(address: u64) {",
        breaks: "the page-table-base instruction door is visible beyond its architecture module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "pub(super) unsafe fn save(area: *mut u8) {",
        replace: "pub(crate) unsafe fn save(area: *mut u8) {",
        breaks: "the extended-state save instruction door is visible beyond its architecture module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "pub(super) unsafe fn restore(area: *const u8) {",
        replace: "pub(crate) unsafe fn restore(area: *const u8) {",
        breaks: "the extended-state restore instruction door is visible beyond its architecture module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "fn write_thread_base(base: CanonicalAddress) {",
        replace: "pub(super) fn write_thread_base(base: CanonicalAddress) {",
        breaks: "the checked thread-base writer is visible outside its sole caller module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "pub(super) unsafe fn set_page_table_base(address: u64) {",
        replace: "pub(super) fn set_page_table_base(address: u64) {",
        breaks: "the page-table-base raw instruction loses its explicit unsafe contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "pub(super) unsafe fn save(area: *mut u8) {",
        replace: "pub(super) fn save(area: *mut u8) {",
        breaks: "the extended-state save instruction loses its explicit unsafe contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "pub(super) unsafe fn restore(area: *const u8) {",
        replace: "pub(super) fn restore(area: *const u8) {",
        breaks: "the extended-state restore instruction loses its explicit unsafe contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "fn write_thread_base(base: CanonicalAddress) {",
        replace: "unsafe fn write_thread_base(base: CanonicalAddress) {",
        breaks: "the checked thread-base writer becomes a second raw instruction door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/msr.rs",
        find: "pub(super) unsafe fn write(msr: u32, value: u64) {",
        replace: "pub unsafe fn write(msr: u32, value: u64) {",
        breaks: "an unreviewed unrestricted public unsafe instruction door enters wost",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    xstate::user_state_is_isolated()",
        replace: "    xstate::is_enabled()",
        breaks: "ring three trusts remembered extended-state enablement instead of the live isolation predicate",
        reddens: Caught::Check("missing extended state closes the user boundary"),
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "    ensure!(\n        Report::parse(text)? == *selected,\n        \"mutation report changed while selecting the current pass\"\n    );",
        replace: "    let _unchecked_selected_report = (selected, text);",
        breaks: "the current mutation pass digest can bind bytes from a substituted report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_current_pass_cannot_bind_bytes_from_a_substituted_report",
        },
    },
    Mutation {
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
        file: "wost/src/arch/x86_64/apic.rs",
        find: "pub struct EoiToken<'frame> {\n    vector: InterruptVector,\n    frame: &'frame TrapFrame,\n    interrupts: Option<interrupt::InterruptState>,\n}\n\nimpl fmt::Debug for EoiToken<'_> {",
        replace: "pub struct EoiToken<'frame> {\n    vector: InterruptVector,\n    frame: &'frame TrapFrame,\n    interrupts: Option<interrupt::InterruptState>,\n}\n\nimpl Clone for EoiToken<'_> {\n    fn clone(&self) -> Self {\n        Self {\n            vector: self.vector,\n            frame: self.frame,\n            interrupts: None,\n        }\n    }\n}\n\nimpl fmt::Debug for EoiToken<'_> {",
        breaks: "an APIC completion authority can be cloned while its original remains live",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_completion_and_disarm_authorities_are_linear",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/apic.rs",
        find: "#[derive(Debug)]\npub struct DisarmedTimer {",
        replace: "#[derive(Debug, Clone)]\npub struct DisarmedTimer {",
        breaks: "a timer-disarm proof can be cloned without satisfying Copy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_completion_and_disarm_authorities_are_linear",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "#[derive(Debug)]\npub struct Shared(Arc<SharedAllocation>);",
        replace: "#[derive(Debug, Clone)]\npub struct Shared(Arc<SharedAllocation>);",
        breaks: "an unpartitioned shared allocation can be cloned into overlapping access modes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::shared_memory_access_requires_one_partition",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "#[derive(Debug)]\npub struct SharedBytes(Arc<SharedAllocation>);",
        replace: "#[derive(Debug, Clone)]\npub struct SharedBytes(Arc<SharedAllocation>);",
        breaks: "a volatile-byte shared-memory partition can be duplicated",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::shared_memory_access_requires_one_partition",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "#[derive(Debug)]\npub struct SharedRing<const WORDS: usize> {",
        replace: "#[derive(Debug, Clone)]\npub struct SharedRing<const WORDS: usize> {",
        breaks: "a ring-partitioned shared allocation can be duplicated",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::shared_memory_access_requires_one_partition",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\npub struct DmaState {",
        replace: "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct DmaState {",
        breaks: "DMA ownership state can be copied as well as cloned",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(Debug)]\npub struct DmaPublication<'a> {",
        replace: "#[derive(Debug, Clone)]\npub struct DmaPublication<'a> {",
        breaks: "a device-publication proof can be cloned while its source remains live",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(Debug)]\npub struct DmaPublication<'a> {",
        replace: "#[derive(Debug, Clone, Copy)]\npub struct DmaPublication<'a> {",
        breaks: "a device-publication proof can be copied without consuming its authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        file: "xtask/src/image/mod.rs",
        find: "        build(&staging, &mut disk)?;",
        replace: "        let _ignored_construction_failure = build(&staging, &mut disk);",
        breaks: "a failed boot-image construction can publish its partial stage over the complete image",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::a_failed_authoritative_image_stage_preserves_the_complete_image",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    let line = line.strip_suffix('\\n')?;",
        replace: "    let line = line.strip_suffix('\\n').unwrap_or(line);",
        breaks: "an unterminated serial fragment can satisfy exact boot evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::required_boot_records_refuse_framing_and_message_near_misses",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    if text.contains('\\r') || text.contains('\\n') {\n        return None;\n    }",
        replace: "",
        breaks: "an internal carriage return can enter a framed panic transcript",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "            Self::Warning => \"[warn ] \",",
        replace: "            Self::Warning => \"[info ] \",",
        breaks: "warning evidence is accepted with info-record framing",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::required_boot_records_refuse_framing_and_message_near_misses",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "fn is_required_boot_marker(line: &str, record: RequiredLogRecord) -> bool {\n    if record == NO_X2APIC_RECORD {\n        return is_exact_no_x2apic_diagnosis(line);\n    }\n    is_exact_log_message(line, record.severity, record.message)\n}",
        replace: "fn is_required_boot_marker(line: &str, record: RequiredLogRecord) -> bool {\n    if record.message == DEGRADED_FILESYSTEM_MARKER {\n        return line.contains(\"tmpfs:\");\n    }\n    if record == NO_X2APIC_RECORD {\n        return is_exact_no_x2apic_diagnosis(line);\n    }\n    is_exact_log_message(line, record.severity, record.message)\n}",
        breaks: "a tmpfs failure warning can forge the degraded filesystem success evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::failure_records_cannot_satisfy_required_boot_evidence",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        .strip_prefix(SETTLED_MARKER)",
        replace: "        .split_once(SETTLED_MARKER)\n        .map(|(_, uptime)| uptime)",
        breaks: "text before the steady-state message envelope is accepted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::steady_state_requires_one_exact_dynamic_info_record",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        .and_then(|uptime| uptime.strip_suffix(\" ms\"))",
        replace: "        .and_then(|uptime| uptime.split_once(\" ms\").map(|(uptime, _)| uptime))",
        breaks: "text after the steady-state uptime suffix is accepted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::steady_state_requires_one_exact_dynamic_info_record",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    if text.is_empty()\n        || (text.len() > 1 && text.starts_with('0'))\n        || !text.bytes().all(|byte| byte.is_ascii_digit())\n    {",
        replace: "    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {",
        breaks: "a noncanonical leading-zero uptime satisfies steady state",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::steady_state_requires_one_exact_dynamic_info_record",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    if text.is_empty()\n        || (text.len() > 1 && text.starts_with('0'))\n        || !text.bytes().all(|byte| byte.is_ascii_digit())\n    {",
        replace: "    if text.is_empty() || (text.len() > 1 && text.starts_with('0')) {",
        breaks: "a signed uptime satisfies the canonical decimal grammar",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::steady_state_requires_one_exact_dynamic_info_record",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    <u64 as core::str::FromStr>::from_str(text).ok()",
        replace: "    Some(<u64 as core::str::FromStr>::from_str(text).unwrap_or(u64::MAX))",
        breaks: "an overflowing uptime satisfies the steady-state record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::steady_state_requires_one_exact_dynamic_info_record",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "            if is_settled_record(&text) {",
        replace: "            if text.contains(SETTLED_MARKER) {",
        breaks: "the serial watcher publishes steady state from a marker substring",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "                let present = is_required_boot_marker(&text, *record);",
        replace: "                let present = text.contains(record.message);",
        breaks: "the serial watcher bypasses exact required-record matching",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(NVME_FALLBACK_MARKER),",
        replace: "",
        breaks: "the shipped boot can settle without proving first-candidate NVMe retirement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(QEMU_TCG_ISOLATION_MARKER),\n    NO_X2APIC_RECORD,",
        replace: "const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    NO_X2APIC_RECORD,",
        breaks: "the degraded boot can settle without proving its QEMU TCG policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::warning(NO_PCI_MARKER),",
        replace: "    RequiredLogRecord::info(NO_PCI_MARKER),",
        breaks: "the degraded no-PCI diagnosis is enrolled at the wrong severity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(DEGRADED_FILESYSTEM_MARKER),",
        replace: "    RequiredLogRecord::info(\"tmpfs:\"),",
        breaks: "the degraded filesystem requirement is only a broad tmpfs prefix",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(DEGRADED_STORAGE_MARKER),",
        replace: "    RequiredLogRecord::info(\"devfs: storage is\"),",
        breaks: "the degraded storage requirement omits its exact device description",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(DEGRADED_CONSOLE_MARKER),",
        replace: "    RequiredLogRecord::info(\"devfs: console is\"),",
        breaks: "the degraded console requirement omits its exact device description",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(DEGRADED_PROTECTIONS_MARKER),",
        replace: "    RequiredLogRecord::info(\"cpu: protections\"),",
        breaks: "the degraded protection requirement proves neither mandatory nor absent features",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::warning(DEGRADED_XSTATE_MARKER),",
        replace: "    RequiredLogRecord::warning(\"cpu: extended state unavailable\"),",
        breaks: "the degraded extended-state requirement omits the user-boundary refusal",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::warning(DEGRADED_USER_MARKER),",
        replace: "    RequiredLogRecord::warning(\"user: no program\"),",
        breaks: "the degraded user requirement omits its exact first launch veto",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        write_expected(&mut file)?;",
        replace: "        let _ = write_expected(&mut file);",
        breaks: "a failed QEMU storage stage can publish partial bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_failed_storage_stage_preserves_the_complete_authoritative_image",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    }\n    for lba in 0..NVME_SIGNED_BLOCKS {\n        Seek::seek(file, SeekFrom::Start(lba * NVME_BLOCK_BYTES as u64))",
        replace: "    }\n    for lba in 0..NVME_SIGNED_BLOCKS - 1 {\n        Seek::seek(file, SeekFrom::Start(lba * NVME_BLOCK_BYTES as u64))",
        breaks: "the streamed NVMe namespace omits its final signed block",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_nvme_namespace_is_streamed_to_its_exact_extent",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
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
            name: "mutation_workspace::tests::mutation_state_growth_is_refused_after_measurement",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"    ensure!(
        bytes == initial
            && file
                .metadata()
                .with_context(|| format!("remeasuring {}", path.display()))?
                .len()
                == initial,
        "{} changed length while hashing",
        path.display()
    );"#,
        replace: r#"    ensure!(true, "{} changed length while hashing", path.display());"#,
        breaks: "a qualification digest accepts truncation after its preflight measurement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::generic_qualification_digest_truncation_is_refused_after_measurement",
        },
    },
    Mutation {
        file: "xtask/src/sha256.rs",
        find: r#"            return Ok((hasher.bytes, hasher.finish()));"#,
        replace: r#"            return Ok((hasher.bytes.saturating_sub(1), hasher.finish()));"#,
        breaks: "the streaming SHA-256 reader returns a byte count unrelated to what it hashed",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "sha256::tests::a_reader_is_hashed_without_changing_length_or_digest",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: r#"        qualification::read_bounded_text(&manifest, MAX_MANIFEST_BYTES, "evidence manifest")?;"#,
        replace: r#"        qualification::read_bounded_text(&manifest, u64::MAX, "evidence manifest")?;"#,
        breaks: "the offline evidence verifier removes its manifest allocation ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::an_oversized_manifest_is_refused_before_payload_allocation",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"    let text = read_bounded_text(path, MAX_RECORD_BYTES, "qualification record")?;"#,
        replace: r#"    let text = read_bounded_text(path, u64::MAX, "qualification record")?;"#,
        breaks: "qualification records are parsed without their caller-specific allocation ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::an_oversized_qualification_record_is_refused_before_parsing",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"    let text = read_bounded_text(path, MAX_VMWARE_TEXT_BYTES, "VMware generation receipt")?;"#,
        replace: r#"    let text = read_bounded_text(path, u64::MAX, "VMware generation receipt")?;"#,
        breaks: "VMware generation receipts are parsed without their caller-specific allocation ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::an_oversized_generation_receipt_is_refused_before_parsing",
        },
    },
    Mutation {
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
        file: "xtask/src/qualification.rs",
        find: r#"    ensure!(
        length as u64 <= maximum,
        "cargo-audit transcript exceeds its bounded artifact limit"
    );"#,
        replace: r#"    ensure!(
        true,
        "cargo-audit transcript exceeds its bounded artifact limit"
    );"#,
        breaks: "cargo-audit transcript construction allocates beyond its semantic artifact ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory_transcript_rendering_refuses_output_beyond_its_ceiling",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"    ensure!(
        transcript.len() as u64 <= maximum,
        "cargo-audit transcript exceeds its bounded artifact limit"
    );"#,
        replace: r#"    ensure!(
        true,
        "cargo-audit transcript exceeds its bounded artifact limit"
    );"#,
        breaks: "cargo-audit transcript verification ignores its semantic artifact ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory_transcript_verification_refuses_bytes_beyond_its_ceiling",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"    ensure!(
        copied <= maximum,
        "{label} grew beyond its {maximum}-byte capture limit while being copied"
    );"#,
        replace: r#"    ensure!(
        true,
        "{label} grew beyond its {maximum}-byte capture limit while being copied"
    );"#,
        breaks: "a VMware source can grow past its capture ceiling after preflight",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware_capture_refuses_growth_after_its_preflight_measurement",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"    ensure!(
        !exact || copied == maximum,
        "{label} changed outside its exact {maximum}-byte capture extent"
    );"#,
        replace: r#"    ensure!(
        true,
        "{label} changed outside its exact {maximum}-byte capture extent"
    );"#,
        breaks: "an exact VMware source can truncate after preflight and still be captured",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware_capture_refuses_truncation_after_exact_preflight",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
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
            name: "mutation_workspace::tests::mutation_digest_refuses_an_initial_file_beyond_its_ceiling",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
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
            name: "mutation_workspace::tests::mutation_digest_refuses_growth_after_measurement",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
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
            name: "mutation_workspace::tests::mutation_digest_refuses_truncation_after_measurement",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: r#"    ensure!(
        initial <= maximum,
        "{label} is {initial} bytes, exceeding its {maximum}-byte hashing limit"
    );"#,
        replace: r#"    ensure!(
        true,
        "{label} is {initial} bytes, exceeding its {maximum}-byte hashing limit"
    );"#,
        breaks: "a generic evidence digest streams an initially oversized file",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::generic_evidence_digests_have_a_streaming_ceiling",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: r#"    ensure!(
        bytes <= maximum,
        "{label} grew beyond its {maximum}-byte hashing limit while it was hashed"
    );"#,
        replace: r#"    ensure!(
        true,
        "{label} grew beyond its {maximum}-byte hashing limit while it was hashed"
    );"#,
        breaks: "a generic evidence digest can grow beyond its streaming ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::generic_evidence_digest_growth_is_refused_after_measurement",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: r#"    ensure!(
        bytes == initial,
        "{label} changed length while it was hashed"
    );"#,
        replace: r#"    ensure!(true, "{label} changed length while it was hashed");"#,
        breaks: "a generic evidence digest accepts a truncated stream after preflight",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::generic_evidence_digest_truncation_is_refused_during_hashing",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: r#"    ensure!(
        File::open(path)
            .with_context(|| format!("reopening {label} {}", path.display()))?
            .metadata()
            .with_context(|| format!("remeasuring {label} {}", path.display()))?
            .len()
            == initial,
        "{label} changed length after it was hashed"
    );"#,
        replace: r#"    ensure!(true, "{label} changed length after it was hashed");"#,
        breaks: "a generic evidence digest skips its final source-length remeasurement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::generic_evidence_digest_remeasures_after_hashing",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: r#"    ensure!(
        digest.bytes == expected,
        "{label} hashed {} bytes, expected exactly {expected}",
        digest.bytes
    );"#,
        replace: r#"    ensure!(
        true,
        "{label} hashed {} bytes, expected exactly {expected}",
        digest.bytes
    );"#,
        breaks: "an exact evidence digest trusts a forged returned byte count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::deterministic_artifact_digest_requires_the_exact_returned_byte_count",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: r#"    ensure!(
        limited
            .get_ref()
            .metadata()
            .with_context(|| format!("remeasuring {label} {}", path.display()))?
            .len()
            == expected,
        "{label} changed outside its exact {expected}-byte extent while being hashed"
    );"#,
        replace: r#"    ensure!(
        true,
        "{label} changed outside its exact {expected}-byte extent while being hashed"
    );"#,
        breaks: "an exact evidence digest skips its final source-extent remeasurement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::deterministic_artifact_digest_remeasures_the_exact_extent",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"            ADVISORY_POLICY_PATH => (MAX_ADVISORY_POLICY_BYTES, false),"#,
        replace: r#"            ADVISORY_POLICY_PATH => (MAX_ADVISORY_POLICY_BYTES + 1, false),"#,
        breaks: "the advisory policy artifact selects a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"        Kind::Mutation => (MAX_MUTATION_REPORT_BYTES, false),"#,
        replace: r#"        Kind::Mutation => (MAX_MUTATION_REPORT_BYTES + 1, false),"#,
        breaks: "the mutation report artifact selects a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"        Kind::Model => (MAX_MODEL_TRANSCRIPT_BYTES, false),"#,
        replace: r#"        Kind::Model => (MAX_MODEL_TRANSCRIPT_BYTES + 1, false),"#,
        breaks: "model transcript artifacts select a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"            VMWARE_IMAGE => (boot_image_bytes(), true),"#,
        replace: r#"            VMWARE_IMAGE => (boot_image_bytes() + 1, true),"#,
        breaks: "VMware boot images select the wrong exact artifact extent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"            VMWARE_NAMESPACE => (namespace_bytes(), true),"#,
        replace: r#"            VMWARE_NAMESPACE => (namespace_bytes() + 1, true),"#,
        breaks: "VMware namespaces select the wrong exact artifact extent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"            VMWARE_TRANSCRIPT => (MAX_VMWARE_TRANSCRIPT_BYTES, false),"#,
        replace: r#"            VMWARE_TRANSCRIPT => (MAX_VMWARE_TRANSCRIPT_BYTES + 1, false),"#,
        breaks: "VMware serial transcripts select a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"            | VMWARE_OBSERVER => (MAX_VMWARE_TEXT_BYTES, false),"#,
        replace: r#"            | VMWARE_OBSERVER => (MAX_VMWARE_TEXT_BYTES + 1, false),"#,
        breaks: "VMware text artifacts select a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"        return Ok((MAX_RECORD_BYTES, false));"#,
        replace: r#"        return Ok((MAX_RECORD_BYTES + 1, false));"#,
        breaks: "qualification bundle records select a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"        ensure!(
            length <= maximum,
            "qualification artifact {name} is {length} bytes, exceeding its {maximum}-byte limit"
        );"#,
        replace: r#"        ensure!(
            true,
            "qualification artifact {name} is {length} bytes, exceeding its {maximum}-byte limit"
        );"#,
        breaks: "the artifact-size caller ignores every bounded artifact policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"        ensure!(
            length == maximum,
            "qualification bundle file {} is {length} bytes, expected exactly {maximum}",
            spec.relative
        );"#,
        replace: r#"        ensure!(
            true,
            "qualification bundle file {} is {length} bytes, expected exactly {maximum}",
            spec.relative
        );"#,
        breaks: "the bundle-size caller ignores every exact artifact policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: r#"        ensure!(
            length <= maximum,
            "qualification bundle file {} is {length} bytes, exceeding its {maximum}-byte limit",
            spec.relative
        );"#,
        replace: r#"        ensure!(
            true,
            "qualification bundle file {} is {length} bytes, exceeding its {maximum}-byte limit",
            spec.relative
        );"#,
        breaks: "the bundle-size caller ignores every bounded artifact policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        file: "xtask/src/supply_chain.rs",
        find: r#"    let staging = directory.join(SPDX_STAGING);"#,
        replace: r#"    let staging = directory.join(SPDX_STAGING);
    std::fs::write(destination, b"partial")?;"#,
        breaks: "a failed SPDX stage can expose partial bytes through the authoritative destination",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::a_failed_spdx_stage_never_exposes_partial_authoritative_bytes",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"fn parse_failed_check(line: &str) -> Option<&str> {
    let message = log_message(line, LogSeverity::Error)?;
    let name = message
        .strip_prefix(SELFTEST_CHECK_PREFIX)?
        .strip_suffix(SELFTEST_CHECK_FAILED)?;
    (!name.is_empty()).then_some(name)
}"#,
        replace: r#"fn parse_failed_check(line: &str) -> Option<&str> {
    let line = line.trim_end();
    let at = line.find(SELFTEST_CHECK_PREFIX)?;
    line[at + SELFTEST_CHECK_PREFIX.len()..].strip_suffix(SELFTEST_CHECK_FAILED)
}"#,
        breaks: "a substring or unterminated record can forge a named failed self-test check",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_failed_check_requires_one_exact_terminated_error_record",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"    let (severity, message) = log_message(line, LogSeverity::Info)
        .map(|message| (LogSeverity::Info, message))
        .or_else(|| {
            log_message(line, LogSeverity::Error).map(|message| (LogSeverity::Error, message))
        })?;"#,
        replace: r#"    let start = line.find(SELFTEST_VERDICT_PREFIX)?;
    let severity = if line[..start].contains("[error] ") {
        LogSeverity::Error
    } else {
        LogSeverity::Info
    };
    let message = line[start..].trim_end();"#,
        breaks: "a severity-less or unterminated substring can forge the final self-test verdict",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::verdict_records_require_exact_framing_and_status_severity",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"        ensure!(
            matches!(
                (severity, status),
                (LogSeverity::Info, VerdictStatus::Pass)
                    | (LogSeverity::Error, VerdictStatus::Fail)
            ),
            "self-test verdict status has the wrong log severity"
        );"#,
        replace: r#"        let _unchecked_severity = severity;"#,
        breaks: "a PASS/error or FAIL/info record can donate a final self-test verdict",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::verdict_records_require_exact_framing_and_status_severity",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    let mut fields = message.strip_prefix(SELFTEST_VERDICT_PREFIX)?.split(' ');",
        replace: "    let mut fields = message\n        .strip_prefix(SELFTEST_VERDICT_PREFIX)?\n        .split_whitespace();",
        breaks: "noncanonical whitespace can separate self-test verdict fields",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::verdict_fields_require_single_ascii_spaces",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"    usize::try_from(canonical_decimal(value).context("invalid self-test verdict count")?)
        .context("self-test verdict count does not fit usize")"#,
        replace: r#"    value.parse().context("invalid self-test verdict count")"#,
        breaks: "self-test verdict counts accept noncanonical integer spellings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::verdict_counts_use_canonical_decimals",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"    let message = log_message(line, LogSeverity::Info)?;
    let mut fields = message.strip_prefix(BASELINE_PREFIX)?.split(' ');"#,
        replace: r#"    let start = line.find(BASELINE_PREFIX)?;
    let mut fields = line[start + BASELINE_PREFIX.len()..].trim_end().split(' ');"#,
        breaks: "a severity-less or unterminated substring can forge a performance baseline",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::baseline_records_require_exact_info_framing",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    let mut fields = message.strip_prefix(BASELINE_PREFIX)?.split(' ');",
        replace: "    let mut fields = message\n        .strip_prefix(BASELINE_PREFIX)?\n        .split_whitespace();",
        breaks: "noncanonical whitespace can separate performance-baseline fields",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::baseline_fields_require_single_ascii_spaces",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"    canonical_decimal(value).with_context(|| format!("invalid baseline field {name}"))"#,
        replace: r#"    value
        .parse()
        .with_context(|| format!("invalid baseline field {name}"))"#,
        breaks: "performance-baseline values accept noncanonical integer spellings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::baseline_values_use_canonical_decimals",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: ".position(|record| is_required_boot_marker(line, *record))",
        replace: ".position(|record| line.contains(record.message))",
        breaks: "a substring can trigger an interactive QMP injection round",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::interactive_input_requires_exact_terminated_info_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"    ensure!(
        index == *next,
        "self-test interactive marker {} arrived while waiting for {}",
        index + 1,
        *next + 1
    );"#,
        replace: r#"    let _unchecked_index = index;"#,
        breaks: "interactive QMP injection accepts duplicate or out-of-order marker records",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::interactive_input_markers_are_unique_and_ordered",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"                    match observe_interactive_marker(&text, &mut next_interactive_marker) {
                        Ok(true) => markers_seen.store(next_interactive_marker, Ordering::Release),
                        Ok(false) => {}
                        Err(error) => {
                            serial_failed.store(true, Ordering::Release);
                            return Err(error).context("reading interactive serial evidence");
                        }
                    }"#,
        replace: r#"                    let _ = &mut next_interactive_marker;
                    if text.contains(STORAGE_DELAY_MARKER)
                        || text.contains(READY_MARKER)
                        || text.contains(TRANSLATED_MARKER)
                    {
                        markers_seen.fetch_add(1, Ordering::Release);
                    }"#,
        breaks: "the serial forwarder bypasses ordered exact interactive-record matching",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(STORAGE_DELAY_MARKER),",
        replace: "    RequiredLogRecord::warning(STORAGE_DELAY_MARKER),",
        breaks: "the storage-delay injection round is enrolled at warning severity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(READY_MARKER),",
        replace: "    RequiredLogRecord::warning(READY_MARKER),",
        breaks: "the injected-keystroke round is enrolled at warning severity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(TRANSLATED_MARKER),",
        replace: "    RequiredLogRecord::warning(TRANSLATED_MARKER),",
        breaks: "the translated-keystroke round is enrolled at warning severity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"        if observation.failed.iter().any(|failed| failed == name) {
            bail!("the self-test kernel named one failed check more than once");
        }"#,
        replace: "",
        breaks: "duplicate failed-check records can forge the mutation failure count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::failed_check_names_are_unique_and_precede_the_terminal_verdict",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: r#"    if observation.verdict.is_some() {
        bail!("the self-test kernel printed serial data after its final verdict");
    }"#,
        replace: "",
        breaks: "failed-check evidence can be attributed after the terminal self-test verdict",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::failed_check_names_are_unique_and_precede_the_terminal_verdict",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: r#"            error!("selftest: {name} FAILED");"#,
        replace: r#"            warn!("selftest: {name} FAILED");"#,
        breaks: "the failed-check producer emits the wrong serial severity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: r#"            "selftest: verdict v{} PASS passed={} failed={} expected={}","#,
        replace: r#"            "selftest: verdict v{} OK passed={} failed={} expected={}","#,
        breaks: "the kernel PASS verdict producer drifts from the parser contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: r#"            "selftest: verdict v{} FAIL passed={} failed={} expected={}","#,
        replace: r#"            "selftest: verdict v{} ERROR passed={} failed={} expected={}","#,
        breaks: "the kernel FAIL verdict producer drifts from the parser contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: r#"        "selftest: baseline v1 boot_ns={} frames_free={} heap_slab={} heap_large={} heap_live={} handles_committed={} handles_live={} trap_ticks={} storage_bytes={} storage_ticks={} counter_hz={}","#,
        replace: r#"        "selftest: baseline v1 boot={} frames_free={} heap_slab={} heap_large={} heap_live={} handles_committed={} handles_live={} trap_ticks={} storage_bytes={} storage_ticks={} counter_hz={}","#,
        breaks: "the kernel performance-baseline producer drifts from the parser contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: r#"fn awaiting(marker: &str, press: &str) {
    info!("{marker}");"#,
        replace: r#"fn awaiting(marker: &str, press: &str) {
    warn!("{marker}");"#,
        breaks: "the shared interactive marker producer emits the wrong serial severity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: r#"        "selftest: awaiting storage-delay keystroke","#,
        replace: r#"        "selftest: awaiting storage delay keystroke","#,
        breaks: "the storage-delay producer literal drifts from its injection enrollment",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: r#"        "selftest: awaiting injected keystroke","#,
        replace: r#"        "selftest: awaiting injected key","#,
        breaks: "the injected-keystroke producer literal drifts from its injection enrollment",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: r#"        "selftest: awaiting translated keystroke","#,
        replace: r#"        "selftest: awaiting translated key","#,
        breaks: "the translated-keystroke producer literal drifts from its injection enrollment",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        file: "xtask/src/vmware.rs",
        find: "    let mut fields = verdicts[0].1[crate::qemu::SELFTEST_VERDICT_PREFIX.len()..].split(' ');",
        replace: "    let mut fields = verdicts[0].1[crate::qemu::SELFTEST_VERDICT_PREFIX.len()..].split_whitespace();",
        breaks: "VMware self-test verdicts accept noncanonical whitespace separators",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::vmware_selftest_verdict_requires_single_ascii_spaces",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "#[cfg(feature = \"selftest\")]\npub fn drive_fault_recovery_for_test() -> bool {",
        replace: "/// A mutated general unchecked copy door.\npub fn copy_from_user_unwalked(_from: u64, _into: &mut [u8]) -> Result<(), UserCopyError> {\n    Ok(())\n}\n\n#[cfg(feature = \"selftest\")]\npub fn drive_fault_recovery_for_test() -> bool {",
        breaks: "the removed general unwalked user-copy door returns to the shipped API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "#[cfg(feature = \"selftest\")]\npub fn drive_fault_recovery_for_test() -> bool {",
        replace: "pub fn drive_fault_recovery_for_test() -> bool {",
        breaks: "the fixed user-copy recovery driver reaches the shipped wost graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "    let exact = left == scratch.len() && scratch == UNCHANGED;",
        replace: "    let exact = left == 0 && scratch == UNCHANGED;",
        breaks: "the fixed-hole driver accepts a completed copy instead of the exact full remainder",
        reddens: Caught::Check("a faulting user copy resumes and reports"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "    drop(window);\n    let exact = left == scratch.len() && scratch == UNCHANGED;",
        replace: "    drop(window);\n    scratch[0] ^= 1;\n    let exact = left == scratch.len() && scratch == UNCHANGED;",
        breaks: "the fixed-hole driver changes its private destination before reporting",
        reddens: Caught::Check("a faulting user copy resumes and reports"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "            Some((_, flags)) if flags.contains(Flags::USER) => {",
        replace: "            Some((_, flags)) => {",
        breaks: "a walked user copy accepts a supervisor-only lower leaf",
        reddens: Caught::Check("bytes cross the boundary only where they may"),
    },
    Mutation {
        file: "libs/x86-apic/src/vector.rs",
        find: "        if vector >= FIRST_PIC_VECTOR && vector < FIRST_ASSIGNABLE {",
        replace: "        if vector >= FIRST_PIC_VECTOR && vector < FIRST_ASSIGNABLE && vector != 0x2f {",
        breaks: "the typed vector boundary admits the last remapped PIC slot",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "vector::tests::every_remapped_pic_vector_is_reserved",
        },
    },
    Mutation {
        file: "kernel/src/arch/x86_64/vector.rs",
        find: "pub(crate) const SYSCALL: InterruptVector = SOURCES[4];",
        replace: "pub(crate) const SYSCALL: InterruptVector = SOURCES[3];",
        breaks: "the syscall gate aliases the local APIC timer's assigned vector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_c_header_and_the_kernel_agree_about_the_abi",
        },
    },
    Mutation {
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "    fn route_line(\n",
        replace: "    #[allow(dead_code)]\n    fn route_raw_for_mutation(\n        &mut self,\n        line: u32,\n        vector: u8,\n        wiring: Wiring,\n        destination: u32,\n    ) -> Result<(), RoutingError> {\n        let Some(vector) = InterruptVector::new(vector) else {\n            return Ok(());\n        };\n        self.route_line(line, vector, wiring, destination)\n    }\n\n    fn route_line(\n",
        breaks: "an I/O APIC programming door recovers a raw vector parameter",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::one_assignment_and_typed_signatures_own_every_vector_door",
        },
    },
    Mutation {
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        && controller.registers.low == u32::from(super::vector::KEYBOARD.get())",
        replace: "        && controller.registers.low == u32::from(super::vector::TIMER.get())",
        breaks: "the isolated I/O APIC route accepts another source's encoded vector",
        reddens: Caught::Check("an isolated I/O APIC route encodes its typed vector"),
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let current = access(at, CommandAccess::Read).ok_or(EnableError::Access)? as u16;",
        replace: "    let current = access(at, CommandAccess::Read).unwrap_or(u32::from(state.installed)) as u16;",
        breaks: "an unreadable bus-master upgrade borrows its expected command word",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_bus_master_upgrade_access_refusal_preserves_its_owner",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        Err(error) => {\n            if error != EnableError::Restore {\n                state.phase = CommandPhase::Installed;\n            }\n            Err(error)\n        }",
        replace: "        Err(error) => {\n            state.phase = CommandPhase::Installed;\n            Err(error)\n        }",
        breaks: "an uncertain bus-master rollback loses its retry phase",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::failed_bus_master_upgrade_rollback_never_loses_cleanup_authority",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if !leases.holds_function(function, state.lease) || state.phase != CommandPhase::Installed {",
        replace: "    if state.phase != CommandPhase::Installed {",
        breaks: "a command owner upgrades bus mastering through another function",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::bus_master_upgrade_rejects_another_function_before_access",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    admit_bus_master_with_leases(&mut functions.commands, authority.function, state, access)",
        replace: "    admit_bus_master_with_leases(&mut functions.commands, state.lease.function, state, access)",
        breaks: "bus-master admission substitutes the command owner's function for the mapped BAR",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::mapped_bus_master_admission_requires_its_bar_and_command_owner",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if cc & configuration::ENABLE != 0 || csts & status::READY != 0 {",
        replace: "        if csts & status::READY != 0 {",
        breaks: "NVMe quiescence ignores an enabled controller",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::only_disabled_and_not_ready_registers_pass_dma_classification",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if cc & configuration::ENABLE != 0 || csts & status::READY != 0 {",
        replace: "        if cc & configuration::ENABLE != 0 {",
        breaks: "NVMe quiescence ignores a controller that remains ready",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::only_disabled_and_not_ready_registers_pass_dma_classification",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        match DisabledRegisters::from_bits(cc, csts) {",
        replace: "        match DisabledRegisters::from_bits(0, 0) {",
        breaks: "unverified NVMe register state can discard quarantine authority",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_unverified_preparation_access_failure_preserves_its_quarantine_owner",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "impl<T> DisabledPreparation<T> {\n    /// Attempt one disabled-controller access with exact retirement fallback.\n    pub fn access<U, E>(\n        mut self,\n        operation: impl FnOnce(&mut T) -> Result<U, E>,\n    ) -> Result<(Self, U), PreparationFailure<Self, E>> {\n        match operation(&mut self.owner) {\n            Ok(value) => Ok((self, value)),\n            Err(reason) => Err(PreparationFailure {\n                preparation: self,\n                reason,\n            }),\n        }\n    }",
        replace: "impl<T> DisabledPreparation<T> {\n    /// Attempt one disabled-controller access with exact retirement fallback.\n    pub fn access<U, E>(\n        mut self,\n        operation: impl FnOnce(&mut T) -> Result<U, E>,\n    ) -> Result<(Self, U), PreparationFailure<Self, E>> {\n        match operation(&mut self.owner) {\n            Ok(value) => Ok((self, value)),\n            Err(_) => panic!(\"mutant discarded disabled-controller cleanup authority\"),\n        }\n    }",
        breaks: "a disabled-controller access refusal discards exact retirement authority",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_disabled_preparation_access_failure_returns_the_exact_retirement_owner",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/pci.rs",
        find: "            .read_u32(register::CC)",
        replace: "            .read_u32(register::CSTS)",
        breaks: "NVMe DMA authority reads readiness twice instead of controller enable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::nvme_bus_master_authority_is_fresh_linear_and_mapping_bound",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/pci.rs",
        find: "#[derive(Debug)]\n#[must_use = \"NVMe quiescence authority must be consumed by DMA admission\"]\npub struct NvmeBusMasterAuthority<'bar> {",
        replace: "#[derive(Debug, Clone)]\n#[must_use = \"NVMe quiescence authority must be consumed by DMA admission\"]\npub struct NvmeBusMasterAuthority<'bar> {",
        breaks: "one fresh NVMe observation can authorize more than one DMA transition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/pci.rs",
        find: "pub struct NvmeBusMasterAuthority<'bar> {\n    bar: &'bar MappedBar,\n    _observed: DisabledRegisters,\n}",
        replace: "pub struct NvmeBusMasterAuthority<'bar> {\n    pub bar: &'bar MappedBar,\n    pub _observed: DisabledRegisters,\n}",
        breaks: "safe callers can forge NVMe DMA authority from unrelated values",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/pci.rs",
        find: "pub fn admit_bus_master(\n    authority: NvmeBusMasterAuthority<'_>,",
        replace: "pub fn enable_bus_master(at: Address) -> Result<(), EnableError> {\n    let mut configuration = CONFIGURATION.lock_leaf();\n    let (address, data) = configuration.ports.as_mut().ok_or(EnableError::Access)?;\n    let command = at.at(header::COMMAND).ok_or(EnableError::Stale)?;\n    Port::write(address, command.bits());\n    let current = Port::read(data) as u16;\n    let wanted = current | pci_config::command::MEMORY_SPACE | pci_config::command::BUS_MASTER;\n    Port::write(address, command.bits());\n    Port::write(data, u32::from(wanted));\n    Port::write(address, command.bits());\n    let found = Port::read(data) as u16;\n    if found == wanted { Ok(()) } else { Err(EnableError::Refused(found)) }\n}\n\npub fn admit_bus_master(\n    authority: NvmeBusMasterAuthority<'_>,",
        breaks: "a raw PCI address can enable DMA without mapping or quiescence authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if !functions.holds(function) || bars.function != Some(function.lease) {",
        replace: "    if !functions.holds(function) {",
        breaks: "a function can reserve a BAR probed through another function generation",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_mapping_authority_is_linear_and_bound_to_its_probe_function",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if Bar::decode(low, high) != Some(authority.window.bar) {",
        replace: "    if false {",
        breaks: "memory decode ignores a changed live BAR identity",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::changed_or_foreign_bar_identity_performs_zero_command_writes",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "const PROBE_DISABLED: u16 = PROBE_DECODE | command::BUS_MASTER;",
        replace: "const PROBE_DISABLED: u16 = PROBE_DECODE;",
        breaks: "BAR probing leaves inherited bus mastering enabled",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::inherited_bus_master_stays_off_until_the_staged_command_retires",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        Ok(CommandState {\n            lease: probe.0,",
        replace: "        let _released = self.release(probe.0);\n        Ok(CommandState {\n            lease: probe.0,",
        breaks: "a staged probe releases command ownership before retirement",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::inherited_bus_master_stays_off_until_the_staged_command_retires",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "                assert!(\n                    leases.finish_probe(authority, probe).is_ok(),\n                    \"a restored BAR-probe refusal lost its exact reservation\"\n                );",
        replace: "                let _retained = probe;",
        breaks: "a clean BAR-probe refusal retains its function sublease after exact rollback",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::refused_bus_master_clear_releases_only_after_exact_rollback",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        if memory_type == Self::BELOW_ONE_MEGABYTE {\n            return None;\n        }",
        replace: "        if false && memory_type == Self::BELOW_ONE_MEGABYTE {\n            return None;\n        }",
        breaks: "the obsolete below-one-megabyte BAR encoding is accepted as ordinary memory",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_memory_bar_type_encoding_has_one_exact_outcome",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        if memory_type == Self::RESERVED {\n            return None;\n        }",
        replace: "        if false && memory_type == Self::RESERVED {\n            return None;\n        }",
        breaks: "the reserved memory BAR encoding is accepted as ordinary memory",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_memory_bar_type_encoding_has_one_exact_outcome",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "            Bar::decode(original[low], 0)?,",
        replace: "            Bar::decode(original[low], 0).unwrap_or(Bar::Absent),",
        breaks: "an unsupported memory BAR reaches sizing writes",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::unsupported_memory_bar_types_refuse_before_any_probe_write",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let low = read(low_at, authority.index).ok_or(BarIdentityError::Access)?;",
        replace: "    let low = read(low_at, authority.index).ok_or(BarIdentityError::Access)? & !Bar::TYPE_MASK;",
        breaks: "a type-only live BAR change is erased before identity verification",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_type_only_bar_change_performs_zero_command_writes",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let low = read(low_at, authority.index).ok_or(BarIdentityError::Access)?;\n    let high = match authority.window.bar {\n        Bar::Memory { wide: true, .. } => {\n            let high_offset = low_offset.checked_add(4).ok_or(BarIdentityError::Stale)?;\n            if authority.index + 1 >= function.bar_registers() {\n                return Err(BarIdentityError::Stale);\n            }\n            let high_at = function\n                .address()\n                .at(high_offset)\n                .ok_or(BarIdentityError::Stale)?;\n            read(high_at, authority.index + 1).ok_or(BarIdentityError::Access)?\n        }\n        Bar::Memory { wide: false, .. } => 0,\n        Bar::Io { .. } | Bar::Absent => return Err(BarIdentityError::Stale),\n    };",
        replace: "    let high = match authority.window.bar {\n        Bar::Memory { wide: true, .. } => {\n            let high_offset = low_offset.checked_add(4).ok_or(BarIdentityError::Stale)?;\n            if authority.index + 1 >= function.bar_registers() {\n                return Err(BarIdentityError::Stale);\n            }\n            let high_at = function\n                .address()\n                .at(high_offset)\n                .ok_or(BarIdentityError::Stale)?;\n            read(high_at, authority.index + 1).ok_or(BarIdentityError::Access)?\n        }\n        Bar::Memory { wide: false, .. } => 0,\n        Bar::Io { .. } | Bar::Absent => return Err(BarIdentityError::Stale),\n    };\n    let low = read(low_at, authority.index).ok_or(BarIdentityError::Access)?;",
        breaks: "a wide BAR reads its high half before its low half",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::exact_wide_bar_reads_precede_the_first_decode_write",
        },
    },
    Mutation {
        file: "libs/memory-map/src/lib.rs",
        find: "        matches!(self, Self::Reserved | Self::Unknown(_))",
        replace: "        matches!(\n            self,\n            Self::Reserved | Self::Unknown(_) | Self::BootloaderReclaimable\n        )",
        breaks: "device admission treats bootloader storage as an MMIO hole",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::every_forbidden_kind_and_byte_in_a_page_refuses_device_mapping",
        },
    },
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\n#[must_use = \"a BAR authority must remain with its mapping or be returned\"]\npub struct BarAuthority {",
        replace: "#[derive(Debug, Clone, PartialEq, Eq)]\n#[must_use = \"a BAR authority must remain with its mapping or be returned\"]\npub struct BarAuthority {",
        breaks: "one BAR reservation can be replayed into multiple mappings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_bar_mapping_requires_exact_linear_authority",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "        page_start < page_end && page_end <= 1_u64 << physical_bits,",
        replace: "        false && page_start < page_end && page_end <= 1_u64 << physical_bits,",
        breaks: "the live NVMe BAR witness withholds its admitted physical-width result",
        reddens: Caught::Check("a live nvme bar obeys maxphyaddr"),
    },
    Mutation {
        file: "libs/x86-cpuid/src/physical.rs",
        find: "        if maximum_extended_leaf < ADDRESS_SIZE_LEAF || bits < 32 || bits > 52 {",
        replace: "        if bits < 32 || bits > 52 {",
        breaks: "an absent address-size leaf lends an unrelated register value as MAXPHYADDR",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "physical::tests::an_absent_leaf_never_borrows_an_unrelated_answer",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/physical.rs",
        find: "        start < end && end <= 1_u64 << self.0",
        replace: "        start < end && end < 1_u64 << self.0",
        breaks: "the final implemented physical byte is rejected by MAXPHYADDR admission",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "physical::tests::each_supported_limit_is_exclusive",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/device.rs",
        find: "        let page_start = physical - physical % PAGE_SIZE;",
        replace: "        let page_start = physical;",
        breaks: "device admission omits bytes before an unaligned register base in its first page",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "device::tests::a_short_register_range_expands_to_the_complete_page",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/device.rs",
        find: "        if !width.contains(page_start, page_end) {",
        replace: "        if false {",
        breaks: "device admission ignores the running processor's physical-address width",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "device::tests::each_runtime_width_accepts_its_last_page_and_refuses_the_next",
        },
    },
    Mutation {
        file: "xtask/src/image/program.rs",
        find: "    let slot = code.get_mut(site).with_context(|| {",
        replace: "    let slot = code.get_mut(0).with_context(|| {",
        breaks: "the fixed witness accepts a branch patch outside its emitted program",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::program::tests::branch_patching_refuses_an_invalid_site",
        },
    },
    Mutation {
        file: "xtask/src/image/program.rs",
        find: "        i8::try_from(distance).with_context(|| {",
        replace: "        i8::try_from(distance.min(i8::MAX as usize)).with_context(|| {",
        breaks: "the fixed witness truncates an out-of-range forward branch",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::program::tests::branch_patching_enforces_the_signed_byte_extent",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        if !self.layout.contains_publication_word(offset) {\n            return None;\n        }",
        replace: "        if false && !self.layout.contains_publication_word(offset) {\n            return None;\n        }",
        breaks: "a publication load can address bytes outside its immutable word partition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        if !self.layout.contains_publication_word(offset) {\n            return false;\n        }",
        replace: "        if false && !self.layout.contains_publication_word(offset) {\n            return false;\n        }",
        breaks: "a publication store can address bytes outside its immutable word partition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        let end = offset.checked_add(size_of::<u32>())?;",
        replace: "        let end = offset.wrapping_add(size_of::<u32>());",
        breaks: "publication-word end arithmetic can wrap into an apparently in-range address",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        if end as u64 > self.len() {\n            return None;\n        }",
        replace: "        if false && end as u64 > self.len() {\n            return None;\n        }",
        breaks: "a publication word can extend beyond its retained allocation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        let address = base.addr().checked_add(offset)?;",
        replace: "        let address = base.addr().wrapping_add(offset);",
        breaks: "publication pointer arithmetic can wrap away from its checked direct-map base",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        if !address.is_multiple_of(align_of::<u32>()) {\n            return None;\n        }",
        replace: "        if false && !address.is_multiple_of(align_of::<u32>()) {\n            return None;\n        }",
        breaks: "a misaligned publication word reaches an indivisibility-dependent instruction",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            if current.is_huge() || current.flags().contains(Flags::USER) {",
        replace: "            if current.flags().contains(Flags::USER) {",
        breaks: "shared topology accepts a huge top-level entry as a table",
        reddens: Caught::Check("shared topology rejects hostile root entries"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            if current.is_huge() || current.flags().contains(Flags::USER) {",
        replace: "            if current.is_huge() {",
        breaks: "shared topology accepts a user-accessible top-level table",
        reddens: Caught::Check("shared topology rejects hostile root entries"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            self.take_owned_table(DEVICE_WINDOW_START, Level::Pml4, TablePrivilege::Supervisor)?;",
        replace: "            self.take_owned_table(DEVICE_WINDOW_START, Level::Pml4, TablePrivilege::User)?;",
        breaks: "shared topology reserves a user-accessible device-window root",
        reddens: Caught::Check("shared topology rejects hostile root entries"),
    },
    Mutation {
        file: "libs/x86-paging/src/lib.rs",
        find: "        self.address() == found.address()\n            && self.flags().difference(Flags::ACCESSED).bits()\n                == found.flags().difference(Flags::ACCESSED).bits()",
        replace: "        self.bits() == found.bits()",
        breaks: "binding rejects harmless hardware-accessed drift in a shared root",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::table_identity_ignores_only_accessed",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/lib.rs",
        find: "        self.address() == found.address()\n            && self.flags().difference(Flags::ACCESSED).bits()\n                == found.flags().difference(Flags::ACCESSED).bits()",
        replace: "        self.address() == found.address()\n            && self\n                .flags()\n                .difference(Flags::ACCESSED.union(Flags::NO_CACHE))\n                .bits()\n                == found\n                    .flags()\n                    .difference(Flags::ACCESSED.union(Flags::NO_CACHE))\n                    .bits()",
        breaks: "binding ignores software-controlled cache-policy drift in a shared root",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::table_identity_ignores_only_accessed",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if flags.contains(Flags::NO_CACHE) || flags.contains(Flags::WRITE_THROUGH) {",
        replace: "    if flags.contains(Flags::NO_CACHE) && flags.contains(Flags::WRITE_THROUGH) {",
        breaks: "a shared mapping admits an individual non-write-back cache policy alias",
        reddens: Caught::Check("shared mappings reject cache aliases"),
    },
    Mutation {
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
        file: "xtask/src/api.rs",
        find: "fn optional_path_is_present(path: &Path) -> Result<bool> {\n    path.try_exists()\n        .with_context(|| format!(\"checking whether {} exists\", path.display()))\n}",
        replace: "fn optional_path_is_present(path: &Path) -> Result<bool> {\n    Ok(path.exists())\n}",
        breaks: "a metadata error makes an optional API-audit source directory appear absent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::optional_source_directory_metadata_errors_are_not_absence",
        },
    },
    Mutation {
        file: "libs/slab-layout/src/ownership.rs",
        find: "        self.free_frames\n            .checked_add(self.slab_frames)?\n            .checked_add(self.large_frames)",
        replace: "        Some(\n            self.free_frames\n                .wrapping_add(self.slab_frames)\n                .wrapping_add(self.large_frames),\n        )",
        breaks: "overflow wraps an allocator ownership account",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::every_boundary_account_fails_closed_on_overflow",
        },
    },
    Mutation {
        file: "libs/slab-layout/src/ownership.rs",
        find: "                .is_some_and(|accounted| before.accounted_physical_frames() == Some(accounted))",
        replace: "                .is_some_and(|accounted| {\n                    before.accounted_physical_frames().unwrap_or(accounted) == accounted\n                })",
        breaks: "an overflowing earlier allocator account defaults to the later total",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::every_boundary_account_fails_closed_on_overflow",
        },
    },
    Mutation {
        file: "libs/slab-layout/src/ownership.rs",
        find: "            && self\n                .accounted_physical_frames()\n                .is_some_and(|accounted| before.accounted_physical_frames() == Some(accounted))",
        replace: "            && self.accounted_physical_frames().is_some()",
        breaks: "allocator ownership accepts a changed physical-frame total",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::every_small_free_and_slab_account_obeys_conservation",
        },
    },
    Mutation {
        file: "libs/slab-layout/src/ownership.rs",
        find: "        self.live_allocations == before.live_allocations\n            && self.slab_frames >= before.slab_frames",
        replace: "        self.slab_frames >= before.slab_frames",
        breaks: "allocator ownership ignores a changed live-allocation count",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::live_count_is_exact_while_large_frame_ownership_may_move",
        },
    },
    Mutation {
        file: "libs/slab-layout/src/ownership.rs",
        find: "        self.live_allocations == before.live_allocations\n            && self.slab_frames >= before.slab_frames",
        replace: "        self.live_allocations == before.live_allocations\n            && self.slab_frames >= before.slab_frames\n            && self.large_frames == before.large_frames",
        breaks: "allocator ownership rejects a live allocation's capacity change",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::live_count_is_exact_while_large_frame_ownership_may_move",
        },
    },
    Mutation {
        file: "libs/slab-layout/src/ownership.rs",
        find: "        self.live_allocations == before.live_allocations\n            && self.slab_frames >= before.slab_frames",
        replace: "        self.live_allocations == before.live_allocations",
        breaks: "allocator ownership accepts the disappearance of a retained slab",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::every_conserved_slab_transition_is_directionally_monotonic",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: "    fn capture() -> Self {\n        let disabled = interrupt::disable();\n        let snapshot = Self {\n            free_frames: mm::free_frames(),\n            heap: mm::heap::stats(),\n        };\n        disabled.restore();\n        snapshot\n    }",
        replace: "    fn capture() -> Self {\n        Self {\n            free_frames: mm::free_frames(),\n            heap: mm::heap::stats(),\n        }\n    }",
        breaks: "an allocator ownership snapshot spans interrupt epochs",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::allocator_ledgers_hold_one_interrupt_epoch",
        },
    },
    Mutation {
        file: "kernel/src/selftest.rs",
        find: "fn spawn_resources() -> SpawnResources {\n    let disabled = interrupt::disable();\n    let allocator = AllocatorSnapshot::capture();\n    let resources = SpawnResources {\n        frames: allocator.free_frames,\n        heap: allocator.heap,\n        arena: HandleTable::arena_usage_for_test(),\n        channels: object::live_channels_for_test(),\n        rings: queue::live_for_test(),\n        thread_slots: thread::free_slots(),\n        process_records: user::process_records_for_test(),\n        mapping_refusals: user::refused_mapping_reclaims(),\n        table_refusals: user::refused_table_reclaims(),\n        task_retirements: user::refused_task_retirements(),\n    };\n    disabled.restore();\n    resources\n}",
        replace: "fn spawn_resources() -> SpawnResources {\n    let allocator = AllocatorSnapshot::capture();\n    SpawnResources {\n        frames: allocator.free_frames,\n        heap: allocator.heap,\n        arena: HandleTable::arena_usage_for_test(),\n        channels: object::live_channels_for_test(),\n        rings: queue::live_for_test(),\n        thread_slots: thread::free_slots(),\n        process_records: user::process_records_for_test(),\n        mapping_refusals: user::refused_mapping_reclaims(),\n        table_refusals: user::refused_table_reclaims(),\n        task_retirements: user::refused_task_retirements(),\n    }\n}",
        breaks: "a multi-owner spawn ledger spans interrupt epochs",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::allocator_ledgers_hold_one_interrupt_epoch",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "                table: bootstrap.table.identity_for_test(),",
        replace: "                table: bootstrap.table.identity_for_test().wrapping_add(1),",
        breaks: "a zombie record names a different process incarnation",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    (launches, bootstraps, zombies)",
        replace: "    (\n        launches,\n        bootstraps + usize::from(zombies != 0),\n        zombies,\n    )",
        breaks: "a retained zombie is also counted as a live bootstrap record",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "            RECLAIMED_PAGES.fetch_add(success.observed_returned_pages, Ordering::AcqRel);",
        replace: "            RECLAIMED_PAGES.fetch_add(\n                success.observed_returned_pages + 1,\n                Ordering::AcqRel,\n            );",
        breaks: "successful collection overcounts returned user pages",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "            RECLAIMED_TABLES.fetch_add(success.reclaimed.tables, Ordering::AcqRel);",
        replace: "            RECLAIMED_TABLES.fetch_add(\n                success.reclaimed.tables + 1,\n                Ordering::AcqRel,\n            );",
        breaks: "successful collection overcounts returned page-table frames",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    \"push rax\",\n    \"mov ax, {kernel_data}\",\n    \"mov ss, ax\",",
        replace: "    \"push rax\",",
        breaks: "a ring-three trap can carry a null stack selector into kernel work",
        reddens: Caught::Check("ring-three entry restores the kernel stack selector"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "#[cfg(feature = \"selftest\")]\npub const SAVED_FRAME_BYTES: u64 = size_of::<RawTrapFrame>() as u64;",
        replace: "pub const SAVED_FRAME_BYTES: u64 = size_of::<RawTrapFrame>() as u64;",
        breaks: "a machine-only hardware observer reaches the shipped wost API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "pub(super) const RECORD_BUFFER_CLEARS: usize = cfg!(feature = \"selftest\") as usize;",
        replace: "pub(super) const RECORD_BUFFER_CLEARS: usize = 1;",
        breaks: "buffer-clear evidence writes reach shipped return paths",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::buffer_clear_state_is_selftest_only",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "#[cfg(feature = \"selftest\")]\nstatic REFUSED_MAPPING_RECLAIMS: AtomicUsize = AtomicUsize::new(0);",
        replace: "static REFUSED_MAPPING_RECLAIMS: AtomicUsize = AtomicUsize::new(0);",
        breaks: "mapping-reclaim evidence state reaches the shipped kernel",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    \".if {record_clears}\",\n    \"jnz 3f\",\n    \"inc qword ptr [rip + BUFFER_CLEARS]\",",
        replace: "    \".if 1\",\n    \"jnz 3f\",\n    \"inc qword ptr [rip + BUFFER_CLEARS]\",",
        breaks: "the trap-return buffer-clear tally ignores its self-test selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::buffer_clear_assembly_writes_are_guarded",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "        \".if {record_clears}\",\n        \"jnz 3f\",\n        \"inc qword ptr [rip + BUFFER_CLEARS]\",",
        replace: "        \".if 1\",\n        \"jnz 3f\",\n        \"inc qword ptr [rip + BUFFER_CLEARS]\",",
        breaks: "the first-user-entry buffer-clear tally ignores its self-test selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::buffer_clear_assembly_writes_are_guarded",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "#[cfg(feature = \"selftest\")]\npub fn free_frames() -> usize {",
        replace: "pub fn free_frames() -> usize {",
        breaks: "the free-frame observer reaches the shipped wost API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        file: "wost/src/mmio.rs",
        find: "    #[cfg(feature = \"selftest\")]\n    pub(crate) const fn base_address(&self) -> *mut u8 {",
        replace: "    pub(crate) const fn base_address(&self) -> *mut u8 {",
        breaks: "the framebuffer MMIO base observer reaches shipped wost",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "    #[cfg(feature = \"selftest\")]\n    #[must_use = \"a refused segment read copied no bytes\"]\n    pub fn read_at(&self, offset: usize, into: &mut [u8]) -> bool {",
        replace: "    #[must_use = \"a refused segment read copied no bytes\"]\n    pub fn read_at(&self, offset: usize, into: &mut [u8]) -> bool {",
        breaks: "the owned-segment byte observer reaches shipped wost",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    #[cfg(feature = \"selftest\")]\n    pub const fn was_user(&self) -> bool {",
        replace: "    pub const fn was_user(&self) -> bool {",
        breaks: "the unmapped-range user-bit observer reaches shipped wost",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        file: "wost/src/mm/heap.rs",
        find: "        #[cfg(feature = \"selftest\")]\n        if !self.record_slab_frame() {",
        replace: "        if !self.record_slab_frame() {",
        breaks: "the slab-evidence helper executes on the shipped refill path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        file: "kernel/src/dev.rs",
        find: "pub(crate) fn keep_io_apic(io_apic: IoApic, #[cfg(feature = \"selftest\")] gsi: u32) {",
        replace: "pub(crate) fn keep_io_apic(io_apic: IoApic, gsi: u32) {",
        breaks: "the routed-GSI evidence parameter reaches the shipped helper",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "        #[cfg(feature = \"selftest\")]\n        gsi,",
        replace: "        gsi,",
        breaks: "the routed-GSI evidence argument reaches the shipped call",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            #[cfg(feature = \"selftest\")]\n            let result = if mapping_refusal(&MAPPING_WRITES_BEFORE_REFUSAL) {\n                Err(MapError::OutOfMemory)\n            } else {\n                self.set(virtual_address, physical, flags, level)\n            };\n            #[cfg(not(feature = \"selftest\"))]\n            let result = self.set(virtual_address, physical, flags, level);",
        replace: "            #[cfg(feature = \"selftest\")]\n            let injected = mapping_refusal(&MAPPING_WRITES_BEFORE_REFUSAL);\n            #[cfg(not(feature = \"selftest\"))]\n            let injected = false;\n            let result = if injected {\n                Err(MapError::OutOfMemory)\n            } else {\n                self.set(virtual_address, physical, flags, level)\n            };",
        breaks: "the shipped mapping loop carries a no-op test-refusal branch",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            #[cfg(not(feature = \"selftest\"))]\n            dismiss(eoi);",
        replace: "            #[cfg(not(feature = \"selftest\"))]\n            {\n                let eoi = Some(eoi);\n                if let Some(eoi) = eoi {\n                    dismiss(eoi);\n                }\n            }",
        breaks: "the shipped ping path wraps completion authority for test-only recovery",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::every_interrupt_arm_has_explicit_completion_ownership",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            let callee_receiver = if let Some(qself) = &path.qself {\n                Some(ExactReceiver {\n                    range: range(qself.ty.span()),\n                    exact_nominal: exact_nominal_type_path(&qself.ty).is_some(),\n                })\n            } else {\n                path_prefix_receiver(&path.path)\n            };\n            let argument_receiver = path.qself.is_none().then(|| {\n                expression\n                    .args\n                    .first()\n                    .and_then(exact_dispatch_receiver_range)\n            });",
        replace: "            let callee_receiver = path\n                .qself\n                .is_none()\n                .then(|| path_prefix_receiver(&path.path))\n                .flatten();\n            let argument_receiver = Some(\n                expression\n                    .args\n                    .first()\n                    .and_then(exact_dispatch_receiver_range),\n            );",
        breaks: "a qualified static trait call treats its first argument as Self evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::concrete_qself_static_dispatch_uses_self_not_argument_zero",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        declarations.take(2).count() == 1",
        replace: "        false && declarations.take(2).count() == 1",
        breaks: "an exact lexical generic trait call loses its declaration-only route",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::generic_qself_static_dispatch_credits_only_its_exact_trait_declaration",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if !self.dispatch_requires_resolution(inventory, site, evidence) {\n            return DispatchResolution::NoCredit;\n        }",
        replace: "        if !self.dispatch_requires_resolution(inventory, site, evidence) {\n            let DispatchKind::Call { member, .. } = &site.kind;\n            if let Some(activation) = inventory\n                .trait_activations\n                .iter()\n                .find(|activation| activation.key.member == *member)\n            {\n                return DispatchResolution::Implementation(activation.implementation);\n            }\n            return DispatchResolution::NoCredit;\n        }",
        breaks: "bare method syntax resolves an external trait implementation by member name alone",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::untyped_external_method_dispatch_does_not_guess_an_implementation",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if self.exact_self_type(inventory, site, evidence).is_some()\n            || evidence.callee_nodes.iter().any(|node| {",
        replace: "        if false\n            || evidence.callee_nodes.iter().any(|node| {",
        breaks: "concrete Self evidence is admitted by the generic declaration-only route",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::generic_qself_with_concrete_self_evidence_is_not_declaration_only",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "fn qself_trait_path(expression: &syn::ExprPath) -> Option<String> {\n    let qself = expression.qself.as_ref()?;\n    qself.as_token?;\n    if qself.position == 0 || qself.position + 1 != expression.path.segments.len() {",
        replace: "fn qself_trait_path(expression: &syn::ExprPath) -> Option<String> {\n    let qself = expression.qself.as_ref()?;\n    qself.as_token?;\n    if qself.position == 0\n        || qself.position + 1 != expression.path.segments.len()\n        || expression\n            .path\n            .segments\n            .iter()\n            .take(qself.position)\n            .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))\n    {",
        breaks: "generic arguments erase an explicit trait dispatch qualifier",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::generic_trait_arguments_preserve_the_full_explicit_dispatch_qualifier",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                        self.key.trait_path == qualifier\n                            || !qualifier.contains(\"::\")\n                                && last_path_segment(&self.key.trait_path) == qualifier",
        replace: "                        last_path_segment(&self.key.trait_path)\n                            == last_path_segment(qualifier)",
        breaks: "a qualified call accepts a different trait with the same terminal name",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::qualified_dispatch_distinguishes_same_named_traits",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            .take(count)\n            .take(count.saturating_sub(1))\n            .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))",
        replace: "            .take(count)\n            .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))",
        breaks: "a parameterized direct trait bound is rejected before its generic qself call",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::parameterized_generic_qself_credits_only_its_exact_trait_declaration",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            .is_some_and(|bounds| bounds.contains(&trait_path))",
        replace: "            .is_some_and(|bounds| !bounds.is_empty())",
        breaks: "any bound on a type parameter is mistaken for the directly named qself trait",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::generic_qself_rejects_indirect_and_nonparameter_shapes",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        let callee_self = evidence\n            .self_nodes\n            .intersection(&evidence.callee_nodes)",
        replace: "        let callee_self = evidence\n            .nodes\n            .intersection(&evidence.callee_nodes)",
        breaks: "a trait's generic argument is mistaken for the qself Self type",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::parameterized_generic_qself_credits_only_its_exact_trait_declaration",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            || self.local_default_dispatch(inventory, site, evidence)",
        replace: "            || false && self.local_default_dispatch(inventory, site, evidence)",
        breaks: "an exact local default method is refused because a sibling type overrides it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::an_exact_local_default_is_not_confused_with_a_sibling_override",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if self.selected_non_trait_callable(inventory, evidence) {",
        replace: "        if false && self.selected_non_trait_callable(inventory, evidence) {",
        breaks: "an exact ordinary call is forced through an unrelated same-named trait route",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::an_exact_free_call_excludes_a_same_named_external_trait_body",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            if self.declaration.is_none()\n                && trait_qualifier.is_none()\n                && !site.callee_names(callee_nodes, exact_self_type)\n            {",
        replace: "            if false\n                && self.declaration.is_none()\n                && trait_qualifier.is_none()\n                && !site.callee_names(callee_nodes, exact_self_type)\n            {",
        breaks: "exact Self evidence activates an external trait without an explicit trait qualifier",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::an_unqualified_external_dispatch_with_exact_self_fails_closed",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                if let DispatchResolution::Implementation(implementation) =\n                    self.dispatch_resolution(inventory, site, evidence)\n                    && live.insert(implementation)",
        replace: "                if let Some(implementation) = self\n                    .dispatch_matches(inventory, site, evidence)\n                    .first()\n                    .map(|activation| activation.implementation)\n                    && live.insert(implementation)",
        breaks: "liveness bypasses the shared dispatch classification and activates a generic specialization",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::generic_qself_static_dispatch_credits_only_its_exact_trait_declaration",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "self.dispatch_resolution(inventory, dispatch, evidence)",
        replace: "match self.dispatch_matches(inventory, dispatch, evidence).as_slice() {\n                    [activation] => {\n                        DispatchResolution::Implementation(activation.implementation)\n                    }\n                    matches => DispatchResolution::Failure(matches.len()),\n                }",
        breaks: "dispatch validation bypasses the shared classification and refuses a valid generic call",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::generic_qself_static_dispatch_credits_only_its_exact_trait_declaration",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        if let Some(exact) = preferred.next().copied() {",
        replace: "        if let Some(exact) = preferred.next().copied().filter(|_| false) {",
        breaks: "a type-qualified static call loses the callee Self priority over argument zero",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::type_qualified_static_dispatch_uses_callee_not_argument_zero",
        },
    },
    Mutation {
        file: "docs/014-storage.md",
        find: "measurement, so the current self-test image still needs the same human round;",
        replace: "measurement, so the current 189-check image still needs the same human round;",
        breaks: "a stale numeric machine total competes with the checked current summary",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::the_phase_close_says_what_the_source_says",
        },
    },
    Mutation {
        file: "xtask/src/prose.rs",
        find: "character.is_alphanumeric() || matches!(character, '_' | '-')",
        replace: "character.is_alphanumeric() || character == '_'",
        breaks: "a then-current historical count is mistaken for a current numeric claim",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::current_numeric_history_is_distinct_from_a_current_claim",
        },
    },
    Mutation {
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
        file: "AGENTS.md",
        find: "Consumer scaffolding remains stopped until PW-601 has both a pinned draft\nartifact and a runtime seam;",
        replace: "Consumer scaffolding starts when PW-601 has a pinned draft artifact;",
        breaks: "contributor guidance authorizes consumer scaffolding without a runtime seam",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::current_facing_foundation_prose_names_the_later_resolutions",
        },
    },
    Mutation {
        file: "README.md",
        find: "`cargo xtask qualify-vmware --shipped`",
        replace: "`cargo xtask qualify-vmware`",
        breaks: "the phase-close guide omits qualification of the shipped VMware transcript",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::current_facing_foundation_prose_names_the_later_resolutions",
        },
    },
    Mutation {
        file: "docs/004-decision-log.md",
        find: "scaffolding remains stopped until both a pinned draft artifact and its runtime\nseam exist.",
        replace: "scaffolding may start once a draft artifact exists.",
        breaks: "D-183's current amendment authorizes OS scaffolding before PW-601",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::current_facing_foundation_prose_names_the_later_resolutions",
        },
    },
    Mutation {
        file: "wost/src/panic.rs",
        find: "    crate::arch::current::pci::force_unlock(panic);\n",
        replace: "",
        breaks: "the panic path forgets to reclaim the PCI configuration lock",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::every_static_lock_in_wost_is_reclaimed_by_the_panic_path",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    PERMANENT_STACKS.force_unlock(panic);\n    TASK_STACK_PIN.force_unlock(panic);\n",
        replace: "    PERMANENT_STACKS.force_unlock(panic);\n",
        breaks: "the paging panic door omits the task stack pin ledger",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::every_static_lock_in_wost_is_reclaimed_by_the_panic_path",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "        workspace.require_cargo_inputs_unchanged()?;\n        plan.compiler.require_target_libraries_unchanged()?;\n",
        replace: "        plan.compiler.require_target_libraries_unchanged()?;\n",
        breaks: "an instrumented compiler round never rebinds the materialized registry",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::source_freshness_is_checked_after_compilation_and_before_the_verdict",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "                && !site.callee_names(callee_nodes, exact_self_type)\n            {",
        replace: "                && true\n            {",
        breaks: "a type-qualified external trait call is refused although its exact callee type has one audited implementation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::a_type_qualified_external_trait_call_credits_its_exact_callee_type",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    fn callee_names(&self, callee_nodes: &BTreeSet<usize>, self_type: usize) -> bool {\n        self.exact_callee_self_syntax && !self.explicit_qself && callee_nodes.contains(&self_type)\n    }",
        replace: "    fn callee_names(&self, _callee_nodes: &BTreeSet<usize>, _self_type: usize) -> bool {\n        self.exact_callee_self_syntax && !self.explicit_qself\n    }",
        breaks: "an aliased callee lets argument zero supply the Self of an external trait call",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::an_aliased_callee_does_not_borrow_argument_zero_for_an_external_trait",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        (DatabaseBinding::Pinned, JsonValue::Null, JsonValue::Null) => None,",
        replace: "        (_, JsonValue::Null, JsonValue::Null) => None,",
        breaks: "a population report can omit the database identity cargo-audit fetched",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_population_transcript_without_database_identity_is_rejected",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        audited.advisory_count == fetched.advisory_count,\n        \"the pinned audit and the population run saw different advisory databases\"\n    );",
        replace: "    let _unchecked_database_agreement = (audited.advisory_count, fetched.advisory_count);",
        breaks: "the pinned audit can see a different advisory database than the population run attested",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::the_pinned_audit_must_see_the_population_runs_database",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        ensure!(\n            is_rfc3339_timestamp(record.get(\"database.last-updated\")?),\n            \"advisory qualification names no RFC3339 database update time\"\n        );",
        replace: "        let _unchecked_database_update_time = record.get(\"database.last-updated\")?;",
        breaks: "an advisory record can carry a malformed database update time",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory_schema_binds_database_time_command_and_tool_identity",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "        ensure!(\n            record.get(\"database.last-updated\")? == attestation.last_updated,\n            \"advisory record names another database update time\"\n        );",
        replace: "        let _unchecked_attested_update_time = attestation.last_updated;",
        breaks: "an advisory record can name an update time its population transcript does not attest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::an_advisory_record_must_repeat_the_attested_database_update_time",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "            ensure!(\n                previous < raw,\n                \"Git tracked paths are not unique and sorted\"\n            );",
        replace: "            ensure!(\n                previous != raw,\n                \"Git tracked paths are not unique and sorted\"\n            );",
        breaks: "a reversed Git tracked-path inventory is accepted as sorted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::tracked_path_order_refuses_duplicates_and_reversals",
        },
    },
    Mutation {
        file: "xtask/src/cargo.rs",
        find: r#"            Prefix::VerbatimUNC(server, share) => {
                let mut root = OsString::from(r"\\");
                root.push(server);
                root.push(r"\");
                root.push(share);
                PathBuf::from(root)
            }"#,
        replace: "            Prefix::VerbatimUNC(_, _) => return path.to_path_buf(),",
        breaks: "a Windows verbatim UNC target path reaches a GNU toolchain subprocess",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::a_cargo_command_sheds_the_windows_verbatim_unc_target_prefix",
        },
    },
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Baselines {
    host: bool,
    api: bool,
    selftest: bool,
    degraded: bool,
}

fn baselines_for(mutations: &[Mutation]) -> Baselines {
    let mut baselines = Baselines::default();
    for mutation in mutations {
        match mutation.reddens {
            Caught::HostTest { .. } => baselines.host = true,
            Caught::ApiAudit(_) => baselines.api = true,
            Caught::Check(_) | Caught::Panic { .. } => baselines.selftest = true,
            Caught::BootMarker(_) | Caught::DegradedPanic { .. } => {
                baselines.degraded = true;
            }
        }
    }
    baselines
}

fn require_green(label: &str, result: Result<()>) -> Result<()> {
    result.with_context(|| format!("pristine {label} mutation baseline is not green"))
}

fn prove_green_baselines(paths: &Paths, mutations: &[Mutation]) -> Result<()> {
    let baselines = baselines_for(mutations);
    if baselines.host {
        println!("[xtask] proving the pristine host mutation baseline");
        require_green("host", tasks::test(paths))?;
    }
    if baselines.api {
        println!("[xtask] proving the pristine API-audit mutation baseline");
        require_green("API audit", crate::api::check(paths))?;
    }
    if baselines.selftest {
        println!("[xtask] proving the pristine self-test mutation baseline");
        require_green(
            "self-test",
            tasks::test_boot(paths, crate::cargo::Profile::Release),
        )?;
    }
    if baselines.degraded {
        println!("[xtask] proving the pristine degraded mutation baseline");
        require_green(
            "degraded",
            tasks::degraded_check(paths, crate::cargo::Profile::Release),
        )?;
    }
    Ok(())
}

/// Apply each mutation, run its exact machine or host witness, and require red.
///
/// # Errors
///
/// If a pattern no longer matches, if the tree cannot be restored, or if any
/// mutation **survives** — which is the interesting failure and the reason
/// this exists.
pub fn run(paths: &Paths) -> Result<()> {
    let registry = registry_fingerprint();
    let qualification_attempt = qualification::begin_mutation(paths, &registry, MUTATIONS.len())?;
    let mut session = Session::open(paths, &registry, MUTATIONS.len())?;
    if let Some(active) = session.active() {
        let mutation = MUTATIONS
            .get(active)
            .with_context(|| format!("journal names absent mutation {active}"))?;
        println!(
            "[xtask] restoring interrupted mutation {}/{}: {}",
            active + 1,
            MUTATIONS.len(),
            mutation.breaks
        );
        session.recover(active, mutation.file)?;
    }

    let first = session.completed().len();
    if first != 0 {
        println!(
            "[xtask] resuming after {first}/{} durable mutation outcomes",
            MUTATIONS.len()
        );
    }
    prove_green_baselines(session.paths(), &MUTATIONS[first..])?;
    for (index, mutation) in MUTATIONS.iter().enumerate().skip(first) {
        ensure_target_is_rebuilt(mutation)?;
        let original = session.original(mutation.file)?;
        let mutated = apply(&original, mutation)?;

        println!(
            "[xtask] mutate {}/{}: {}",
            index + 1,
            MUTATIONS.len(),
            mutation.breaks
        );
        session.begin(index)?;
        Session::write(&session, mutation.file, &mutated)?;
        let outcome = match mutation_reports(session.paths(), mutation.reddens) {
            Ok(true) => {
                println!("[xtask]   caught");
                Outcome::Caught
            }
            Ok(false) => {
                println!("[xtask]   SURVIVED");
                Outcome::Survived
            }
            Err(error) => {
                // A different red verdict did not answer this mutation's
                // question, so it is inconclusive rather than a survivor.
                println!("[xtask]   inconclusive: {error}");
                Outcome::Inconclusive(format!("{error:#}"))
            }
        };
        session.finish(index, mutation.file, outcome)?;
    }

    let outcomes = session.complete(MUTATIONS.len())?;
    let mut survivors = Vec::new();
    let mut inconclusive = Vec::new();
    for (mutation, outcome) in MUTATIONS.iter().zip(&outcomes) {
        match outcome {
            Outcome::Caught => {}
            Outcome::Survived => survivors.push(mutation),
            Outcome::Inconclusive(error) => inconclusive.push((mutation, error)),
        }
    }
    if survivors.is_empty() && inconclusive.is_empty() {
        let pass = crate::mutation_workspace::current_pass(paths, &registry, MUTATIONS.len())?;
        qualification_attempt.pass(&pass)?;
        println!("[xtask] every mutation was caught by the check it names");
        return Ok(());
    }
    for survivor in &survivors {
        println!("[xtask] survived: {} ({})", survivor.breaks, survivor.file);
    }
    for (mutation, error) in &inconclusive {
        println!(
            "[xtask] inconclusive: {} ({}): {error}",
            mutation.breaks, mutation.file
        );
    }

    match (survivors.len(), inconclusive.len()) {
        (0, count) => bail!(
            "{count} of {} mutations were inconclusive; each needs a valid build and its \
             exact expected verdict",
            MUTATIONS.len()
        ),
        (count, 0) => bail!(
            "{count} of {} mutations survived; each is a property nothing checks",
            MUTATIONS.len()
        ),
        (survived, inconclusive) => bail!(
            "{survived} of {} mutations survived and {inconclusive} were inconclusive",
            MUTATIONS.len()
        ),
    }
}

pub(crate) fn registry_fingerprint() -> String {
    fn field(material: &mut Vec<u8>, value: &str) {
        material.extend_from_slice(&(value.len() as u64).to_le_bytes());
        material.extend_from_slice(value.as_bytes());
    }

    let mut material = Vec::new();
    for mutation in MUTATIONS {
        field(&mut material, mutation.file);
        field(&mut material, mutation.find);
        field(&mut material, mutation.replace);
        field(&mut material, mutation.breaks);
        match mutation.reddens {
            Caught::Check(name) => {
                field(&mut material, "check");
                field(&mut material, name);
            }
            Caught::HostTest { package, name } => {
                field(&mut material, "host");
                field(&mut material, package);
                field(&mut material, name);
            }
            Caught::ApiAudit(item) => {
                field(&mut material, "api-audit");
                field(&mut material, item);
            }
            Caught::BootMarker(marker) => {
                field(&mut material, "boot-marker");
                field(&mut material, marker);
            }
            Caught::DegradedPanic { marker, why } => {
                field(&mut material, "degraded-panic");
                field(&mut material, marker);
                field(&mut material, why);
            }
            Caught::Panic { marker, why } => {
                field(&mut material, "panic");
                field(&mut material, marker);
                field(&mut material, why);
            }
        }
    }
    sha256::hex(&sha256::digest(&material))
}

/// Refuse source the already-running mutation driver would consume itself.
fn ensure_target_is_rebuilt(mutation: &Mutation) -> Result<()> {
    let rebuilt_witness = matches!(
        mutation.reddens,
        Caught::HostTest {
            package: "xtask",
            ..
        } | Caught::ApiAudit(_)
    );
    if mutation.file.starts_with("xtask/") && !rebuilt_witness {
        bail!(
            "{}: `{}` belongs to the running mutation driver and would not be rebuilt by its witness",
            mutation.file,
            mutation.breaks
        );
    }
    Ok(())
}

/// The mutated text, or an error naming why the pattern is no longer usable.
fn apply(original: &str, mutation: &Mutation) -> Result<String> {
    match original.matches(mutation.find).count() {
        1 => Ok(original.replacen(mutation.find, mutation.replace, 1)),
        0 => bail!(
            "{}: the pattern for `{}` no longer occurs; the mutation tests nothing",
            mutation.file,
            mutation.breaks
        ),
        many => bail!(
            "{}: the pattern for `{}` occurs {many} times; which one it breaks is not decided",
            mutation.file,
            mutation.breaks
        ),
    }
}

/// Report whether this mutation reached its exact named red verdict.
/// A compile refusal or another red verdict is inconclusive, not caught.
fn mutation_reports(paths: &Paths, wanted: Caught) -> Result<bool> {
    if let Caught::HostTest { package, name } = wanted {
        return host_test_reports(paths, package, name);
    }
    if let Caught::BootMarker(marker) = wanted {
        return match tasks::degraded_check(paths, crate::cargo::Profile::Release) {
            Ok(()) => Ok(false),
            Err(error) => {
                let expected = format!("without reporting {marker:?}");
                if format!("{error:#}").contains(&expected) {
                    Ok(true)
                } else {
                    Err(error)
                }
            }
        };
    }
    if matches!(wanted, Caught::DegradedPanic { .. }) {
        return qualify_degraded_mutation(wanted, || {
            tasks::observe_degraded_boot(paths, crate::cargo::Profile::Release)
        });
    }
    if let Caught::ApiAudit(item) = wanted {
        return match crate::api::check(paths) {
            Ok(()) => Ok(false),
            Err(error) => {
                let report = format!("{error:#}");
                if crate::api::report_names_exact_dead_item(&report, item) {
                    Ok(true)
                } else {
                    Err(error)
                }
            }
        };
    }
    qualify_machine_mutation(wanted, || {
        tasks::observe_test_boot(paths, crate::cargo::Profile::Release)
    })
}

const MACHINE_OBSERVATIONS: usize = 2;

fn qualify_machine_mutation(
    wanted: Caught,
    mut observe: impl FnMut() -> Result<qemu::SelftestOutcome>,
) -> Result<bool> {
    let mut first_error = None;
    for attempt in 1..=MACHINE_OBSERVATIONS {
        match observe().and_then(|observed| assess_selftest(wanted, observed)) {
            Ok(caught) => return Ok(caught),
            Err(error) if attempt < MACHINE_OBSERVATIONS => {
                let report = format!("{error:#}");
                eprintln!("[xtask]   machine observation inconclusive; retrying once: {report}");
                first_error = Some(report);
            }
            Err(error) => {
                if let Some(first) = first_error {
                    bail!(
                        "machine mutation remained inconclusive after {MACHINE_OBSERVATIONS} observations; first: {first}; retry: {error:#}"
                    );
                }
                return Err(error);
            }
        }
    }
    unreachable!("the bounded observation loop returns on every outcome")
}

fn qualify_degraded_mutation(
    wanted: Caught,
    mut observe: impl FnMut() -> Result<qemu::DegradedOutcome>,
) -> Result<bool> {
    let mut first_error = None;
    for attempt in 1..=MACHINE_OBSERVATIONS {
        match observe().and_then(|observed| assess_degraded_panic(wanted, observed)) {
            Ok(caught) => return Ok(caught),
            Err(error) if attempt < MACHINE_OBSERVATIONS => {
                let report = format!("{error:#}");
                eprintln!("[xtask]   degraded observation inconclusive; retrying once: {report}");
                first_error = Some(report);
            }
            Err(error) => {
                if let Some(first) = first_error {
                    bail!(
                        "degraded mutation remained inconclusive after {MACHINE_OBSERVATIONS} observations; first: {first}; retry: {error:#}"
                    );
                }
                return Err(error);
            }
        }
    }
    unreachable!("the bounded observation loop returns on every outcome")
}

fn assess_degraded_panic(wanted: Caught, observed: qemu::DegradedOutcome) -> Result<bool> {
    let Caught::DegradedPanic { marker, .. } = wanted else {
        unreachable!("only a degraded-panic witness reaches degraded assessment")
    };
    match observed {
        qemu::DegradedOutcome::Settled => Ok(false),
        qemu::DegradedOutcome::Panicked {
            no_x2apic_diagnosed: false,
            transcript,
        } => bail!(
            "mutation expected a prior no-x2APIC diagnosis before panic {marker:?}, but observed: {transcript}"
        ),
        qemu::DegradedOutcome::Panicked { transcript, .. } if transcript.contains(marker) => {
            Ok(true)
        }
        qemu::DegradedOutcome::Panicked { transcript, .. } => {
            bail!("mutation expected degraded panic marker {marker:?}, but observed: {transcript}")
        }
    }
}

fn assess_selftest(wanted: Caught, observed: qemu::SelftestOutcome) -> Result<bool> {
    match (wanted, observed) {
        (_, qemu::SelftestOutcome::Passed) => Ok(false),
        (Caught::Check(name), qemu::SelftestOutcome::Failed(failed))
            if failed.iter().any(|actual| actual == name) =>
        {
            Ok(true)
        }
        (Caught::Check(name), qemu::SelftestOutcome::Failed(failed)) => bail!(
            "mutation expected check {name:?}, but different checks failed: {}",
            failed.join("; ")
        ),
        (Caught::Check(name), qemu::SelftestOutcome::Panicked { failed, .. })
            if failed.iter().any(|actual| actual == name) =>
        {
            Ok(true)
        }
        (Caught::Check(name), qemu::SelftestOutcome::Panicked { transcript, failed }) => {
            let earlier = if failed.is_empty() {
                "none".to_string()
            } else {
                failed.join("; ")
            };
            bail!(
                "mutation expected check {name:?}, but earlier checks were [{earlier}] before the kernel panicked: {transcript}"
            )
        }
        (Caught::Panic { marker, .. }, qemu::SelftestOutcome::Panicked { transcript, .. })
            if transcript.contains(marker) =>
        {
            Ok(true)
        }
        (Caught::Panic { marker, .. }, qemu::SelftestOutcome::Panicked { transcript, .. }) => {
            bail!("mutation expected panic marker {marker:?}, but observed: {transcript}")
        }
        (Caught::Panic { marker, .. }, qemu::SelftestOutcome::Failed(failed)) => bail!(
            "mutation expected panic marker {marker:?}, but checks failed: {}",
            failed.join("; ")
        ),
        (
            Caught::HostTest { .. }
            | Caught::ApiAudit(_)
            | Caught::BootMarker(_)
            | Caught::DegradedPanic { .. },
            _,
        ) => {
            unreachable!("other witnesses return before self-test assessment")
        }
    }
}

const HOST_TEST_TIMEOUT: Duration = Duration::from_secs(5 * 60);

fn host_test_reports(paths: &Paths, package: &str, name: &str) -> Result<bool> {
    host_test_reports_with(paths, package, name, |command, timeout, label| {
        crate::bounded_process::output(command, timeout, label)
    })
}

fn host_test_reports_with<F>(paths: &Paths, package: &str, name: &str, run: F) -> Result<bool>
where
    F: FnOnce(Command, Duration, &str) -> Result<Output>,
{
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(paths.root())
        .env(
            "CARGO_TARGET_DIR",
            crate::cargo::child_process_path(paths.target_dir()),
        )
        .args(["test", "--package", package, name, "--", "--exact"]);
    let label = format!("host mutation test {package}::{name}");
    let output = run(command, HOST_TEST_TIMEOUT, &label)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assess_host_test(
        name,
        output.status.success(),
        &stdout,
        &stderr,
        &output.status.to_string(),
    )
}

fn assess_host_test(
    name: &str,
    success: bool,
    stdout: &str,
    stderr: &str,
    status: &str,
) -> Result<bool> {
    let failed = format!("test {name} ... FAILED");
    let failed = stdout
        .lines()
        .chain(stderr.lines())
        .filter(|line| *line == failed)
        .count();
    ensure!(
        failed <= 1,
        "host mutation printed duplicate failed verdicts"
    );
    if failed == 1 {
        ensure!(
            !success,
            "host mutation printed a failed verdict but Cargo exited successfully"
        );
        return Ok(true);
    }
    if success {
        let passed = format!("test {name} ... ok");
        let passed = stdout
            .lines()
            .chain(stderr.lines())
            .filter(|line| *line == passed)
            .count();
        if passed == 1 {
            return Ok(false);
        }
        bail!("host mutation selected no exact test `{name}`");
    }
    bail!("host mutation did not reach `{name}`; cargo exited {status}\n{stdout}{stderr}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use syn::visit::{self, Visit};

    use super::*;

    #[derive(Default)]
    struct MachineCheckInventory {
        names: BTreeMap<String, usize>,
        unsupported: usize,
    }

    impl<'ast> Visit<'ast> for MachineCheckInventory {
        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            let report_receiver = matches!(
                node.receiver.as_ref(),
                syn::Expr::Path(path) if path.path.is_ident("report")
            );
            if node.method == "check" && report_receiver {
                match node.args.first() {
                    Some(syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(name),
                        ..
                    })) if node.args.len() == 2 => {
                        *self.names.entry(name.value()).or_default() += 1;
                    }
                    _ => self.unsupported += 1,
                }
            }
            visit::visit_expr_method_call(self, node);
        }
    }

    fn machine_checks(source: &str) -> MachineCheckInventory {
        let syntax = syn::parse_file(source).expect("parsing machine self-test source");
        let mut inventory = MachineCheckInventory::default();
        inventory.visit_file(&syntax);
        inventory
    }

    #[test]
    #[ignore]
    fn parked_host_mutation_child() {
        loop {
            std::thread::park();
        }
    }

    #[test]
    fn a_parked_host_mutation_is_inconclusive_not_caught() {
        let root = std::env::current_dir().expect("current directory");
        let paths = Paths::new(
            root.clone(),
            Some(&root.join("target/mutation-timeout-fixture")),
        );
        let executable = std::env::current_exe().expect("current test executable");
        let started = std::time::Instant::now();
        let error = host_test_reports_with(
            &paths,
            "xtask",
            "mutate::tests::parked_host_mutation_child",
            |command, timeout, label| {
                assert_eq!(command.get_current_dir(), Some(root.as_path()));
                assert_eq!(timeout, HOST_TEST_TIMEOUT);
                assert_eq!(
                    command.get_args().collect::<Vec<_>>(),
                    [
                        "test",
                        "--package",
                        "xtask",
                        "mutate::tests::parked_host_mutation_child",
                        "--",
                        "--exact",
                    ]
                    .map(std::ffi::OsStr::new)
                );
                let mut parked = Command::new(&executable);
                parked.args([
                    "--exact",
                    "mutate::tests::parked_host_mutation_child",
                    "--ignored",
                ]);
                crate::bounded_process::output(parked, Duration::from_millis(50), label)
            },
        )
        .unwrap_err();

        assert!(
            format!("{error:#}").contains("timed out after 50 ms"),
            "{error:#}"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn api_audit_mutations_require_the_current_structured_dead_item_verdict() {
        let current =
            "1 public API item(s) have no caller:\n  fn is_relative at libs/path-walk/src/lib.rs:1";
        assert!(crate::api::report_names_exact_dead_item(
            current,
            "fn is_relative"
        ));
        assert!(!crate::api::report_names_exact_dead_item(
            &current.replace("fn is_relative at", "fn is_relative_helper at"),
            "fn is_relative"
        ));

        for rejected in [
            "1 public API item(s) have no production caller (0 verification-only, 1 with no compiled caller):\n  fn is_relative at libs/path-walk/src/lib.rs:1 [no compiled caller]",
            "1 forged API-audit summary:\n  fn is_relative at libs/path-walk/src/lib.rs:1",
            "1 public API item(s) have no caller: fn is_relative at libs/path-walk/src/lib.rs:1",
            "2 public API item(s) have no caller:\n  fn is_relative at libs/path-walk/src/lib.rs:1\n  fn other at libs/path-walk/src/lib.rs:2",
            "1 public API item(s) have no caller:\n  fn is_relative at libs/path-walk/src/lib.rs:1\n  fn other at libs/path-walk/src/lib.rs:2",
            "0 public API item(s) have no caller:",
            "1 public API item(s) have no caller:\n  fn is_relative at libs/path-walk/src/lib.rs:1 [no compiled caller]",
            "1 public API item(s) have no caller:\n  fn is_relative at ",
            "1 public API item(s) have no caller:\nfn is_relative at libs/path-walk/src/lib.rs:1",
        ] {
            assert!(
                !crate::api::report_names_exact_dead_item(rejected, "fn is_relative"),
                "accepted malformed or stale API-audit output: {rejected}"
            );
        }
    }

    #[test]
    fn api_audit_commands_rebind_original_and_copied_sources_before_verdicts() {
        let source = include_str!("api.rs");
        let metadata = source
            .split_once("impl CargoCatalog {")
            .and_then(|(_, source)| source.split_once("    fn parse("))
            .map(|(source, _)| source)
            .expect("Cargo metadata command body");
        let compilation = source
            .split_once("fn compile_graph(")
            .and_then(|(_, source)| source.split_once("fn cargo_arguments("))
            .map(|(source, _)| source)
            .expect("compiler graph command body");

        for (label, command, verdict) in [
            ("Cargo metadata", metadata, "ensure!("),
            ("compiler graph", compilation, "if !result.status.success()"),
        ] {
            let output = command.find(".output()").expect("command completion");
            let copied = command
                .find("workspace.require_copy_unchanged(expected)?;")
                .expect("copied-source freshness guard");
            let original = command
                .find("workspace.require_source_unchanged(&workspace.source_root)?;")
                .expect("original-source freshness guard");
            let verdict = command.find(verdict).expect("command verdict");
            assert!(
                output < copied && copied < original && original < verdict,
                "{label} output was interpreted before both source trees were rebound"
            );
        }
    }

    fn rust_sources_contain(directory: &std::path::Path, needle: &str) -> bool {
        crate::paths::directory_entries(directory)
            .unwrap_or_else(|reason| {
                panic!(
                    "complete directory listing for {}: {reason}",
                    directory.display()
                )
            })
            .into_iter()
            .any(|entry| {
                let path = entry.path();
                if entry.is_dir() {
                    rust_sources_contain(&path, needle)
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    std::fs::read_to_string(&path)
                        .unwrap_or_else(|_| panic!("reading {}", path.display()))
                        .contains(needle)
                } else {
                    false
                }
            })
    }

    /// Every mutation still names something that exists, exactly once.
    ///
    /// The cheap tier, and the one that matters more: a pattern that has
    /// stopped matching is a mutation that silently stopped testing anything —
    /// which is the failure mode this whole file exists to catch, one level up.
    /// Milliseconds, in `cargo test`, so it is in the gate where the boot
    /// itself cannot be.
    #[test]
    fn every_mutation_still_applies() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");

        let mut stale = Vec::new();
        for mutation in MUTATIONS {
            let path = root.join(mutation.file);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("reading {}", path.display()));
            let found = text.matches(mutation.find).count();
            if found != 1 {
                stale.push(format!(
                    "{}: `{}` occurs {found} times",
                    mutation.file, mutation.breaks
                ));
            }
            assert_ne!(mutation.find, mutation.replace, "{}", mutation.breaks);
        }
        assert!(
            stale.is_empty(),
            "mutation patterns without one current source anchor:\n{}",
            stale.join("\n")
        );
    }

    /// Mutating the driver cannot change the binary already building images.
    #[test]
    fn a_mutation_cannot_target_the_running_driver() {
        for mutation in MUTATIONS {
            ensure_target_is_rebuilt(mutation).unwrap_or_else(|error| panic!("{error}"));
        }

        let mut impossible = MUTATIONS[0];
        impossible.file = "xtask/src/image/program.rs";
        impossible.reddens = Caught::Check("unused");
        let error =
            ensure_target_is_rebuilt(&impossible).expect_err("the driver is already running");
        assert!(error.to_string().contains("would not be rebuilt"));
    }

    /// Require every named check or host test to remain present without
    /// recursively invoking Cargo or paying for a machine boot.
    #[test]
    fn every_named_check_exists() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let selftest = std::fs::read_to_string(root.join("kernel/src/selftest.rs"))
            .expect("reading selftest.rs");
        let checks = machine_checks(&selftest);
        assert_eq!(
            checks.unsupported, 0,
            "every machine report.check name must be one exact source literal"
        );
        for (name, count) in &checks.names {
            assert_eq!(
                *count, 1,
                "machine check `{name}` is registered {count} times"
            );
        }
        let stale = MUTATIONS
            .iter()
            .filter_map(|mutation| match mutation.reddens {
                Caught::Check(name) if checks.names.get(name) != Some(&1) => {
                    Some(format!("{} -> {name}", mutation.breaks))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            stale.is_empty(),
            "machine mutations without one current report.check:\n{}",
            stale.join("\n")
        );

        for mutation in MUTATIONS {
            match mutation.reddens {
                Caught::Check(_) => {}
                Caught::HostTest { package, name } => {
                    assert!(
                        crate::paths::HOST_PACKAGES.contains(&package),
                        "`{}` names non-host package `{package}`",
                        mutation.breaks
                    );
                    let function = name
                        .rsplit_once("::")
                        .map_or(name, |(_, function)| function);
                    let package = if package == "xtask" {
                        root.join(package)
                    } else {
                        root.join("libs").join(package)
                    };
                    assert!(
                        rust_sources_contain(&package, &format!("fn {function}(")),
                        "`{}` names host test `{name}`, which package `{}` does not have",
                        mutation.breaks,
                        package.display()
                    );
                }
                Caught::ApiAudit(item) => {
                    assert!(!item.is_empty(), "API-audit mutations name an exact item");
                }
                Caught::BootMarker(marker) => {
                    let qemu = std::fs::read_to_string(root.join("xtask/src/qemu.rs"))
                        .expect("reading qemu.rs");
                    assert!(
                        qemu.contains(marker),
                        "`{}` names boot marker `{marker}`, which qemu.rs does not have",
                        mutation.breaks
                    );
                }
                Caught::DegradedPanic { .. } | Caught::Panic { .. } => {}
            }
        }
    }

    #[test]
    fn machine_check_inventory_rejects_decoys_and_nonliteral_names() {
        let source = r#"
            fn observe(report: &mut Report, decoy: &mut Report) {
                const BORROWED: &str = "borrowed name";
                let _history = "historical check";
                decoy.check("forged check", true);
                report.check("current check", true);
                report.check(BORROWED, true);
            }
        "#;

        let checks = machine_checks(source);
        assert_eq!(checks.names.get("current check"), Some(&1));
        assert!(!checks.names.contains_key("historical check"));
        assert!(!checks.names.contains_key("forged check"));
        assert!(!checks.names.contains_key("borrowed name"));
        assert_eq!(checks.unsupported, 1);
    }

    #[test]
    fn every_host_library_has_a_persistent_source_mutation() {
        for &package in crate::paths::HOST_PACKAGES {
            if package == "xtask" {
                continue;
            }
            let source = format!("libs/{package}/src/");
            assert!(
                MUTATIONS.iter().any(|mutation| {
                    mutation.file.starts_with(&source)
                        && matches!(
                            mutation.reddens,
                            Caught::HostTest {
                                package: actual,
                                ..
                            } if actual == package
                        )
                }),
                "{package} has no persistent production-source mutation"
            );
        }
    }

    #[test]
    fn an_exact_host_witness_must_run() {
        let name = "tests::the_exact_test";
        assert!(assess_host_test(name, true, "running 0 tests", "", "0").is_err());
        assert!(
            assess_host_test(name, true, "test tests::the_exact_test ... ok", "", "0")
                .is_ok_and(|caught| !caught)
        );
        assert!(
            assess_host_test(
                name,
                false,
                "test tests::the_exact_test ... FAILED",
                "",
                "1"
            )
            .is_ok_and(|caught| caught)
        );
        assert!(
            assess_host_test(name, true, "test tests::the_exact_test ... FAILED", "", "0").is_err()
        );
        assert!(
            assess_host_test(
                name,
                false,
                "diagnostic: test tests::the_exact_test ... FAILED (forged)",
                "",
                "1"
            )
            .is_err()
        );
        assert!(
            assess_host_test(
                name,
                true,
                "test tests::the_exact_test ... ok\ntest tests::the_exact_test ... ok",
                "",
                "0"
            )
            .is_err()
        );
        assert!(assess_host_test(name, false, "", "compiler error", "1").is_err());
    }

    /// Every mutation says what catching it looks like.
    ///
    /// A generic panic would be the way to credit an unrelated crash, and a
    /// panic with no reason would be the way to dodge naming a check.
    #[test]
    fn a_panicking_mutation_names_an_exact_marker_and_why_it_is_terminal() {
        for mutation in MUTATIONS {
            if let Caught::Panic { marker, why } | Caught::DegradedPanic { marker, why } =
                mutation.reddens
            {
                assert!(
                    marker.len() > 12,
                    "`{}` has no exact panic marker",
                    mutation.breaks
                );
                assert!(
                    why.len() > 40,
                    "`{}` claims nothing names it and does not say why",
                    mutation.breaks
                );
            }
        }
    }

    #[test]
    fn a_red_baseline_stops_before_any_mutation() {
        let error = require_green("self-test", Err(anyhow::anyhow!("already red")))
            .expect_err("a red baseline cannot be accepted");
        assert!(format!("{error:#}").contains("pristine self-test mutation baseline"));
    }

    #[test]
    fn a_wrong_machine_failure_is_inconclusive_not_caught() {
        let wanted = Caught::Check("the intended check");
        assert!(
            assess_selftest(
                wanted,
                qemu::SelftestOutcome::Failed(vec!["another check".to_string()])
            )
            .is_err()
        );
        assert!(
            assess_selftest(
                wanted,
                qemu::SelftestOutcome::Panicked {
                    transcript: "unrelated panic".to_string(),
                    failed: Vec::new(),
                }
            )
            .is_err()
        );
        assert!(assess_selftest(wanted, qemu::SelftestOutcome::Passed).is_ok_and(|caught| !caught));
        assert!(
            assess_selftest(
                wanted,
                qemu::SelftestOutcome::Failed(vec!["the intended check".to_string()])
            )
            .is_ok_and(|caught| caught)
        );
        assert!(
            assess_selftest(
                wanted,
                qemu::SelftestOutcome::Panicked {
                    transcript: "later corruption".to_string(),
                    failed: vec!["the intended check".to_string()],
                }
            )
            .is_ok_and(|caught| caught)
        );
    }

    #[test]
    fn a_panic_requires_its_exact_marker() {
        let wanted = Caught::Panic {
            marker: "the exact cause",
            why: "this fixture explains why no ordinary check can report the terminal defect",
        };
        assert!(
            assess_selftest(
                wanted,
                qemu::SelftestOutcome::Panicked {
                    transcript: "a different panic".to_string(),
                    failed: Vec::new(),
                }
            )
            .is_err()
        );
        assert!(
            assess_selftest(
                wanted,
                qemu::SelftestOutcome::Panicked {
                    transcript: "panic: the exact cause".to_string(),
                    failed: Vec::new(),
                }
            )
            .is_ok_and(|caught| caught)
        );
    }

    #[test]
    fn a_degraded_panic_requires_its_diagnosis_and_exact_marker() {
        let wanted = Caught::DegradedPanic {
            marker: "the exact degraded cause",
            why: "this terminal branch exists only before the degraded kernel can report a verdict",
        };
        assert!(
            assess_degraded_panic(
                wanted,
                qemu::DegradedOutcome::Panicked {
                    transcript: "KERNEL PANIC: the exact degraded cause".to_string(),
                    no_x2apic_diagnosed: false,
                }
            )
            .is_err()
        );
        assert!(
            assess_degraded_panic(
                wanted,
                qemu::DegradedOutcome::Panicked {
                    transcript: "KERNEL PANIC: another cause".to_string(),
                    no_x2apic_diagnosed: true,
                }
            )
            .is_err()
        );
        assert!(
            assess_degraded_panic(
                wanted,
                qemu::DegradedOutcome::Panicked {
                    transcript: "KERNEL PANIC: the exact degraded cause".to_string(),
                    no_x2apic_diagnosed: true,
                }
            )
            .is_ok_and(|caught| caught)
        );
        assert!(
            assess_degraded_panic(wanted, qemu::DegradedOutcome::Settled)
                .is_ok_and(|caught| !caught)
        );
    }

    #[test]
    fn an_inconclusive_machine_observation_gets_one_bounded_retry() {
        let wanted = Caught::Check("the intended check");
        let mut observations = 0;
        let caught = qualify_machine_mutation(wanted, || {
            observations += 1;
            Ok(if observations == 1 {
                qemu::SelftestOutcome::Panicked {
                    transcript: "incidental panic".to_string(),
                    failed: Vec::new(),
                }
            } else {
                qemu::SelftestOutcome::Failed(vec!["the intended check".to_string()])
            })
        })
        .expect("the exact retry is causal");
        assert!(caught);
        assert_eq!(observations, 2);

        observations = 0;
        let error = qualify_machine_mutation(wanted, || {
            observations += 1;
            Ok(qemu::SelftestOutcome::Panicked {
                transcript: "incidental panic".to_string(),
                failed: Vec::new(),
            })
        })
        .expect_err("repeated inconclusive observations still fail qualification");
        assert_eq!(observations, 2);
        assert!(error.to_string().contains("after 2 observations"));
    }

    /// No two mutations break the same property.
    ///
    /// A duplicate is a witness run spent twice on one question; machine
    /// entries make that cost a complete boot.
    #[test]
    fn no_two_mutations_name_the_same_property() {
        for (index, mutation) in MUTATIONS.iter().enumerate() {
            for earlier in &MUTATIONS[..index] {
                assert_ne!(earlier.breaks, mutation.breaks, "{}", mutation.breaks);
            }
        }
    }

    #[test]
    fn a_pattern_that_matches_once_is_applied_and_others_are_refused() {
        let mutation = Mutation {
            file: "x",
            find: "keep",
            replace: "drop",
            breaks: "a thing",
            reddens: Caught::Check("a thing goes red"),
        };
        assert_eq!(apply("a keep b", &mutation).expect("once"), "a drop b");
        assert!(apply("nothing here", &mutation).is_err());
        assert!(apply("keep and keep", &mutation).is_err());
    }
}
