use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 10,
        file: "xtask/src/evidence.rs",
        find: "            digest: sha256::hex(&sha256::digest(raw)),",
        replace: "            digest: sha256::hex(&sha256::digest(&[])),",
        breaks: "the documentation-child relation digest stops binding changed blob identities",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::parent_docs_relation_accepts_only_exact_regular_file_modifications",
        },
    },
    Mutation {
        id: 11,
        file: "xtask/src/evidence.rs",
        find: "    policy == \"same-commit\"",
        replace: "    false && policy == \"same-commit\"",
        breaks: "same-commit evidence no longer requires the live current mutation pass",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::same_commit_relation_requires_the_live_current_mutation_pass",
        },
    },
    Mutation {
        id: 12,
        file: "xtask/src/evidence.rs",
        find: "        words == [evidence, qualified],",
        replace: "        words.first() == Some(&evidence),",
        breaks: "a merge or unrelated qualification source can pose as the direct parent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::parent_docs_relation_requires_one_exact_direct_parent",
        },
    },
    Mutation {
        id: 13,
        file: "xtask/src/evidence.rs",
        find: "        ensure!(\n            ALLOWED.contains(&path),\n            \"parent-docs-only-v1 refuses `{path}`\"\n        );",
        replace: "        let _unchecked_closure_path = path;",
        breaks: "a source modification can enter the documentation-only closure commit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::parent_docs_relation_accepts_only_exact_regular_file_modifications",
        },
    },
    Mutation {
        id: 15,
        file: "xtask/src/qualification.rs",
        find: "            _lock: Some(pass),",
        replace: "            _lock: {\n                drop(pass);\n                None\n            },",
        breaks: "the selected mutation pass releases its global lock before evidence publication",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::snapshot_retains_mutation_lock_through_install_refusal",
        },
    },
    Mutation {
        id: 16,
        file: "xtask/src/qualification.rs",
        find: "    BundleFile {\n        prefix: \"qualification.codex-security-deep.findings\",\n        relative: \"qualification/codex-security-deep/findings.json\",\n    },\n",
        replace: "",
        breaks: "offline evidence omits the canonical Deep findings document",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::qualification_records_cannot_be_omitted_or_changed_offline",
        },
    },
    Mutation {
        id: 24,
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
        id: 26,
        file: "xtask/src/qualification/advisory.rs",
        find: "        .args([\"--deny\", \"warnings\", \"--file\"])",
        replace: "        .args([\"--deny\", \"none\", \"--file\"])",
        breaks: "the RustSec run does not deny advisory warnings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::advisory_refresh_and_review_commands_are_direct_exact_and_bounded",
        },
    },
    Mutation {
        id: 27,
        file: "xtask/src/qualification/advisory.rs",
        find: "    invalidate(paths, Kind::Advisory)?;",
        replace: "    let _preserved_advisory_pass = paths;",
        breaks: "a failed advisory attempt can preserve an older green record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::advisory_invalidation_precedes_even_failed_tool_resolution",
        },
    },
    Mutation {
        id: 28,
        file: "xtask/src/qualification/verify.rs",
        find: "    verify_advisory_qualification(paths, root, source)?;",
        replace: "    let _omitted_advisory_qualification = (source, root);",
        breaks: "offline evidence can omit the required advisory qualification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::a_missing_advisory_record_is_rejected",
        },
    },
    Mutation {
        id: 29,
        file: "xtask/src/qualification/verify.rs",
        find: "    require_source(&record, source)?;\n    require_advisory_inputs(paths, &record)?;",
        replace: "    let _unchecked_advisory_source = source;\n    require_advisory_inputs(paths, &record)?;",
        breaks: "an advisory result can be donated by another source commit and tree",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::an_advisory_record_bound_to_another_source_is_rejected",
        },
    },
    Mutation {
        id: 30,
        file: "xtask/src/qualification/verify.rs",
        find: "    ensure!(\n        record.get(\"input.cargo-lock.bytes\")? == lock.bytes.to_string()\n            && record.get(\"input.cargo-lock.sha256\")? == lock.sha256,\n        \"advisory qualification belongs to another Cargo.lock\"\n    );",
        replace: "    let _unchecked_advisory_lock = (record, lock);",
        breaks: "a current source can reuse an advisory result for different locked bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::a_stale_advisory_record_cannot_describe_another_lockfile",
        },
    },
    Mutation {
        id: 31,
        file: "xtask/src/qualification/verify.rs",
        find: "            ensure!(\n                bytes == ADVISORY_POLICY.as_bytes(),\n                \"advisory policy differs from the no-ignore phase-close policy\"\n            );",
        replace: "            ensure!(!bytes.is_empty(), \"advisory policy is empty\");",
        breaks: "an ignored RustSec advisory can enter a phase-close result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_advisory_policy_and_result_bytes_are_rejected_semantically",
        },
    },
    Mutation {
        id: 32,
        file: "xtask/src/qualification/advisory.rs",
        find: "    ensure!(\n        lines[1] == \"exit.code=0\",\n        \"cargo-audit transcript does not record a successful result\"\n    );",
        replace: "    ensure!(lines[1].starts_with(\"exit.code=\"), \"cargo-audit transcript has no result\");",
        breaks: "a failed cargo-audit execution can be reframed as a successful transcript",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_advisory_policy_and_result_bytes_are_rejected_semantically",
        },
    },
    Mutation {
        id: 33,
        file: "xtask/src/qualification/size_policy.rs",
        find: "            ADVISORY_TRANSCRIPT => (MAX_ADVISORY_TRANSCRIPT_BYTES, false),",
        replace: "            ADVISORY_TRANSCRIPT => (u64::MAX, false),",
        breaks: "an oversized advisory transcript reaches payload allocation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 34,
        file: "xtask/src/qualification/record.rs",
        find: "            record.get(\"database.url\")? == ADVISORY_DATABASE_URL\n                && is_git_id(record.get(\"database.revision\")?)\n                && is_git_id(record.get(\"database.tree\")?),",
        replace: "            record.get(\"database.url\")? == ADVISORY_DATABASE_URL,",
        breaks: "a malformed advisory database revision can enter the qualification record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::advisory_schema_binds_database_time_command_and_tool_identity",
        },
    },
    Mutation {
        id: 35,
        file: "xtask/src/qualification/record.rs",
        find: "            record.get(\"execution.command\")? == ADVISORY_COMMAND\n                && record.get(\"execution.exit-code\")? == \"0\"\n                && record.get(\"execution.refresh-command\")? == ADVISORY_REFRESH_COMMAND\n                && record.get(\"execution.refresh-exit-code\")? == \"0\",",
        replace: "            record.get(\"execution.exit-code\")? == \"0\"\n                && record.get(\"execution.refresh-command\")? == ADVISORY_REFRESH_COMMAND\n                && record.get(\"execution.refresh-exit-code\")? == \"0\",",
        breaks: "an advisory record can name a weaker or stale command line",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::advisory_schema_binds_database_time_command_and_tool_identity",
        },
    },
    Mutation {
        id: 36,
        file: "xtask/src/qualification/record.rs",
        find: "            observed != 0 && observed.to_string() == record.get(\"observation.utc-unix-seconds\")?,",
        replace: "            observed.to_string() == record.get(\"observation.utc-unix-seconds\")?,",
        breaks: "an advisory record can carry no real UTC observation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::advisory_schema_binds_database_time_command_and_tool_identity",
        },
    },
    Mutation {
        id: 37,
        file: "xtask/src/qualification/advisory.rs",
        find: "    let version = report\n        .strip_prefix(\"cargo-audit-audit \")\n        .or_else(|| report.strip_prefix(\"cargo-audit \"))\n        .context(\"cargo-audit printed no supported version identity\")?;",
        replace: "    let version = report;",
        breaks: "another executable can pose as the resolved RustSec scanner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::cargo_audit_version_is_one_exact_resolved_identity",
        },
    },
    Mutation {
        id: 38,
        file: "xtask/src/qualification/advisory.rs",
        find: "    ensure!(\n        report == ADVISORY_TOOL_VERSION,\n        \"cargo-audit version differs from the report schema qualification\"\n    );",
        replace: "    let _unqualified_advisory_tool_version = report;",
        breaks: "an unqualified cargo-audit release can emit a different report schema",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::cargo_audit_version_is_one_exact_resolved_identity",
        },
    },
    Mutation {
        id: 39,
        file: "xtask/src/qualification/advisory.rs",
        find: "    ensure!(\n        origin == ADVISORY_DATABASE_URL,\n        \"advisory database origin differs from the RustSec authority\"\n    );",
        replace: "    let _unchecked_advisory_origin = origin;",
        breaks: "another Git remote can pose as the RustSec advisory database",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::advisory_database_identity_rejects_forged_origin_and_dirty_bytes",
        },
    },
    Mutation {
        id: 40,
        file: "xtask/src/qualification/advisory.rs",
        find: "    ensure!(status.is_empty(), \"advisory database checkout is dirty\");",
        replace: "    let _unchecked_advisory_status = status;",
        breaks: "uncommitted advisory database bytes can escape the recorded Git identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::advisory_database_identity_rejects_forged_origin_and_dirty_bytes",
        },
    },
    Mutation {
        id: 41,
        file: "xtask/src/qualification/advisory.rs",
        find: "        ensure!(\n            advisory_database_identity(&directory, &database)? == database_identity,\n            \"advisory database changed during the revision-pinned audit\"\n        );",
        replace: "        let _unchecked_database_after_review = &database_identity;",
        breaks: "the advisory database can change during the revision-pinned review",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        id: 42,
        file: "xtask/src/qualification/advisory.rs",
        find: "        ensure!(\n            advisory_database_identity(&directory, &database)? == database_identity,\n            \"advisory database changed after cargo-audit completed\"\n        );",
        replace: "        let _unchecked_database_before_commit = &database_identity;",
        breaks: "the advisory database can change before qualification publication",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        id: 43,
        file: "xtask/src/qualification/advisory.rs",
        find: "    require_json_fields(\n        report,\n        &[\n            \"database\",\n            \"lockfile\",\n            \"settings\",\n            \"vulnerabilities\",\n            \"warnings\",\n        ],\n        \"cargo-audit report\",\n    )?;",
        replace: "    let _unchecked_advisory_report_schema = report;",
        breaks: "forged top-level cargo-audit fields can enter a clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        id: 44,
        file: "xtask/src/qualification/advisory.rs",
        find: "    require_json_fields(\n        database,\n        &[\"advisory-count\", \"last-commit\", \"last-updated\"],\n        \"cargo-audit database\",\n    )?;",
        replace: "    let _unchecked_advisory_database_schema = database;",
        breaks: "forged advisory database fields can enter a clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        id: 45,
        file: "xtask/src/qualification/advisory.rs",
        find: "    ensure!(\n        advisory_count != 0,\n        \"cargo-audit report names an empty advisory database\"\n    );",
        replace: "    let _unchecked_advisory_count = database;",
        breaks: "an empty advisory database can qualify a locked graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::an_empty_advisory_database_cannot_author_a_qualification",
        },
    },
    Mutation {
        id: 46,
        file: "xtask/src/qualification/advisory.rs",
        find: "            ensure!(\n                commit == database_revision,\n                \"cargo-audit report belongs to another advisory database revision\"\n            );",
        replace: "            let _unchecked_report_revision = database_revision;",
        breaks: "a cargo-audit report can name another advisory database revision",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        id: 47,
        file: "xtask/src/qualification/advisory.rs",
        find: "            ensure!(\n                is_rfc3339_timestamp(updated),\n                \"cargo-audit report has no RFC3339 database update time\"\n            );",
        replace: "            let _unchecked_advisory_update_time = updated;",
        breaks: "an advisory report can omit a real database update time",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        id: 48,
        file: "xtask/src/qualification/advisory.rs",
        find: "    require_json_fields(lockfile, &[\"dependency-count\"], \"cargo-audit lockfile\")?;",
        replace: "    let _unchecked_advisory_lock_schema = lockfile;",
        breaks: "forged lockfile report fields can enter a clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        id: 49,
        file: "xtask/src/qualification/advisory.rs",
        find: "    ensure!(\n        json_u64(\n            json_field(lockfile, \"dependency-count\")?,\n            \"dependency-count\"\n        )? != 0,\n        \"cargo-audit report names an empty lockfile\"\n    );",
        replace: "    let _unchecked_advisory_dependency_count = lockfile;",
        breaks: "an empty reported lockfile can pose as the audited dependency graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        id: 50,
        file: "xtask/src/qualification/advisory.rs",
        find: "    require_json_fields(\n        settings,\n        &[\n            \"ignore\",\n            \"informational_warnings\",\n            \"severity\",\n            \"target_arch\",\n            \"target_os\",\n        ],\n        \"cargo-audit settings\",\n    )?;",
        replace: "    let _unchecked_advisory_settings_schema = settings;",
        breaks: "forged cargo-audit setting fields can enter a clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        id: 51,
        file: "xtask/src/qualification/advisory.rs",
        find: "    ensure!(\n        json_array(json_field(settings, \"target_arch\")?, \"target_arch\")?.is_empty()\n            && json_array(json_field(settings, \"target_os\")?, \"target_os\")?.is_empty()\n            && matches!(json_field(settings, \"severity\")?, JsonValue::Null)\n            && json_array(json_field(settings, \"ignore\")?, \"ignore\")?.is_empty()\n            && json_string_array(\n                json_field(settings, \"informational_warnings\")?,\n                \"informational_warnings\"\n            )? == [\"unmaintained\", \"unsound\", \"notice\"],\n        \"cargo-audit report was generated with different audit settings\"\n    );",
        replace: "    let _unchecked_advisory_settings = settings;",
        breaks: "ignored advisories can be hidden inside a forged clean report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        id: 52,
        file: "xtask/src/qualification/advisory.rs",
        find: "    require_json_fields(\n        vulnerabilities,\n        &[\"count\", \"found\", \"list\"],\n        \"cargo-audit vulnerabilities\",\n    )?;",
        replace: "    let _unchecked_vulnerability_schema = vulnerabilities;",
        breaks: "forged vulnerability fields can enter a clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        id: 53,
        file: "xtask/src/qualification/advisory.rs",
        find: "    ensure!(\n        matches!(\n            json_field(vulnerabilities, \"found\")?,\n            JsonValue::Bool(false)\n        ) && json_u64(json_field(vulnerabilities, \"count\")?, \"vulnerability count\")? == 0\n            && json_array(json_field(vulnerabilities, \"list\")?, \"vulnerability list\")?.is_empty(),\n        \"cargo-audit report is not a consistent vulnerability-free result\"\n    );",
        replace: "    let _unchecked_vulnerability_result = vulnerabilities;",
        breaks: "a vulnerable or internally inconsistent report can pose as clean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        id: 54,
        file: "xtask/src/qualification/advisory.rs",
        find: "    ensure!(\n        warnings.is_empty(),\n        \"cargo-audit report contains a dependency warning\"\n    );",
        replace: "    let _unchecked_advisory_warnings = warnings;",
        breaks: "a dependency warning can enter a forged clean result",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::forged_green_advisory_json_is_rejected_semantically",
        },
    },
    Mutation {
        id: 58,
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
        id: 59,
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
        id: 60,
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
        id: 61,
        file: "xtask/src/evidence.rs",
        find: "    let path = checked_bundle_file(root, spec.relative, spec.prefix)?;",
        replace: "    let path = root.join(spec.relative);",
        breaks: "qualification hashing can start through a linked bundle parent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_bundle_parent_link_is_rejected_before_qualification_hashing",
        },
    },
    Mutation {
        id: 62,
        file: "xtask/src/qualification/vmware.rs",
        find: "    require_real_directory(directory, \"VMware selector directory\")?;",
        replace: "    let _unchecked_selector_directory = directory;",
        breaks: "a VMware selector name can substitute another in-root directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::a_vmware_selector_directory_cannot_be_substituted",
        },
    },
    Mutation {
        id: 63,
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
        id: 64,
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
        id: 65,
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
        id: 66,
        file: "xtask/src/qualification/record.rs",
        find: "    require_current_source(paths, record)?;\n    std::fs::rename(&temporary, &final_path)",
        replace: "    std::fs::rename(&temporary, &final_path)",
        breaks: "a qualification record can commit after its source checkout changes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::source_is_rechecked_immediately_before_a_record_commits",
        },
    },
    Mutation {
        id: 68,
        file: "xtask/src/qualification/vmware.rs",
        find: "        let observation = observe_running_vm(self.paths.root(), &self.vmrun, artifacts.vmx)?;",
        replace: "        let observation = format!(\"{VMRUN_LIST_HEADER}1\\n{}\", artifacts.vmx.display());",
        breaks: "VMware qualification fabricates availability text instead of executing vmrun",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        id: 69,
        file: "xtask/src/qualification.rs",
        find: "const VMRUN_LIST_ARGS: &[&str] = &[\"-T\", \"ws\", \"list\"];",
        replace: "const VMRUN_LIST_ARGS: &[&str] = &[\"-T\", \"ws\", \"stop\"];",
        breaks: "VMware qualification invokes another vmrun operation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::vmrun_list_requires_one_exact_running_machine",
        },
    },
    Mutation {
        id: 70,
        file: "xtask/src/qualification/vmware.rs",
        find: "    let before = run(&tool.path, &[], ExitPolicy::Any)?;",
        replace: "    let before = tool.version.clone();",
        breaks: "the VMware observer can skip its immediate pre-execution tool identity probe",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::vmrun_observation_executes_list_between_exact_tool_probes",
        },
    },
    Mutation {
        id: 71,
        file: "xtask/src/qualification/vmware.rs",
        find: "    let after = run(&tool.path, &[], ExitPolicy::Any)?;",
        replace: "    let after = tool.version.clone();",
        breaks: "the VMware observer can skip its post-execution tool identity probe",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::vmrun_observation_executes_list_between_exact_tool_probes",
        },
    },
    Mutation {
        id: 72,
        file: "xtask/src/qualification/vmware.rs",
        find: "    ensure!(\n        !report.contains(['\\0', '\\r']),\n        \"vmrun list contains a forbidden control byte\"\n    );",
        replace: "    ensure!(!report.contains('\\0'), \"vmrun list contains a NUL byte\");",
        breaks: "the archived VMware observer can use noncanonical CR line endings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::vmrun_list_requires_one_exact_running_machine",
        },
    },
    Mutation {
        id: 73,
        file: "xtask/src/qualification/vmware.rs",
        find: "        parsed == 1 && paths.len() == parsed,",
        replace: "        parsed >= 1 && paths.len() == parsed,",
        breaks: "another running VMware machine can share the qualification observation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::vmrun_list_requires_one_exact_running_machine",
        },
    },
    Mutation {
        id: 74,
        file: "xtask/src/qualification/vmware.rs",
        find: "        observed == expected,\n        \"vmrun reported another machine: {}\",",
        replace: "        observed != expected,\n        \"vmrun reported another machine: {}\",",
        breaks: "vmrun may report a foreign VMX instead of the generated selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::vmrun_list_requires_one_exact_running_machine",
        },
    },
    Mutation {
        id: 75,
        file: "xtask/src/qualification/vmware.rs",
        find: "        actual == expected,\n        \"VMware generation inventory differs: missing {:?}; unexpected {:?}\",",
        replace: "        actual.is_superset(&expected),\n        \"VMware generation inventory differs: missing {:?}; unexpected {:?}\",",
        breaks: "stale VMware firmware or suspend state survives generation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::generation_receipt_binds_reset_source_selector_and_machine_bytes",
        },
    },
    Mutation {
        id: 76,
        file: "xtask/src/qualification/vmware.rs",
        find: "                .is_file(),\n            \"VMware generation entry is not a regular file: {}\",",
        replace: "                .is_file() || true,\n            \"VMware generation entry is not a regular file: {}\",",
        breaks: "an authoritative VMware artifact name can be a directory or link",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::generation_receipt_binds_reset_source_selector_and_machine_bytes",
        },
    },
    Mutation {
        id: 77,
        file: "xtask/src/qualification/vmware.rs",
        find: "    generation_transcript_name(&generation)",
        replace: "    Ok(VMWARE_TRANSCRIPT.to_string())",
        breaks: "successive VMware generations reuse one replayable transcript path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::each_vmware_generation_owns_a_distinct_serial_file",
        },
    },
    Mutation {
        id: 81,
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
        id: 102,
        file: "xtask/src/evidence.rs",
        find: "    verify_record(paths, &root, &record)?;\n    crate::git::resolved()?.require_unchanged()?;\n    let manifest = root.join(MANIFEST_NAME);\n    write_manifest(&root, &manifest, &record.render())?;",
        replace: "    crate::git::resolved()?.require_unchanged()?;\n    let manifest = root.join(MANIFEST_NAME);\n    write_manifest(&root, &manifest, &record.render())?;\n    verify_record(paths, &root, &record)?;",
        breaks: "a failed final evidence verification leaves a committed manifest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::an_evidence_attempt_invalidates_an_older_success_first",
        },
    },
    Mutation {
        id: 104,
        file: "xtask/src/qualification/tests.rs",
        find: "    let mutation_start = mutation_source\n        .rfind(\"\\npub fn run(paths: &Paths) -> Result<()> {\")\n        .unwrap();",
        replace: "    let mutation_start = 0;",
        breaks: "the exceptional-order check admits mutation registry fixtures as production markers",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::exceptional_tasks_commit_only_after_their_terminal_verdict",
        },
    },
    Mutation {
        id: 119,
        file: "xtask/src/qualification/record.rs",
        find: "    ensure!(actual == expected, \"qualification checksum does not match\");",
        replace: "    ensure!(is_digest(expected), \"qualification checksum does not match\");",
        breaks: "a changed qualification record retains its old checksum",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_qualification_record_is_canonical_and_checksummed",
        },
    },
    Mutation {
        id: 120,
        file: "xtask/src/qualification/record.rs",
        find: "    ensure!(\n        actual == expected,\n        \"qualification schema differs: missing {:?}; unknown {:?}\",",
        replace: "    ensure!(\n        expected.is_subset(&actual),\n        \"qualification schema differs: missing {:?}; unknown {:?}\",",
        breaks: "an invented qualification claim is accepted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 121,
        file: "xtask/src/qualification/record.rs",
        find: "        record.get(\"source.commit\")? == source.commit,",
        replace: "        is_git_id(record.get(\"source.commit\")?),",
        breaks: "qualification from another commit remains current",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::stale_source_identity_is_rejected_field_by_field",
        },
    },
    Mutation {
        id: 122,
        file: "xtask/src/qualification/record.rs",
        find: "        record.get(\"source.tree\")? == source.tree,",
        replace: "        is_git_id(record.get(\"source.tree\")?),",
        breaks: "qualification from another tree remains current",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::stale_source_identity_is_rejected_field_by_field",
        },
    },
    Mutation {
        id: 123,
        file: "xtask/src/qualification/record.rs",
        find: "            std::fs::remove_dir_all(&directory)\n                .with_context(|| format!(\"invalidating stale {} qualification\", kind.slug()))?;",
        replace: "            return Ok(());",
        breaks: "a new exceptional attempt leaves an older pass valid",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::starting_an_attempt_invalidates_an_older_pass",
        },
    },
    Mutation {
        id: 124,
        file: "xtask/src/qualification/vmware.rs",
        find: "        receipt == generation_record(source, kind, directory, generation)?,",
        replace: "        receipt.get(\"input.kernel\")? == kind.slug(),",
        breaks: "a VMware receipt ignores its source and machine bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::generation_receipt_binds_reset_source_selector_and_machine_bytes",
        },
    },
    Mutation {
        id: 125,
        file: "xtask/src/qualification/vmware.rs",
        find: "        transcript.bytes != 0 && transcript.sha256 != empty_digest(),",
        replace: "        transcript.bytes == 0 || transcript.sha256 != empty_digest(),",
        breaks: "an untouched VMware reset counts as execution",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::generation_receipt_binds_reset_source_selector_and_machine_bytes",
        },
    },
    Mutation {
        id: 126,
        file: "xtask/src/qualification/vmware.rs",
        find: "        reset.bytes == 0 && reset.sha256 == empty_digest(),",
        replace: "        reset.bytes <= reset.bytes,",
        breaks: "a generation receipt can be issued over an old transcript",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::generation_receipt_binds_reset_source_selector_and_machine_bytes",
        },
    },
    Mutation {
        id: 127,
        file: "xtask/src/qualification/vmware.rs",
        find: "    for path in [receipt.to_path_buf(), parent.join(VMWARE_GENERATION_TEMP)] {",
        replace: "    for path in std::iter::empty::<PathBuf>() {",
        breaks: "VMware generation preserves stale complete and partial receipts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::generation_start_invalidates_both_receipt_names",
        },
    },
    Mutation {
        id: 128,
        file: "xtask/src/qualification/verify.rs",
        find: "            record.get(&format!(\"{prefix}.sha256\"))? == digest.sha256,",
        replace: "            digest.bytes == digest.bytes,",
        breaks: "inner qualification support can differ at equal length",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::copied_support_bytes_must_match_their_inner_record",
        },
    },
    Mutation {
        id: 129,
        file: "xtask/src/qualification/verify.rs",
        find: "        record.get(\"input.registry.sha256\")? == registry,",
        replace: "        is_digest(registry),",
        breaks: "mutation qualification accepts another registry",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        id: 130,
        file: "xtask/src/qualification/verify.rs",
        find: "        record.get(\"input.mutations\")? == mutation_count.to_string(),",
        replace: "        mutation_count != 0,",
        breaks: "mutation qualification accepts another registry cardinality",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        id: 131,
        file: "xtask/src/qualification/verify.rs",
        find: "            mutation.registry == registry",
        replace: "            true",
        breaks: "the newest mutation pass can name another registry",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        id: 132,
        file: "xtask/src/qualification/verify.rs",
        find: "                && mutation.mutation_count == mutation_count",
        replace: "                && true",
        breaks: "the newest mutation pass can name another count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        id: 133,
        file: "xtask/src/qualification/verify.rs",
        find: "                && record.get(\"source.commit\")? == mutation.commit",
        replace: "                && is_git_id(&mutation.commit)",
        breaks: "the newest mutation pass can belong to another commit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        id: 134,
        file: "xtask/src/qualification/verify.rs",
        find: "                && record.get(\"input.source.sha256\")? == mutation.source_digest",
        replace: "                && is_digest(&mutation.source_digest)",
        breaks: "the newest mutation pass can describe other source bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        id: 135,
        file: "xtask/src/qualification/verify.rs",
        find: "                && record.get(\"input.index.sha256\")? == mutation.index_digest",
        replace: "                && is_digest(&mutation.index_digest)",
        breaks: "the newest mutation pass can describe another index",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        id: 136,
        file: "xtask/src/qualification/verify.rs",
        find: "                && record.get(\"artifact.report.sha256\")? == mutation.report_digest,",
        replace: "                && is_digest(&mutation.report_digest),",
        breaks: "the newest mutation pass can point at another report",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        id: 137,
        file: "xtask/src/qualification/verify.rs",
        find: "fn require_record_inputs(record: &Record, expected: &Record) -> Result<()> {\n    for (key, value) in &expected.fields {\n        ensure!(\n            record.get(key)? == value,",
        replace: "fn require_record_inputs(record: &Record, expected: &Record) -> Result<()> {\n    for (key, value) in &expected.fields {\n        ensure!(\n            record.fields.contains_key(key),",
        breaks: "current TLC and VMware tool inputs are compared by name only",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::newest_mutation_and_tool_inputs_are_compared_field_by_field",
        },
    },
    Mutation {
        id: 154,
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
        id: 155,
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
        id: 156,
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
        id: 157,
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
        id: 158,
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
        id: 159,
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
        id: 160,
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
        id: 467,
        file: "xtask/src/evidence.rs",
        find: "            self.fields.insert(key.clone(), value).is_none(),",
        replace: "            self.fields.insert(key.clone(), value).is_none() || true,",
        breaks: "a later evidence field silently replaces an earlier claim",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_duplicate_evidence_key_is_rejected_before_canonical_rendering",
        },
    },
    Mutation {
        id: 468,
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
        id: 469,
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
        id: 470,
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
        id: 471,
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
        id: 472,
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
        id: 473,
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
        id: 475,
        file: "xtask/src/qualification/vmware.rs",
        find: "    Ok(version.to_string())",
        replace: "    Ok(report.trim().to_string())",
        breaks: "vmrun usage text is recorded as part of its version identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::vmrun_version_is_separated_from_its_usage_text",
        },
    },
    Mutation {
        id: 476,
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
        id: 477,
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
        id: 652,
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
        id: 658,
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
        id: 659,
        file: "xtask/src/qualification/vmware.rs",
        find: "            transcript.source.file_name().and_then(|name| name.to_str())\n                == Some(generation_transcript_name(generation)?.as_str()),",
        replace: "            true,",
        breaks: "a transcript path from another valid VMware generation enters the capture",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::one_immutable_vmware_generation_is_captured_before_publication",
        },
    },
    Mutation {
        id: 660,
        file: "xtask/src/qualification/vmware.rs",
        find: "            receipt == generation_record_from_digests(source, kind, generation, &digests)?,",
        replace: "            generation_record_from_digests(source, kind, generation, &digests).is_ok(),",
        breaks: "machine bytes from multiple VMware generations share one captured receipt",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::one_immutable_vmware_generation_is_captured_before_publication",
        },
    },
    Mutation {
        id: 661,
        file: "xtask/src/qualification/vmware.rs",
        find: "                digest_file(&file.source)? == file.digest,",
        replace: "                true,",
        breaks: "a VMware source artifact can change after its immutable capture",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::one_immutable_vmware_generation_is_captured_before_publication",
        },
    },
    Mutation {
        id: 662,
        file: "xtask/src/qualification/vmware.rs",
        find: "            ensure!(\n                destination == file.captured && digest_file(&destination)? == file.digest,\n                \"captured VMware artifact changed during publication\"\n            );",
        replace: "            std::fs::copy(&file.source, &destination)\n                .with_context(|| format!(\"recopying {}\", file.source.display()))?;",
        breaks: "VMware publication reopens moving sources instead of using the immutable capture",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::one_immutable_vmware_generation_is_captured_before_publication",
        },
    },
    Mutation {
        id: 666,
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
        id: 668,
        file: "xtask/src/evidence.rs",
        find: "    let root = crate::mutate::workspace::prepare_real_directory_tree(\n        &evidence_root(paths),\n        \"evidence root\",\n    )?;",
        replace: "    let root = evidence_root(paths);\n    std::fs::create_dir_all(&root)\n        .with_context(|| format!(\"creating evidence directory {}\", root.display()))?;",
        breaks: "a redirected target receives evidence bytes before its ancestry is refused",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_redirected_target_is_refused_before_evidence_root_creation",
        },
    },
    Mutation {
        id: 669,
        file: "xtask/src/qualification/record.rs",
        find: "fn invalidate(paths: &Paths, kind: Kind) -> Result<()> {\n    let root = crate::mutate::workspace::prepare_real_directory_tree(\n        &qualification_root(paths),\n        \"qualification root\",\n    )?;",
        replace: "fn invalidate(paths: &Paths, kind: Kind) -> Result<()> {\n    let root = qualification_root(paths);\n    std::fs::create_dir_all(&root).with_context(|| format!(\"creating {}\", root.display()))?;",
        breaks: "a redirected target receives qualification bytes before its ancestry is refused",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_redirected_target_is_refused_before_qualification_root_creation",
        },
    },
    Mutation {
        id: 680,
        file: "xtask/src/evidence.rs",
        find: "fn require_real_directory(path: &Path, label: &str) -> Result<()> {\n    crate::mutate::workspace::canonical_existing_real_directory(path, label)?;\n    Ok(())\n}",
        replace: "fn require_real_directory(path: &Path, label: &str) -> Result<()> {\n    let metadata = std::fs::symlink_metadata(path)\n        .with_context(|| format!(\"inspecting {}\", path.display()))?;\n    ensure!(\n        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),\n        \"{label} is not a real directory: {}\",\n        path.display()\n    );\n    Ok(())\n}",
        breaks: "an evidence root can hide a redirected ancestor behind a real final directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_substituted_evidence_root_is_never_recursively_reset",
        },
    },
    Mutation {
        id: 681,
        file: "xtask/src/qualification.rs",
        find: "fn require_real_directory(path: &Path, label: &str) -> Result<()> {\n    crate::mutate::workspace::canonical_existing_real_directory(path, label)?;\n    Ok(())\n}",
        replace: "fn require_real_directory(path: &Path, label: &str) -> Result<()> {\n    let metadata = std::fs::symlink_metadata(path)\n        .with_context(|| format!(\"inspecting {}\", path.display()))?;\n    ensure!(\n        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),\n        \"{label} is not a real directory: {}\",\n        path.display()\n    );\n    Ok(())\n}",
        breaks: "a qualification root can hide a redirected ancestor behind a real final directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_qualification_root_cannot_be_substituted",
        },
    },
    Mutation {
        id: 702,
        file: "xtask/src/qualification/verify.rs",
        find: "            crate::model::verify_transcript(name, &bytes)?;",
        replace: "            let _unchecked_model_transcript = (name, &bytes);",
        breaks: "offline evidence can accept one valid TLC run donated under another identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::model_support_rejects_omitted_substituted_and_reordered_runs",
        },
    },
    Mutation {
        id: 703,
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
        id: 726,
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        initial <= maximum,\n        \"{label} is {initial} bytes, exceeding the {maximum}-byte allocation limit\"\n    );",
        replace: "    ensure!(\n        true,\n        \"{label} is {initial} bytes, exceeding the {maximum}-byte allocation limit\"\n    );",
        breaks: "a semantic qualification artifact can allocate from an already oversized length",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::semantic_artifacts_are_bounded_before_and_during_allocation",
        },
    },
    Mutation {
        id: 727,
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        bytes.len() as u64 <= maximum,\n        \"{label} grew beyond the {maximum}-byte allocation limit while being read\"\n    );",
        replace: "    ensure!(\n        true,\n        \"{label} grew beyond the {maximum}-byte allocation limit while being read\"\n    );",
        breaks: "a semantic qualification artifact can grow past its allocation cap during the read",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::semantic_artifacts_are_bounded_before_and_during_allocation",
        },
    },
    Mutation {
        id: 728,
        file: "xtask/src/qualification/size_policy.rs",
        find: "        ensure!(\n            length == maximum,\n            \"qualification artifact {name} is {length} bytes, expected exactly {maximum}\"\n        );",
        replace: "        ensure!(\n            length <= maximum,\n            \"qualification artifact {name} is {length} bytes, expected exactly {maximum}\"\n        );",
        breaks: "an undersized deterministic machine image can enter qualification evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 729,
        file: "xtask/src/qualification/vmware.rs",
        find: "            initial == maximum,\n            \"{label} is {initial} bytes, expected exactly {maximum}\"",
        replace: "            initial >= maximum,\n            \"{label} is {initial} bytes, expected exactly {maximum}\"",
        breaks: "an oversized sparse VMware image reaches the qualification capture stream",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::vmware_capture_preflights_image_size_before_streaming",
        },
    },
    Mutation {
        id: 730,
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
        id: 741,
        file: "xtask/src/qualification/vmware.rs",
        find: "    require_observed_vmx(&observed, &expected)\n}",
        replace: "    require_observed_vmx(&observed, &observed)\n}",
        breaks: "offline VMware evidence can substitute another absolute running VMX",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::offline_vmware_observer_is_bound_to_its_exact_selector",
        },
    },
    Mutation {
        id: 892,
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        initial <= maximum,\n        \"{} is {initial} bytes, exceeding its {maximum}-byte digest limit\",\n        path.display()\n    );",
        replace: "    ensure!(\n        true,\n        \"{} is {initial} bytes, exceeding its {maximum}-byte digest limit\",\n        path.display()\n    );",
        breaks: "a generic qualification digest allocates from an oversized initial file",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::generic_qualification_digests_have_a_streaming_ceiling",
        },
    },
    Mutation {
        id: 893,
        file: "xtask/src/qualification.rs",
        find: "    ensure!(\n        bytes <= maximum,\n        \"{} grew beyond its {maximum}-byte digest limit while hashing\",\n        path.display()\n    );",
        replace: "    ensure!(\n        true,\n        \"{} grew beyond its {maximum}-byte digest limit while hashing\",\n        path.display()\n    );",
        breaks: "a qualification digest can grow beyond its semantic ceiling after preflight",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::generic_qualification_digest_growth_is_refused_after_measurement",
        },
    },
    Mutation {
        id: 894,
        file: "xtask/src/qualification.rs",
        find: "            length <= MAX_MODEL_FILE_BYTES,",
        replace: "            true,",
        breaks: "one oversized tracked model input reaches the hashing stream",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::tracked_model_files_are_size_checked_before_streaming",
        },
    },
    Mutation {
        id: 895,
        file: "xtask/src/qualification.rs",
        find: "            total <= MAX_MODEL_TREE_BYTES,",
        replace: "            true,",
        breaks: "the tracked model tree has no aggregate byte ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::tracked_model_tree_has_an_independent_total_size_bound",
        },
    },
    Mutation {
        id: 896,
        file: "xtask/src/qualification.rs",
        find: "            .update(&(relative.len() as u64).to_le_bytes())",
        replace: "            .update(&0u64.to_le_bytes())",
        breaks: "the model-tree digest omits its path-length frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::streamed_model_tree_digest_preserves_the_length_delimited_schema",
        },
    },
    Mutation {
        id: 897,
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
        id: 898,
        file: "xtask/src/qualification/record.rs",
        find: "    ensure!(\n        before == after,\n        \"qualification source identity changed while it was captured\"\n    );",
        replace: "    ensure!(\n        before == before,\n        \"qualification source identity changed while it was captured\"\n    );",
        breaks: "qualification accepts two different clean source identities",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::source_capture_requires_two_clean_views_of_one_identity",
        },
    },
    Mutation {
        id: 899,
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
        id: 900,
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
        id: 1206,
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
            name: "qualification::tests::size_policy::generic_qualification_digest_truncation_is_refused_after_measurement",
        },
    },
    Mutation {
        id: 1208,
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
        id: 1209,
        file: "xtask/src/qualification/record.rs",
        find: r#"    let text = read_bounded_text(path, MAX_RECORD_BYTES, "qualification record")?;"#,
        replace: r#"    let text = read_bounded_text(path, u64::MAX, "qualification record")?;"#,
        breaks: "qualification records are parsed without their caller-specific allocation ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::an_oversized_qualification_record_is_refused_before_parsing",
        },
    },
    Mutation {
        id: 1210,
        file: "xtask/src/qualification/vmware.rs",
        find: r#"    let text = read_bounded_text(path, MAX_VMWARE_TEXT_BYTES, "VMware generation receipt")?;"#,
        replace: r#"    let text = read_bounded_text(path, u64::MAX, "VMware generation receipt")?;"#,
        breaks: "VMware generation receipts are parsed without their caller-specific allocation ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::an_oversized_generation_receipt_is_refused_before_parsing",
        },
    },
    Mutation {
        id: 1213,
        file: "xtask/src/qualification/advisory.rs",
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
            name: "qualification::tests::advisory::advisory_transcript_rendering_refuses_output_beyond_its_ceiling",
        },
    },
    Mutation {
        id: 1214,
        file: "xtask/src/qualification/advisory.rs",
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
            name: "qualification::tests::advisory::advisory_transcript_verification_refuses_bytes_beyond_its_ceiling",
        },
    },
    Mutation {
        id: 1215,
        file: "xtask/src/qualification/vmware.rs",
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
            name: "qualification::tests::vmware::vmware_capture_refuses_growth_after_its_preflight_measurement",
        },
    },
    Mutation {
        id: 1216,
        file: "xtask/src/qualification/vmware.rs",
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
            name: "qualification::tests::vmware::vmware_capture_refuses_truncation_after_exact_preflight",
        },
    },
    Mutation {
        id: 1220,
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
        id: 1221,
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
        id: 1222,
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
        id: 1223,
        file: "xtask/src/evidence.rs",
        find: r#"    ensure!(
        crate::mutate::workspace::open_real_file(path, label)?
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
        id: 1224,
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
        id: 1225,
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
        id: 1226,
        file: "xtask/src/qualification/size_policy.rs",
        find: r#"            ADVISORY_POLICY_PATH => (MAX_ADVISORY_POLICY_BYTES, false),"#,
        replace: r#"            ADVISORY_POLICY_PATH => (MAX_ADVISORY_POLICY_BYTES + 1, false),"#,
        breaks: "the advisory policy artifact selects a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 1227,
        file: "xtask/src/qualification/size_policy.rs",
        find: r#"        Kind::Mutation => (MAX_MUTATION_REPORT_BYTES, false),"#,
        replace: r#"        Kind::Mutation => (MAX_MUTATION_REPORT_BYTES + 1, false),"#,
        breaks: "the mutation report artifact selects a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 1228,
        file: "xtask/src/qualification/size_policy.rs",
        find: r#"        Kind::Model => (MAX_MODEL_TRANSCRIPT_BYTES, false),"#,
        replace: r#"        Kind::Model => (MAX_MODEL_TRANSCRIPT_BYTES + 1, false),"#,
        breaks: "model transcript artifacts select a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 1229,
        file: "xtask/src/qualification/size_policy.rs",
        find: r#"            VMWARE_IMAGE => (boot_image_bytes(), true),"#,
        replace: r#"            VMWARE_IMAGE => (boot_image_bytes() + 1, true),"#,
        breaks: "VMware boot images select the wrong exact artifact extent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 1230,
        file: "xtask/src/qualification/size_policy.rs",
        find: r#"            VMWARE_NAMESPACE => (namespace_bytes(), true),"#,
        replace: r#"            VMWARE_NAMESPACE => (namespace_bytes() + 1, true),"#,
        breaks: "VMware namespaces select the wrong exact artifact extent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 1231,
        file: "xtask/src/qualification/size_policy.rs",
        find: r#"            VMWARE_TRANSCRIPT => (MAX_VMWARE_TRANSCRIPT_BYTES, false),"#,
        replace: r#"            VMWARE_TRANSCRIPT => (MAX_VMWARE_TRANSCRIPT_BYTES + 1, false),"#,
        breaks: "VMware serial transcripts select a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 1232,
        file: "xtask/src/qualification/size_policy.rs",
        find: r#"            | VMWARE_OBSERVER => (MAX_VMWARE_TEXT_BYTES, false),"#,
        replace: r#"            | VMWARE_OBSERVER => (MAX_VMWARE_TEXT_BYTES + 1, false),"#,
        breaks: "VMware text artifacts select a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 1233,
        file: "xtask/src/qualification/size_policy.rs",
        find: r#"        return Ok((MAX_RECORD_BYTES, false));"#,
        replace: r#"        return Ok((MAX_RECORD_BYTES + 1, false));"#,
        breaks: "qualification bundle records select a larger size policy than registered",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 1234,
        file: "xtask/src/qualification/size_policy.rs",
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
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 1235,
        file: "xtask/src/qualification/size_policy.rs",
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
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 1236,
        file: "xtask/src/qualification/size_policy.rs",
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
            name: "qualification::tests::size_policy::every_qualification_size_policy_has_an_independent_sparse_caller_witness",
        },
    },
    Mutation {
        id: 1376,
        file: "xtask/src/qualification/advisory.rs",
        find: "        (DatabaseBinding::Pinned, JsonValue::Null, JsonValue::Null) => None,",
        replace: "        (_, JsonValue::Null, JsonValue::Null) => None,",
        breaks: "a population report can omit the database identity cargo-audit fetched",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::a_population_transcript_without_database_identity_is_rejected",
        },
    },
    Mutation {
        id: 1377,
        file: "xtask/src/qualification/advisory.rs",
        find: "    ensure!(\n        audited.advisory_count == fetched.advisory_count,\n        \"the pinned audit and the population run saw different advisory databases\"\n    );",
        replace: "    let _unchecked_database_agreement = (audited.advisory_count, fetched.advisory_count);",
        breaks: "the pinned audit can see a different advisory database than the population run attested",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::the_pinned_audit_must_see_the_population_runs_database",
        },
    },
    Mutation {
        id: 1378,
        file: "xtask/src/qualification/record.rs",
        find: "        ensure!(\n            is_rfc3339_timestamp(record.get(\"database.last-updated\")?),\n            \"advisory qualification names no RFC3339 database update time\"\n        );",
        replace: "        let _unchecked_database_update_time = record.get(\"database.last-updated\")?;",
        breaks: "an advisory record can carry a malformed database update time",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::advisory_schema_binds_database_time_command_and_tool_identity",
        },
    },
    Mutation {
        id: 1379,
        file: "xtask/src/qualification/verify.rs",
        find: "        ensure!(\n            record.get(\"database.last-updated\")? == attestation.last_updated,\n            \"advisory record names another database update time\"\n        );",
        replace: "        let _unchecked_attested_update_time = attestation.last_updated;",
        breaks: "an advisory record can name an update time its population transcript does not attest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::advisory::an_advisory_record_must_repeat_the_attested_database_update_time",
        },
    },
    Mutation {
        id: 1443,
        file: "xtask/src/evidence.rs",
        find: "                && fields[4] == \"M\",",
        replace: "                && !fields[4].is_empty(),",
        breaks: "a parent type change passes as a documentation edit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::the_parent_diff_admits_only_sorted_unique_regular_modifications",
        },
    },
    Mutation {
        id: 1444,
        file: "xtask/src/evidence.rs",
        find: "                && fields[0] == \":100644\"\n                && fields[1] == \"100644\"",
        replace: "                && fields[0].starts_with(':')\n                && !fields[1].is_empty()",
        breaks: "an executable-mode parent change passes as a documentation edit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::the_parent_diff_admits_only_sorted_unique_regular_modifications",
        },
    },
    Mutation {
        id: 1445,
        file: "xtask/src/evidence.rs",
        find: "previous.as_str() < path),",
        replace: "previous.as_str() != path),",
        breaks: "parent documentation paths may arrive out of order",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::the_parent_diff_admits_only_sorted_unique_regular_modifications",
        },
    },
    Mutation {
        id: 1446,
        file: "xtask/src/evidence.rs",
        find: "        !raw.is_empty() && raw.last() == Some(&0),",
        replace: "        !raw.is_empty(),",
        breaks: "a parent diff without its terminal NUL is parsed",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::the_parent_diff_admits_only_sorted_unique_regular_modifications",
        },
    },
    Mutation {
        id: 1447,
        file: "xtask/src/evidence.rs",
        find: "        fields.len() % 2 == 0,",
        replace: "        !fields.is_empty(),",
        breaks: "a parent diff with an incomplete row is parsed",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::the_parent_diff_admits_only_sorted_unique_regular_modifications",
        },
    },
    Mutation {
        id: 1482,
        file: "xtask/src/git.rs",
        find: "        let mut work_tree = OsString::from(\"--work-tree=\");\n        work_tree.push(root);\n        let mut command = Command::new(&self.path);",
        replace: "        let mut work_tree = OsString::from(\"--work-tree=\");\n        work_tree.push(root);\n        let mut command = Command::new(\"git\");",
        breaks: "a run's Git queries spawn whatever `git` PATH names",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::a_run_spawns_the_git_it_resolved_whatever_path_later_says",
        },
    },
    Mutation {
        id: 1483,
        file: "xtask/src/git.rs",
        find: "    let path = std::fs::canonicalize(&found)\n        .with_context(|| format!(\"canonicalizing Git {}\", found.display()))?;",
        replace: "    let path = found;",
        breaks: "resolved Git is recorded under a non-canonical path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::a_run_spawns_the_git_it_resolved_whatever_path_later_says",
        },
    },
    Mutation {
        id: 1484,
        file: "xtask/src/git.rs",
        find: "            measure(&self.path)? == (self.bytes, self.sha256.clone()),",
        replace: "            measure(&self.path).is_ok(),",
        breaks: "a Git executable changed during a run still publishes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::a_changed_executable_blocks_publication",
        },
    },
    Mutation {
        id: 1485,
        file: "xtask/src/git.rs",
        find: "        version.starts_with(\"git version \") && !version.contains(['\\r', '\\n', '\\0']),",
        replace: "        !version.contains(['\\r', '\\n', '\\0']),",
        breaks: "any executable that prints one line passes as Git",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::an_executable_that_is_not_git_is_refused_by_its_version_identity",
        },
    },
    Mutation {
        id: 1486,
        file: "xtask/src/git.rs",
        find: "            .is_ok_and(|value| value > 0 && value.to_string() == bytes),",
        replace: "            .is_ok(),",
        breaks: "a recorded Git length may be zero or noncanonical",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::recorded_git_identity_is_validated_field_by_field",
        },
    },
    Mutation {
        id: 1487,
        file: "xtask/src/git.rs",
        find: "        Path::new(get(FIELDS[1])?).is_absolute(),",
        replace: "        !get(FIELDS[1])?.is_empty(),",
        breaks: "a recorded Git path may be relative",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::recorded_git_identity_is_validated_field_by_field",
        },
    },
    Mutation {
        id: 1488,
        file: "xtask/src/git.rs",
        find: "        sha256.len() == 64\n            && sha256\n                .bytes()\n                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),",
        replace: "        sha256.len() == 64,",
        breaks: "a recorded Git digest may be uppercase or non-hexadecimal",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::recorded_git_identity_is_validated_field_by_field",
        },
    },
    Mutation {
        id: 1489,
        file: "xtask/src/git.rs",
        find: "        get(FIELDS[3])?.starts_with(\"git version \"),",
        replace: "        !get(FIELDS[3])?.is_empty(),",
        breaks: "a recorded Git version may name another program",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::recorded_git_identity_is_validated_field_by_field",
        },
    },
    Mutation {
        id: 1490,
        file: "xtask/src/evidence.rs",
        find: "    for (key, value) in crate::git::resolved()?.fields() {\n        record.insert(key, value)?;\n    }\n",
        replace: "",
        breaks: "the evidence manifest omits the Git that established its source",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::sources::every_source_identity_names_the_git_that_established_it",
        },
    },
    Mutation {
        id: 1491,
        file: "xtask/src/qualification/record.rs",
        find: "    record.insert(\"source.tree\", &source.tree)?;\n    for (key, value) in crate::git::resolved()?.fields() {\n        record.insert(key, value)?;\n    }\n",
        replace: "    record.insert(\"source.tree\", &source.tree)?;\n",
        breaks: "a qualification record omits the Git that established its source",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::sources::every_source_identity_names_the_git_that_established_it",
        },
    },
    Mutation {
        id: 1492,
        file: "xtask/src/qualification/record.rs",
        find: "    validate_record(record, kind)?;\n    crate::git::resolved()?.require_unchanged()?;\n",
        replace: "    validate_record(record, kind)?;\n",
        breaks: "a qualification record commits without re-measuring Git",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::sources::every_source_identity_names_the_git_that_established_it",
        },
    },
    Mutation {
        id: 1493,
        file: "xtask/src/evidence.rs",
        find: "    crate::git::resolved()?.require_unchanged()?;\n    let manifest = root.join(MANIFEST_NAME);",
        replace: "    let manifest = root.join(MANIFEST_NAME);",
        breaks: "an evidence manifest publishes without re-measuring Git",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::sources::every_source_identity_names_the_git_that_established_it",
        },
    },
    Mutation {
        id: 1620,
        file: "xtask/src/qualification/vmware.rs",
        find: "    Ok(digest_bytes(generated.as_bytes()))",
        replace: "    Ok(digest_bytes(&runtime))",
        breaks: "runtime metadata replaces the source-generated input identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::one_immutable_vmware_generation_is_captured_before_publication",
        },
    },
    Mutation {
        id: 1621,
        file: "xtask/src/qualification/vmware.rs",
        find: "    crate::vmware::verify_runtime_vmx(generated.as_bytes(), &runtime)?;",
        replace: "    let _ = (generated.as_bytes(), &runtime);",
        breaks: "a captured VMX from a different generation is accepted as source-equivalent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::one_immutable_vmware_generation_is_captured_before_publication",
        },
    },
    Mutation {
        id: 1744,
        file: "xtask/src/qualification/verify.rs",
        find: "    if !campaigns.deferred(Kind::CodexSecurityDeep) {\n        require_authenticated_deep_qualification(paths, root, source)?;\n    }\n    verify_records_after_authority(",
        replace: "    if !campaigns.deferred(Kind::CodexSecurityDeep) {\n        let _unchecked_deep_authority = (paths, root, source);\n    }\n    verify_records_after_authority(",
        breaks: "offline release-bundle replay bypasses the owned Deep authority verifier",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::online_and_offline_release_paths_share_the_owned_deep_verifier",
        },
    },
    Mutation {
        id: 1787,
        file: "xtask/src/evidence.rs",
        find: "    crate::mutate::workspace::with_clean_source_copy(paths, build)",
        replace: "    build(paths.root())",
        breaks: "one release reproduction reuses the checkout that invoked evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::release_reproduction_uses_copied_sources_and_rejects_host_paths",
        },
    },
    Mutation {
        id: 1832,
        file: "xtask/src/qualification/vmware.rs",
        find: "        bytes == expected,\n        \"VMware DMA handoff authority differs from the source-defined contract\"",
        replace: "        bytes.len() == expected.len(),\n        \"VMware DMA handoff authority differs from the source-defined contract\"",
        breaks: "VMware qualification accepts a foreign same-length DMA handoff authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmware::vmware_accepts_only_the_source_defined_dma_handoff_authority",
        },
    },
    Mutation {
        id: 1904,
        file: "xtask/src/qualification/attempts.rs",
        find: "        _authority: crate::security_deep::DeepAuthority,",
        replace: "        _authority: impl std::borrow::Borrow<crate::security_deep::DeepAuthority>,",
        breaks: "Deep PASS publication can borrow rather than consume its one-shot capability",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::deep_and_mutants_attempts_are_separate_capability_types",
        },
    },
    Mutation {
        id: 1905,
        file: "xtask/src/qualification/attempts.rs",
        find: "        let derived = crate::security_deep::verify_sealed(&self.paths, &directory, &self.source)?;",
        replace: "        let derived = fields.to_vec();",
        breaks: "owned Deep publication trusts caller fields instead of replaying the copied artifacts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::owned_deep_replays_copied_artifacts_before_committing_pass",
        },
    },
    Mutation {
        id: 1906,
        file: "xtask/src/qualification/attempts.rs",
        find: "        let derived = crate::security_deep::verify_sealed(&self.paths, &directory, &self.source)?;\n        require_external_fields(&record, &derived)?;\n        commit_record(&self.paths, Kind::CodexSecurityDeep, &record)",
        replace: "        let derived = crate::security_deep::verify_sealed(&self.paths, &directory, &self.source)?;\n        let _unchecked_derived = &derived;\n        commit_record(&self.paths, Kind::CodexSecurityDeep, &record)",
        breaks: "owned Deep publication does not compare recorded authority with replayed artifacts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::owned_deep_replays_copied_artifacts_before_committing_pass",
        },
    },
    Mutation {
        id: 1907,
        file: "xtask/src/qualification/verify.rs",
        find: "    if !campaigns.deferred(Kind::CodexSecurityDeep) {\n        require_authenticated_deep_qualification(paths, &root, &source)?;\n    }\n    recorded_source_after_authority(paths, &campaigns)",
        replace: "    if !campaigns.deferred(Kind::CodexSecurityDeep) {\n        let _unchecked_deep_authority = (&root, &source);\n    }\n    recorded_source_after_authority(paths, &campaigns)",
        breaks: "online evidence construction bypasses the owned Deep authority verifier",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::online_and_offline_release_paths_share_the_owned_deep_verifier",
        },
    },
    Mutation {
        id: 1908,
        file: "xtask/src/qualification/verify.rs",
        find: "fn require_external_fields(\n    record: &Record,\n    expected: &[(String, String)],\n) -> Result<()> {\n    for (key, value) in expected {\n        ensure!(\n            record.get(key)? == value,",
        replace: "fn require_external_fields(\n    record: &Record,\n    expected: &[(String, String)],\n) -> Result<()> {\n    for (key, value) in expected {\n        ensure!(\n            record.fields.contains_key(key) && !value.is_empty(),",
        breaks: "a forged owned Deep authority value passes when its field name still exists",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::every_forged_owned_deep_authority_field_is_rejected",
        },
    },
    Mutation {
        id: 1909,
        file: "xtask/src/qualification/record.rs",
        find: "        lines.first().copied() == Some(FORMAT),",
        replace: "        lines\n            .first()\n            .is_some_and(|line| line.starts_with(\"wizard-qualification-v\")),",
        breaks: "a superseded qualification schema is accepted by its version prefix",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_v5_is_rejected_even_with_its_own_valid_checksum",
        },
    },
    Mutation {
        id: 1910,
        file: "xtask/src/evidence.rs",
        find: "        lines.next() == Some(FORMAT),",
        replace: "        lines.next().is_some_and(|line| line.starts_with(\"wizard-evidence-v\")),",
        breaks: "a superseded release-evidence schema is accepted by its version prefix",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::evidence_v9_is_rejected_even_when_its_fields_are_canonical",
        },
    },
    Mutation {
        id: 2005,
        file: "xtask/src/qualification/record.rs",
        find: "                \"tool.codex.runtime.root\".to_string(),\n                \"tool.codex.runtime.sha256\".to_string(),\n                \"tool.python.runtime.bytes\".to_string(),",
        replace: "                \"tool.codex.runtime.root\".to_string(),\n                \"tool.python.runtime.bytes\".to_string(),",
        breaks: "qualification can admit owned Deep evidence without the Codex runtime digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2006,
        file: "xtask/src/git.rs",
        find: "                \"-c\",\n                \"core.fsmonitor=\",",
        replace: "",
        breaks: "a repository-local fsmonitor helper can execute during a source query",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::a_local_fsmonitor_command_cannot_run_during_a_hardened_query",
        },
    },
    Mutation {
        id: 2007,
        file: "xtask/src/git.rs",
        find: "        match self.filter_drivers(&root) {\n            Ok(drivers) => self.command_with_filters_disabled(&root, &drivers),\n            Err(_) => self.refused_command(),\n        }",
        replace: "        self.base_command(&root)",
        breaks: "repository content filters are neither discovered nor neutralized",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::repository_content_filters_are_discovered_and_neutralized",
        },
    },
    Mutation {
        id: 2008,
        file: "xtask/src/git.rs",
        find: "            \"--includes\",",
        replace: "            \"--no-includes\",",
        breaks: "content filters reached through repository includes escape discovery",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::repository_content_filters_are_discovered_and_neutralized",
        },
    },
    Mutation {
        id: 2009,
        file: "xtask/src/git.rs",
        find: "                format!(\"filter.{driver}.clean=\"),\n                format!(\"filter.{driver}.smudge=\"),\n                format!(\"filter.{driver}.process=\"),\n                format!(\"filter.{driver}.required=false\"),",
        replace: "                format!(\"filter.{driver}.smudge=\"),\n                format!(\"filter.{driver}.required=false\"),",
        breaks: "a discovered clean filter remains executable during a source query",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::repository_content_filters_are_discovered_and_neutralized",
        },
    },
    Mutation {
        id: 2010,
        file: "xtask/src/git.rs",
        find: "                format!(\"filter.{driver}.process=\"),",
        replace: "",
        breaks: "a discovered long-running process filter remains executable during a source query",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::repository_content_filters_are_discovered_and_neutralized",
        },
    },
    Mutation {
        id: 2011,
        file: "xtask/src/git.rs",
        find: "            Err(_) => self.refused_command(),",
        replace: "            Err(_) => self.base_command(&root),",
        breaks: "malformed repository configuration falls through to an executable source query",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::malformed_repository_configuration_fails_closed_before_the_query",
        },
    },
    Mutation {
        id: 2012,
        file: "xtask/src/git.rs",
        find: "            .current_dir(self.path.parent().unwrap_or(&self.path))\n            .args([\n                \"--no-pager\",\n                \"--literal-pathspecs\",",
        replace: "            .current_dir(root)\n            .args([\n                \"--no-pager\",\n                \"--literal-pathspecs\",",
        breaks: "a source query runs with attacker-controlled checkout files as its process directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::a_run_spawns_the_git_it_resolved_whatever_path_later_says",
        },
    },
    Mutation {
        id: 2013,
        file: "xtask/src/git.rs",
        find: "            .arg(git_dir)\n            .arg(work_tree)",
        replace: "            .arg(\"--git-dir=.git\")\n            .arg(\"--work-tree=.\")",
        breaks: "a source query binds its repository only relative to the process directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::repository_queries_bind_the_checkout_and_disable_executable_helpers",
        },
    },
    Mutation {
        id: 2020,
        file: "xtask/src/git.rs",
        find: "            .args([\n                \"--no-pager\",\n                \"--literal-pathspecs\",",
        replace: "            .args([\n                \"--literal-pathspecs\",",
        breaks: "a repository query can invoke Git without disabling its pager",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::repository_queries_bind_the_checkout_and_disable_executable_helpers",
        },
    },
    Mutation {
        id: 2021,
        file: "xtask/src/git.rs",
        find: "                \"--no-pager\",\n                \"--literal-pathspecs\",\n                \"--no-replace-objects\",",
        replace: "                \"--no-pager\",\n                \"--no-replace-objects\",",
        breaks: "repository queries can reinterpret exact source names as Git pathspec syntax",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::repository_queries_bind_the_checkout_and_disable_executable_helpers",
        },
    },
    Mutation {
        id: 2022,
        file: "xtask/src/git.rs",
        find: "                \"--literal-pathspecs\",\n                \"--no-replace-objects\",\n                \"--no-optional-locks\",",
        replace: "                \"--literal-pathspecs\",\n                \"--no-optional-locks\",",
        breaks: "repository queries can honor attacker-controlled replacement objects",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::repository_queries_bind_the_checkout_and_disable_executable_helpers",
        },
    },
    Mutation {
        id: 2023,
        file: "xtask/src/git.rs",
        find: "                \"--no-replace-objects\",\n                \"--no-optional-locks\",\n            ])",
        replace: "                \"--no-replace-objects\",\n            ])",
        breaks: "read-only repository queries can acquire optional repository locks",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::repository_queries_bind_the_checkout_and_disable_executable_helpers",
        },
    },
    Mutation {
        id: 2024,
        file: "xtask/src/git.rs",
        find: "                \"core.untrackedCache=false\",\n                \"-c\",\n                \"core.pager=\",",
        replace: "                \"core.untrackedCache=false\",",
        breaks: "repository configuration can restore a pager after the command-line pager switch",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::repository_queries_bind_the_checkout_and_disable_executable_helpers",
        },
    },
    Mutation {
        id: 2025,
        file: "xtask/src/git.rs",
        find: "                \"diff.external=\",\n                \"-c\",\n                \"diff.trustExitCode=false\",",
        replace: "                \"diff.external=\",",
        breaks: "repository configuration can make an external diff helper's exit status authoritative",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::repository_queries_bind_the_checkout_and_disable_executable_helpers",
        },
    },
    Mutation {
        id: 2026,
        file: "xtask/src/git.rs",
        find: "        bytes.is_empty() || bytes.last() == Some(&0),\n        \"Git filter configuration is not NUL terminated\"",
        replace: "        true,\n        \"Git filter configuration is not NUL terminated\"",
        breaks: "an unterminated Git filter configuration record can enter parser authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::filter_driver_inventory_is_strict_and_bounded",
        },
    },
    Mutation {
        id: 2027,
        file: "xtask/src/git.rs",
        find: "            !driver.chars().any(char::is_control) && !driver.contains('='),\n            \"Git returned an unsafe filter driver name\"",
        replace: "            true,\n            \"Git returned an unsafe filter driver name\"",
        breaks: "a Git filter driver name can inject command-scope configuration syntax",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::filter_driver_inventory_is_strict_and_bounded",
        },
    },
    Mutation {
        id: 2028,
        file: "xtask/src/git.rs",
        find: "                drivers.len() <= MAX_FILTER_DRIVERS,\n                \"repository defines more than {MAX_FILTER_DRIVERS} filter drivers\"",
        replace: "                true,\n                \"repository defines more than {MAX_FILTER_DRIVERS} filter drivers\"",
        breaks: "repository filter discovery can allocate overrides for more than its driver-count budget",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::filter_driver_inventory_is_strict_and_bounded",
        },
    },
    Mutation {
        id: 2029,
        file: "xtask/src/git.rs",
        find: "                override_bytes <= MAX_FILTER_OVERRIDE_BYTES,\n                \"repository filter overrides exceed {MAX_FILTER_OVERRIDE_BYTES} bytes\"",
        replace: "                true,\n                \"repository filter overrides exceed {MAX_FILTER_OVERRIDE_BYTES} bytes\"",
        breaks: "repository filter discovery can allocate command overrides beyond its byte budget",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "git::tests::filter_driver_inventory_is_strict_and_bounded",
        },
    },
    Mutation {
        id: 2030,
        file: "xtask/src/qualification/record.rs",
        find: "                \"tool.codex.runtime.bytes\".to_string(),\n                \"tool.codex.runtime.files\".to_string(),",
        replace: "                \"tool.codex.runtime.files\".to_string(),",
        breaks: "qualification can admit owned Deep evidence without the Codex runtime byte count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2031,
        file: "xtask/src/qualification/record.rs",
        find: "                \"tool.codex.runtime.bytes\".to_string(),\n                \"tool.codex.runtime.files\".to_string(),\n                \"tool.codex.runtime.root\".to_string(),",
        replace: "                \"tool.codex.runtime.bytes\".to_string(),\n                \"tool.codex.runtime.root\".to_string(),",
        breaks: "qualification can admit owned Deep evidence without the Codex runtime file count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2032,
        file: "xtask/src/qualification/record.rs",
        find: "                \"tool.codex.runtime.files\".to_string(),\n                \"tool.codex.runtime.root\".to_string(),\n                \"tool.codex.runtime.sha256\".to_string(),",
        replace: "                \"tool.codex.runtime.files\".to_string(),\n                \"tool.codex.runtime.sha256\".to_string(),",
        breaks: "qualification can admit owned Deep evidence without the Codex runtime root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2182,
        file: "xtask/src/qualification/record.rs",
        find: "                \"execution.worker-permission\".to_string(),\n                \"execution.worker-effective-profile.sha256\".to_string(),\n                \"execution.worker-effective-state.sha256\".to_string(),\n                \"execution.worker-parent-state.sha256\".to_string(),",
        replace: "                \"execution.worker-permission\".to_string(),\n                \"execution.worker-effective-state.sha256\".to_string(),\n                \"execution.worker-parent-state.sha256\".to_string(),",
        breaks: "qualification can admit owned Deep evidence without the effective-profile digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2183,
        file: "xtask/src/qualification/record.rs",
        find: "                \"execution.worker-effective-state.sha256\".to_string(),\n                \"execution.worker-parent-state.sha256\".to_string(),\n                \"execution.worker-provider-profile.sha256\".to_string(),",
        replace: "                \"execution.worker-effective-state.sha256\".to_string(),\n                \"execution.worker-provider-profile.sha256\".to_string(),",
        breaks: "qualification can admit owned Deep evidence without the parent-state digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2184,
        file: "xtask/src/qualification/record.rs",
        find: "                \"execution.worker-parent-state.sha256\".to_string(),\n                \"execution.worker-provider-profile.sha256\".to_string(),\n                \"execution.worker-preflight.allowed-reads\".to_string(),",
        replace: "                \"execution.worker-parent-state.sha256\".to_string(),\n                \"execution.worker-preflight.allowed-reads\".to_string(),",
        breaks: "qualification can admit owned Deep evidence without the provider-profile digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2195,
        file: "xtask/src/qualification/attempts.rs",
        find: "            (\"artifact.report\", \"report.md\", artifacts.report.as_slice()),\n            (\n                \"artifact.worker-transitions\",\n                \"worker-transitions\",\n                artifacts.worker_transitions.as_slice(),\n            ),",
        replace: "            (\"artifact.report\", \"report.md\", artifacts.report.as_slice()),",
        breaks: "owned Deep publication omits the replayable worker-transition artifact",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::owned_deep_replays_copied_artifacts_before_committing_pass",
        },
    },
    Mutation {
        id: 2196,
        file: "xtask/src/qualification.rs",
        find: "    BundleFile {\n        prefix: \"qualification.codex-security-deep.worker-transitions\",\n        relative: \"qualification/codex-security-deep/worker-transitions\",\n    },",
        replace: "",
        breaks: "offline evidence omits the owned Deep worker-transition artifact",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::qualification_records_cannot_be_omitted_or_changed_offline",
        },
    },
    Mutation {
        id: 2199,
        file: "xtask/src/qualification/record.rs",
        find: "                \"execution.worker-transitions.exec-initial\".to_string(),\n                \"execution.worker-transitions.exec-resumed\".to_string(),\n                \"findings.critical\".to_string(),",
        replace: "                \"execution.worker-transitions.exec-initial\".to_string(),\n                \"findings.critical\".to_string(),",
        breaks: "qualification can admit owned Deep evidence without the resumed-transition count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2200,
        file: "xtask/src/qualification/verify.rs",
        find: "            (\"artifact.report\", \"report.md\"),\n            (\"artifact.worker-transitions\", \"worker-transitions\"),",
        replace: "            (\"artifact.report\", \"report.md\"),",
        breaks: "qualification support verification does not replay the transition artifact's path and digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::owned_deep_replays_copied_artifacts_before_committing_pass",
        },
    },
    Mutation {
        id: 2218,
        file: "xtask/src/qualification/record.rs",
        find: "                \"execution.worker-effective-profile.sha256\".to_string(),\n                \"execution.worker-effective-state.sha256\".to_string(),\n                \"execution.worker-parent-state.sha256\".to_string(),",
        replace: "                \"execution.worker-effective-profile.sha256\".to_string(),\n                \"execution.worker-parent-state.sha256\".to_string(),",
        breaks: "qualification can omit the raw effective sandbox-state digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2219,
        file: "xtask/src/qualification/record.rs",
        find: "                \"execution.worker-preflight.allowed-reads\".to_string(),\n                \"execution.worker-preflight.denied-reads\".to_string(),",
        replace: "                \"execution.worker-preflight.denied-reads\".to_string(),",
        breaks: "qualification can omit the allowed-read preflight count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2220,
        file: "xtask/src/qualification/record.rs",
        find: "                \"execution.worker-preflight.denied-write-creates\".to_string(),\n                \"execution.worker-preflight.denied-write-mutations\".to_string(),",
        replace: "                \"execution.worker-preflight.denied-write-mutations\".to_string(),",
        breaks: "qualification can omit the denied-create preflight count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2221,
        file: "xtask/src/qualification/record.rs",
        find: "                \"execution.worker-preflight.denied-write-mutations\".to_string(),\n                \"execution.worker-preflight.denied-network-listeners\".to_string(),",
        replace: "                \"execution.worker-preflight.denied-network-listeners\".to_string(),",
        breaks: "qualification can omit the denied existing-file mutation count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2222,
        file: "xtask/src/qualification/record.rs",
        find: "                \"execution.worker-preflight.denied-network-listeners\".to_string(),\n                \"execution.worker-preflight.denied-network-connections\".to_string(),",
        replace: "                \"execution.worker-preflight.denied-network-connections\".to_string(),",
        breaks: "qualification can omit the denied-listener preflight count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2223,
        file: "xtask/src/qualification/record.rs",
        find: "                \"execution.worker-preflight.denied-network-connections\".to_string(),\n                \"execution.worker-preflight.denied-descendants\".to_string(),",
        replace: "                \"execution.worker-preflight.denied-descendants\".to_string(),",
        breaks: "qualification can omit the denied outbound-connect preflight count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::qualification_schema_rejects_omission_and_invention",
        },
    },
    Mutation {
        id: 2262,
        file: "xtask/src/qualification/verify.rs",
        find: "        if campaigns.deferred(kind) {\n            require_deferred(root, kind)?;\n        }",
        replace: "        if campaigns.deferred(kind) {\n            let _unchecked_deferral = (root, kind);\n        }",
        breaks: "a deferred continuous campaign can carry a stale qualification record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_deferred_campaign_carrying_a_record_is_refused",
        },
    },
    Mutation {
        id: 2263,
        file: "xtask/src/qualification.rs",
        find: "        !record_present(root, kind)?,\n        \"deferred {} campaign carries a qualification record\"",
        replace: "        record_present(root, kind).is_ok(),\n        \"deferred {} campaign carries a qualification record\"",
        breaks: "a deferral check accepts the record it observed",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_deferred_campaign_carrying_a_record_is_refused",
        },
    },
    Mutation {
        id: 2264,
        file: "xtask/src/qualification.rs",
        find: "                *state = CampaignState::Pass;\n",
        replace: "                let _unobserved_record = &state;\n",
        breaks: "campaign observation reports a present record as deferred",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::a_deferred_campaign_carrying_a_record_is_refused",
        },
    },
    Mutation {
        id: 2265,
        file: "xtask/src/qualification.rs",
        find: "                    self.deferred(*kind)\n                        && spec\n",
        replace: "                    spec\n",
        breaks: "a passing campaign's files leave the release bundle",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::only_a_deferred_campaigns_files_leave_the_bundle",
        },
    },
    Mutation {
        id: 2266,
        file: "xtask/src/qualification.rs",
        find: "            .any(|(candidate, state)| *candidate == kind && state == CampaignState::Deferred)",
        replace: "            .any(|(candidate, _state)| *candidate == kind)",
        breaks: "a passing continuous campaign is treated as deferred",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::only_the_grade_and_deep_campaigns_are_deferrable",
        },
    },
    Mutation {
        id: 2267,
        file: "xtask/src/qualification.rs",
        find: "const DEFERRABLE: [Kind; 2] = [Kind::CargoMutantsGrade, Kind::CodexSecurityDeep];",
        replace: "const DEFERRABLE: [Kind; 2] = [Kind::CargoMutantsGrade, Kind::Model];",
        breaks: "a required release qualification becomes deferrable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::only_the_grade_and_deep_campaigns_are_deferrable",
        },
    },
    Mutation {
        id: 2268,
        file: "xtask/src/qualification.rs",
        find: "                DEFERRED => CampaignState::Deferred,\n                other => bail!(\n                    \"{} campaign status `{other}` is neither pass nor deferred\",\n                    kind.slug()\n                ),\n",
        replace: "                _ => CampaignState::Deferred,\n",
        breaks: "a campaign status other than pass or deferred is read as deferred",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::campaign_status_is_exactly_pass_or_deferred",
        },
    },
    Mutation {
        id: 2269,
        file: "xtask/src/evidence.rs",
        find: "[record.get(&grade)?, record.get(&deep)?]",
        replace: "[record.get(&grade).unwrap_or(\"deferred\"), record.get(&deep).unwrap_or(\"deferred\")]",
        breaks: "an omitted manifest campaign status defaults to deferred",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::an_omitted_campaign_status_is_refused_not_defaulted",
        },
    },
    Mutation {
        id: 2270,
        file: "xtask/src/qualification/verify.rs",
        find: "    if !campaigns.deferred(Kind::CodexSecurityDeep) {\n        require_authenticated_deep_qualification(paths, &root, &source)?;",
        replace: "    if campaigns.deferred(Kind::CodexSecurityDeep) {\n        require_authenticated_deep_qualification(paths, &root, &source)?;",
        breaks: "online evidence construction skips the owned Deep verifier for a passing Deep record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::online_and_offline_release_paths_share_the_owned_deep_verifier",
        },
    },
    Mutation {
        id: 2271,
        file: "xtask/src/qualification/verify.rs",
        find: "    if !campaigns.deferred(Kind::CodexSecurityDeep) {\n        require_authenticated_deep_qualification(paths, root, source)?;",
        replace: "    if campaigns.deferred(Kind::CodexSecurityDeep) {\n        require_authenticated_deep_qualification(paths, root, source)?;",
        breaks: "offline release-bundle replay skips the owned Deep verifier for a passing Deep record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::online_and_offline_release_paths_share_the_owned_deep_verifier",
        },
    },
];
