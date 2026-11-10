use std::time::Instant;

use super::*;
use crate::test_support::{create_directory_symlink, create_file_symlink, success_status};

mod advisory;
mod size_policy;
mod vmware;

const PRODUCTION: &str = concat!(
    include_str!("attempts.rs"),
    include_str!("advisory.rs"),
    include_str!("vmware.rs"),
    include_str!("verify.rs"),
    include_str!("record.rs"),
    include_str!("size_policy.rs"),
    include_str!("../qualification.rs"),
);

fn fixture_git(record: &mut Record) {
    let path = std::env::current_dir().unwrap().join("git");
    for (key, value) in [
        ("tool.git.bytes", "1".to_string()),
        ("tool.git.path", path.to_string_lossy().into_owned()),
        ("tool.git.sha256", "f".repeat(64)),
        ("tool.git.version", "git version 2.44.0".to_string()),
    ] {
        record.insert(key, value).unwrap();
    }
}

fn complete_record(kind: Kind) -> Record {
    let mut record = Record::default();
    record.insert("kind", kind.slug()).unwrap();
    record.insert("result", PASS).unwrap();
    record.insert("source.commit", "a".repeat(40)).unwrap();
    record.insert("source.tree", "b".repeat(40)).unwrap();
    fixture_git(&mut record);
    match kind {
        Kind::Advisory => {
            fixture_tool(&mut record, "input.cargo-audit");
            record.fields.insert(
                "input.cargo-audit.version".to_string(),
                ADVISORY_TOOL_VERSION.to_string(),
            );
            record.insert("input.cargo-lock.bytes", "1").unwrap();
            record
                .insert("input.cargo-lock.sha256", "c".repeat(64))
                .unwrap();
            record.insert("database.revision", "d".repeat(40)).unwrap();
            record.insert("database.tree", "e".repeat(40)).unwrap();
            record
                .insert("database.last-updated", "2026-08-30T12:00:00Z")
                .unwrap();
            record
                .insert("database.url", ADVISORY_DATABASE_URL)
                .unwrap();
            record
                .insert("execution.command", ADVISORY_COMMAND)
                .unwrap();
            record.insert("execution.exit-code", "0").unwrap();
            record
                .insert("execution.refresh-command", ADVISORY_REFRESH_COMMAND)
                .unwrap();
            record.insert("execution.refresh-exit-code", "0").unwrap();
            record
                .insert("observation.utc-unix-seconds", "1788048000")
                .unwrap();
            fixture_artifact(&mut record, "artifact.policy", ADVISORY_POLICY_PATH);
            fixture_artifact(&mut record, "artifact.transcript", ADVISORY_TRANSCRIPT);
            fixture_artifact(
                &mut record,
                "artifact.refresh-transcript",
                ADVISORY_REFRESH_TRANSCRIPT,
            );
        }
        Kind::Mutation => {
            record.insert("input.mutations", "1").unwrap();
            record
                .insert("input.registry.sha256", "c".repeat(64))
                .unwrap();
            record
                .insert("input.source.sha256", "d".repeat(64))
                .unwrap();
            record.insert("input.index.sha256", "e".repeat(64)).unwrap();
            fixture_artifact(&mut record, "artifact.report", MUTATION_REPORT);
        }
        Kind::CargoMutantsGrade => {
            for (key, value) in [
                ("execution.authority", "external-operator-v1"),
                ("grade.adjudicated", "0"),
                ("grade.caught", "1"),
                ("grade.missed", "0"),
                ("grade.timeout", "0"),
                ("grade.total", "1"),
                ("grade.unviable", "0"),
                ("tool.cargo-mutants.bytes", "1"),
                (
                    "tool.cargo-mutants.sha256",
                    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                ),
                ("tool.cargo-mutants.version", "27.1.0"),
                (
                    "tool.cargo-nextest.archive.sha256",
                    "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                ),
                ("tool.cargo-nextest.bytes", "1"),
                ("tool.cargo-nextest.commit", "e"),
                (
                    "tool.cargo-nextest.sha256",
                    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                ),
                ("tool.cargo-nextest.url", "https://example.invalid"),
                ("tool.cargo-nextest.version", "0.9.145"),
            ] {
                record.insert(key, value).unwrap();
            }
            for (prefix, name) in [
                ("artifact.request", "request"),
                ("artifact.inventory", "inventory.json"),
                ("artifact.mutants", "mutants.json"),
                ("artifact.outcomes", "outcomes.json"),
                ("artifact.adjudications", "adjudications.json"),
                ("artifact.doctest", "doctest.transcript"),
            ] {
                fixture_artifact(&mut record, prefix, name);
            }
        }
        Kind::CodexSecurityDeep => {
            for (key, value) in [
                ("authentication.channel", "codex-home-chatgpt"),
                (
                    "environment.policy",
                    "xtask-owned-codex-security-hermetic-v9",
                ),
                ("execution.authority", "xtask-mcp-wrapper-windows-job-v8"),
                ("execution.complete-tool", "complete_codex_security_scan"),
                ("execution.model", "gpt-6.1-sol"),
                ("execution.protocol", "2025-06-18"),
                ("execution.reasoning", "xhigh"),
                ("execution.server-name", "codex-security"),
                ("execution.server-version", "0.1.160"),
                ("execution.start-tool", "start_codex_security_deep_scan"),
                ("execution.tools.bytes", "79316"),
                ("execution.tools.count", "47"),
                (
                    "execution.tools.sha256",
                    "514a38fefaaa3f89b1b897755721eca31b0fef5589c506102a696163e4add29a",
                ),
                (
                    "execution.worker-permission",
                    "provider-envelope=managed-root-read+explicit-deny;effective-worker=exec-only:backend=mxc,root=deny,minimal+owned-repository+scan+plugin+qualified-runtime=read,repository-metadata+credential=deny,outside-canary=default-deny-probed,network=false",
                ),
                (
                    "execution.worker-effective-profile.sha256",
                    "3333333333333333333333333333333333333333333333333333333333333333",
                ),
                (
                    "execution.worker-effective-state.sha256",
                    "4444444444444444444444444444444444444444444444444444444444444444",
                ),
                (
                    "execution.worker-parent-state.sha256",
                    "1111111111111111111111111111111111111111111111111111111111111111",
                ),
                (
                    "execution.worker-provider-profile.sha256",
                    "2222222222222222222222222222222222222222222222222222222222222222",
                ),
                ("execution.worker-preflight.allowed-reads", "7"),
                ("execution.worker-preflight.denied-reads", "4"),
                ("execution.worker-preflight.denied-write-creates", "2"),
                ("execution.worker-preflight.denied-write-mutations", "3"),
                ("execution.worker-preflight.denied-network-listeners", "1"),
                ("execution.worker-preflight.denied-network-connections", "1"),
                ("execution.worker-preflight.denied-descendants", "1"),
                ("execution.worker-transitions.app-server", "1"),
                ("execution.worker-transitions.exec-initial", "2"),
                ("execution.worker-transitions.exec-resumed", "3"),
                ("findings.critical", "0"),
                ("findings.high", "0"),
                ("findings.informational", "0"),
                ("findings.low", "0"),
                ("findings.medium", "0"),
                ("findings.total", "0"),
                ("scan.id", "scan-fixture"),
                ("scan.producer.name", "codex-security-plugin"),
                ("scan.producer.version", "0.1.32"),
                ("scan.surfaces", "0"),
                ("tool.codex-security.integrity.bytes", "17465"),
                (
                    "tool.codex-security.integrity.sha256",
                    "6b342df66fb9b5ce7fc8c9de0e974b59d21340cdc06167df46d1a19bd4ea9441",
                ),
                ("tool.codex-security.tree.files", "142"),
                ("tool.codex-security.version", "0.1.32"),
                ("tool.codex.runtime.bytes", "442152128"),
                ("tool.codex.runtime.files", "4"),
                ("tool.codex.runtime.root", "parent-of-tool.codex"),
                (
                    "tool.codex.runtime.sha256",
                    "72dd0efdd65e34488bc013f88166a8f549cf1f7441d0d23ad7116bc5758de1d5",
                ),
                ("tool.python.runtime.bytes", "1"),
                ("tool.python.runtime.files", "1"),
                ("tool.python.runtime.root", "parent-of-tool.python"),
                (
                    "tool.python.runtime.sha256",
                    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                ),
            ] {
                record.insert(key, value).unwrap();
            }
            for prefix in ["tool.node", "tool.python", "tool.codex"] {
                fixture_tool(&mut record, prefix);
            }
            for (prefix, name) in [
                ("artifact.request", "request"),
                ("artifact.manifest", "scan-manifest.json"),
                ("artifact.findings", "findings.json"),
                ("artifact.coverage", "coverage.json"),
                ("artifact.report", "report.md"),
                ("artifact.worker-transitions", "worker-transitions"),
            ] {
                fixture_artifact(&mut record, prefix, name);
            }
        }
        Kind::Model => {
            fixture_tool(&mut record, "input.java");
            record.insert("input.models.bytes", "1").unwrap();
            record.insert("input.models.files", "1").unwrap();
            record
                .insert("input.models.sha256", "d".repeat(64))
                .unwrap();
            record.insert("input.tlc-jar.bytes", "1").unwrap();
            record
                .insert("input.tlc-jar.sha256", "e".repeat(64))
                .unwrap();
            for artifact in crate::model::TRANSCRIPT_ARTIFACTS {
                fixture_artifact(&mut record, artifact.prefix, artifact.name);
            }
        }
        Kind::VmwareSelfTest | Kind::VmwareShipped => {
            fixture_tool(&mut record, "input.vmrun");
            record
                .insert("execution.vmrun.command", "-T ws list")
                .unwrap();
            record.insert("execution.vmrun.running", "1").unwrap();
            for (prefix, name) in [
                ("artifact.generation", VMWARE_GENERATION_RECEIPT),
                ("artifact.image", VMWARE_IMAGE),
                ("artifact.namespace", VMWARE_NAMESPACE),
                ("artifact.vmdk", VMWARE_VMDK),
                ("artifact.namespace-vmdk", VMWARE_NAMESPACE_VMDK),
                ("artifact.vmx", VMWARE_VMX),
                ("artifact.handoff", VMWARE_HANDOFF),
                ("artifact.transcript", VMWARE_TRANSCRIPT),
                ("artifact.observer", VMWARE_OBSERVER),
            ] {
                fixture_artifact(&mut record, prefix, name);
            }
        }
    }
    record
}

#[test]
#[ignore]
fn slow_external_probe_child() {
    std::thread::sleep(Duration::from_secs(2));
}

#[test]
fn an_external_qualification_probe_has_a_direct_child_return_deadline() {
    let executable = std::env::current_exe().expect("current test executable");
    let arguments = [
        "--exact",
        "qualification::tests::slow_external_probe_child",
        "--ignored",
    ];
    let started = Instant::now();
    let error = output_with_timeout(
        &std::env::current_dir().unwrap(),
        &executable,
        &arguments,
        ExitPolicy::Any,
        Duration::from_millis(100),
    )
    .unwrap_err();

    assert!(format!("{error:#}").contains("timed out after 100 ms"));
    assert!(started.elapsed() < Duration::from_secs(1));
}

fn fixture_tool(record: &mut Record, prefix: &str) {
    record.insert(format!("{prefix}.bytes"), "1").unwrap();
    record
        .insert(format!("{prefix}.sha256"), "f".repeat(64))
        .unwrap();
    record
        .insert(format!("{prefix}.version"), "fixture 1")
        .unwrap();
}

fn fixture_artifact(record: &mut Record, prefix: &str, name: &str) {
    record.insert(format!("{prefix}.bytes"), "1").unwrap();
    record.insert(format!("{prefix}.path"), name).unwrap();
    record
        .insert(format!("{prefix}.sha256"), "0".repeat(64))
        .unwrap();
}

fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("running fixture Git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_snapshot_bundle(root: &Path) {
    for spec in BUNDLE_FILES {
        let path = root.join(bundle_tail(spec).unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = File::create(path).unwrap();
        file.write_all(spec.prefix.as_bytes()).unwrap();
        let (maximum, exact) = bundle_file_size_policy(spec).unwrap();
        if exact {
            file.set_len(maximum).unwrap();
        }
    }
}

#[test]
fn a_qualification_record_is_canonical_and_checksummed() {
    for kind in [
        Kind::Advisory,
        Kind::Mutation,
        Kind::CargoMutantsGrade,
        Kind::CodexSecurityDeep,
        Kind::Model,
        Kind::VmwareSelfTest,
        Kind::VmwareShipped,
    ] {
        let record = complete_record(kind);
        validate_record(&record, kind).unwrap();
        let rendered = record.render();
        assert_eq!(parse_record(&rendered).unwrap(), record);

        let tampered = rendered.replace("result=pass", "result=fail");
        assert!(parse_record(&tampered).is_err());
        assert!(parse_record(rendered.trim_end()).is_err());
    }
}

#[test]
fn online_and_offline_release_paths_share_the_owned_deep_verifier() {
    let implementation = PRODUCTION;
    let tests = implementation.find("\n#[cfg(test)]\nmod tests").unwrap();
    let production = &implementation[..tests];
    let online = implementation
        .find("pub(crate) fn recorded_source(paths: &Paths)")
        .unwrap();
    let offline = implementation.find("fn verify_records(").unwrap();
    let implementation_end = implementation
        .find("fn verify_advisory_qualification(")
        .unwrap();
    let guard = "    if !campaigns.deferred(Kind::CodexSecurityDeep) {\n        ";
    assert!(implementation[online..offline].contains(&format!(
        "{guard}require_authenticated_deep_qualification(paths, &root, &source)?;\n    }}\n"
    )));
    assert!(
        implementation[offline..implementation_end].contains(&format!(
            "{guard}require_authenticated_deep_qualification(paths, root, source)?;\n    }}\n"
        ))
    );
    assert_eq!(
        production
            .matches("require_authenticated_deep_qualification(")
            .count(),
        3
    );
    assert_eq!(
        production
            .matches("fn require_authenticated_deep_qualification(")
            .count(),
        1
    );
}

#[test]
fn a_deferred_campaign_carrying_a_record_is_refused() {
    let root = crate::test_support::reserve_temp_directory("wizard-qualification-deferred-record");
    let paths = Paths::new(root.clone(), None);
    let source = SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    let deferred = CampaignStates::from_labels([DEFERRED, DEFERRED]).unwrap();
    let grade = root.join(Kind::CargoMutantsGrade.slug());
    std::fs::create_dir_all(&grade).unwrap();
    assert_eq!(CampaignStates::observe(&root).unwrap(), deferred);
    require_deferred(&root, Kind::CargoMutantsGrade).unwrap();

    std::fs::write(grade.join(RECORD), "stale grade record\n").unwrap();
    assert_eq!(
        CampaignStates::observe(&root).unwrap(),
        CampaignStates::from_labels([PASS, DEFERRED]).unwrap()
    );
    let error =
        verify_records_after_authority(&paths, &root, &source, &"c".repeat(64), 1, None, &deferred)
            .unwrap_err()
            .to_string();
    assert!(
        error.contains("deferred cargo-mutants-grade campaign carries"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn only_a_deferred_campaigns_files_leave_the_bundle() {
    let relatives = |labels| {
        CampaignStates::from_labels(labels)
            .unwrap()
            .bundle_files()
            .iter()
            .map(|spec| spec.relative)
            .collect::<Vec<_>>()
    };
    let without = |slugs: &[&str]| {
        BUNDLE_FILES
            .iter()
            .map(|spec| spec.relative)
            .filter(|relative| {
                !slugs
                    .iter()
                    .any(|slug| relative.starts_with(&format!("qualification/{slug}/")))
            })
            .collect::<Vec<_>>()
    };
    let grade = "cargo-mutants-grade";
    let deep = "codex-security-deep";
    assert_eq!(relatives([PASS, PASS]), without(&[]));
    assert_eq!(relatives([PASS, PASS]).len(), BUNDLE_FILES.len());
    assert_eq!(relatives([DEFERRED, PASS]), without(&[grade]));
    assert_eq!(relatives([PASS, DEFERRED]), without(&[deep]));
    assert_eq!(relatives([DEFERRED, DEFERRED]), without(&[grade, deep]));
    assert_eq!(
        relatives([DEFERRED, DEFERRED]).len(),
        BUNDLE_FILES.len() - 14
    );
}

#[test]
fn only_the_grade_and_deep_campaigns_are_deferrable() {
    assert_eq!(
        CampaignStates::status_keys(),
        [
            "qualification.cargo-mutants-grade.status".to_string(),
            "qualification.codex-security-deep.status".to_string(),
        ]
    );
    let passed = CampaignStates::from_labels([PASS, PASS]).unwrap();
    let deferred = CampaignStates::from_labels([DEFERRED, DEFERRED]).unwrap();
    for kind in [Kind::CargoMutantsGrade, Kind::CodexSecurityDeep] {
        assert!(deferred.deferred(kind));
        assert!(!passed.deferred(kind));
    }
    for kind in [
        Kind::Mutation,
        Kind::Advisory,
        Kind::Model,
        Kind::VmwareSelfTest,
        Kind::VmwareShipped,
    ] {
        assert!(!deferred.deferred(kind));
    }
}

#[test]
fn campaign_status_is_exactly_pass_or_deferred() {
    for labels in [
        [PASS, PASS],
        [PASS, DEFERRED],
        [DEFERRED, PASS],
        [DEFERRED, DEFERRED],
    ] {
        assert_eq!(
            CampaignStates::from_labels(labels).unwrap().labels(),
            labels
        );
    }
    for invalid in ["", "fail", "Pass", "DEFERRED", "pass ", "skipped"] {
        assert!(
            CampaignStates::from_labels([invalid, PASS]).is_err(),
            "{invalid:?}"
        );
        assert!(
            CampaignStates::from_labels([DEFERRED, invalid]).is_err(),
            "{invalid:?}"
        );
    }
}

fn security_deep_children() -> Vec<String> {
    let mut pending = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src/security_deep")];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().unwrap() == "tests" || path.file_name().unwrap() == "tests.rs" {
                continue;
            }
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(std::fs::read_to_string(path).unwrap());
            }
        }
    }
    assert!(!sources.is_empty(), "security_deep children were not found");
    sources
}

#[test]
fn deep_and_mutants_attempts_are_separate_capability_types() {
    let implementation = PRODUCTION;
    let deep = implementation.find("impl DeepAttempt {").unwrap();
    let owned = implementation.find("impl OwnedDeepAttempt {").unwrap();
    let mutants = implementation.find("impl MutantsAttempt {").unwrap();
    let constants = implementation[mutants..]
        .find("pub(crate) const BUNDLE_FILES")
        .unwrap()
        + mutants;
    assert!(!implementation[deep..owned].contains("pass_observed_deep"));
    assert!(implementation[owned..mutants].contains("pass_observed_deep"));
    assert!(
        implementation[owned..mutants].contains("_authority: crate::security_deep::DeepAuthority")
    );
    assert!(!implementation[deep..mutants].contains("pass_observed_mutants"));
    assert!(implementation[mutants..constants].contains("pass_observed_mutants"));
    let begin_owned = implementation
        .find("pub(crate) fn begin_owned_security_deep(")
        .unwrap();
    let begin_mutants = implementation[begin_owned..]
        .find("pub(crate) fn begin_mutants_grade(")
        .unwrap()
        + begin_owned;
    assert!(
        implementation[begin_owned..begin_mutants]
            .contains("_authority: &crate::security_deep::DeepAuthority")
    );

    let security = include_str!("../security_deep.rs");
    let security_tests = security.find("#[cfg(test)]").unwrap();
    let children = security_deep_children()
        .iter()
        .map(|source| source.matches("DeepAuthority::new()").count())
        .sum::<usize>();
    assert_eq!(
        security[..security_tests]
            .matches("DeepAuthority::new()")
            .count()
            + children,
        1
    );
    let token = security
        .find("#[derive(Debug)]\npub(crate) struct DeepAuthority {")
        .unwrap();
    let token_end = security[token..]
        .find("\n#[derive(Clone, Copy, PartialEq, Eq, Debug)]")
        .unwrap()
        + token;
    let token_definition = &security[token..token_end];
    assert!(token_definition.contains("\n    _private: (),"));
    assert!(token_definition.contains("\n    fn new() -> Self"));
    assert!(!token_definition.contains("Clone"));

    let run = security.find("fn run_owned(paths: &Paths)").unwrap();
    let after_run = security[run..].find("\nfn begin(").unwrap() + run;
    let run_body = &security[run..after_run];
    assert_eq!(run_body.matches("DeepAuthority::new()").count(), 1);
    assert_eq!(
        run_body
            .matches("qualification::begin_owned_security_deep(paths, &authority)")
            .count(),
        1
    );
    assert_eq!(
        run_body
            .matches(".pass_observed_deep(authority, &fields, artifacts)")
            .count(),
        1
    );
}

#[test]
fn owned_deep_replays_copied_artifacts_before_committing_pass() {
    let implementation = PRODUCTION;
    let owned = implementation.find("impl OwnedDeepAttempt {").unwrap();
    let end = implementation[owned..]
        .find("impl MutantsAttempt {")
        .unwrap()
        + owned;
    let body = &implementation[owned..end];

    let copy = body.find("write_new_file(&path, bytes)?;").unwrap();
    let replay = body.find("crate::security_deep::verify_sealed(").unwrap();
    let compare = body
        .find("require_external_fields(&record, &derived)?;")
        .unwrap();
    let commit = body
        .find("commit_record(&self.paths, Kind::CodexSecurityDeep, &record)")
        .unwrap();
    assert!(copy < replay && replay < compare && compare < commit);
    assert_eq!(
        body.matches("crate::security_deep::verify_sealed(").count(),
        1
    );
    assert_eq!(
        body.matches("commit_record(&self.paths, Kind::CodexSecurityDeep, &record)")
            .count(),
        1
    );
    assert_eq!(body.matches("\"artifact.worker-transitions\"").count(), 1);
    let support = implementation.find("fn verify_support(").unwrap();
    let support_end = implementation[support..]
        .find("fn require_external_fields")
        .unwrap()
        + support;
    assert!(
        implementation[support..support_end]
            .contains("(\"artifact.worker-transitions\", \"worker-transitions\")")
    );
}

#[test]
fn owned_deep_cleans_scratch_before_minting_pass() {
    let security = include_str!("../security_deep.rs");
    let run = security.find("fn run_owned(paths: &Paths)").unwrap();
    let end = security[run..].find("\nfn reserve_scratch(").unwrap() + run;
    let body = &security[run..end];
    let transaction = body.find("finalize_owned_transaction(").unwrap();
    let cleanup = body.find("|| remove_scratch(&scratch)").unwrap();
    let pass = body
        .find("|| attempt.pass_observed_deep(authority, &fields, artifacts)")
        .unwrap();
    assert!(
        transaction < cleanup && cleanup < pass,
        "PASS preceded the fallible scratch-cleanup transaction"
    );
}

#[test]
fn qualification_schema_rejects_omission_and_invention() {
    let mut record = complete_record(Kind::Mutation);
    record.fields.remove("artifact.report.sha256");
    assert!(validate_record(&record, Kind::Mutation).is_err());

    let mut record = complete_record(Kind::Model);
    record.fields.remove("artifact.run.ring-clean.sha256");
    assert!(validate_record(&record, Kind::Model).is_err());

    let mut record = complete_record(Kind::Model);
    record.insert("result.claimed-by-prose", PASS).unwrap();
    assert!(validate_record(&record, Kind::Model).is_err());

    let mut record = complete_record(Kind::CodexSecurityDeep);
    record.fields.remove("tool.python.runtime.sha256");
    assert!(validate_record(&record, Kind::CodexSecurityDeep).is_err());

    for field in [
        "tool.codex.runtime.bytes",
        "tool.codex.runtime.files",
        "tool.codex.runtime.root",
        "tool.codex.runtime.sha256",
    ] {
        let mut record = complete_record(Kind::CodexSecurityDeep);
        record.fields.remove(field);
        assert!(
            validate_record(&record, Kind::CodexSecurityDeep).is_err(),
            "missing Codex runtime field {field} was accepted"
        );
    }

    let complete = complete_record(Kind::CodexSecurityDeep);
    validate_record(&complete, Kind::CodexSecurityDeep).unwrap();
    for field in [
        "execution.worker-effective-profile.sha256",
        "execution.worker-effective-state.sha256",
        "execution.worker-parent-state.sha256",
        "execution.worker-provider-profile.sha256",
        "execution.worker-preflight.allowed-reads",
        "execution.worker-preflight.denied-reads",
        "execution.worker-preflight.denied-write-creates",
        "execution.worker-preflight.denied-write-mutations",
        "execution.worker-preflight.denied-network-listeners",
        "execution.worker-preflight.denied-network-connections",
        "execution.worker-preflight.denied-descendants",
        "execution.worker-transitions.app-server",
        "execution.worker-transitions.exec-initial",
        "execution.worker-transitions.exec-resumed",
    ] {
        let mut record = complete.clone();
        record.fields.remove(field);
        assert!(
            validate_record(&record, Kind::CodexSecurityDeep).is_err(),
            "missing worker authority field {field} was accepted"
        );
    }

    let mut record = complete_record(Kind::CodexSecurityDeep);
    record
        .insert("execution.claimed-by-provider", PASS)
        .unwrap();
    assert!(validate_record(&record, Kind::CodexSecurityDeep).is_err());
}

#[test]
fn every_forged_owned_deep_authority_field_is_rejected() {
    let record = complete_record(Kind::CodexSecurityDeep);
    let expected: Vec<(String, String)> = record
        .fields
        .iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "authentication.channel" | "environment.policy"
            ) || key.starts_with("execution.")
                || key.starts_with("findings.")
                || key.starts_with("scan.")
                || key.starts_with("tool.codex-security.")
                || key.starts_with("tool.node.")
                || key.starts_with("tool.python.")
                || key.starts_with("tool.codex.")
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    assert_eq!(expected.len(), 59);
    require_external_fields(&record, &expected).unwrap();

    for (key, _) in &expected {
        let mut forged = record.clone();
        forged.fields.insert(key.clone(), "forged".to_string());
        assert!(
            require_external_fields(&forged, &expected).is_err(),
            "forged owned Deep field `{key}` was accepted"
        );
    }
}

#[test]
fn qualification_v5_is_rejected_even_with_its_own_valid_checksum() {
    let rendered = complete_record(Kind::Mutation).render();
    let fields_start = rendered.find('\n').unwrap() + 1;
    let checksum_start = rendered.rfind("checksum.sha256=").unwrap();
    let body = format!(
        "wizard-qualification-v5\n{}",
        &rendered[fields_start..checksum_start]
    );
    let old = format!(
        "{body}checksum.sha256={}\n",
        sha256::hex(&sha256::digest(body.as_bytes()))
    );

    let error = parse_record(&old).unwrap_err().to_string();
    assert!(error.contains("no supported header"), "{error}");
}

#[test]
fn an_artifact_swapped_for_a_link_after_its_check_is_never_read() {
    let root = crate::test_support::reserve_temp_directory("wizard-swapped-artifact");
    let path = root.join("report.md");
    let parked = root.join("parked");
    let secret = root.join("secret");
    std::fs::write(&path, b"producer report").unwrap();
    std::fs::write(&secret, b"operator secret").unwrap();
    let (link, target, original, restored) =
        (path.clone(), secret.clone(), path.clone(), parked.clone());
    let result = read_bounded_file_with(
        &path,
        64,
        "fixture report",
        move || {
            std::fs::rename(&link, &parked)?;
            create_file_symlink(&target, &link);
            Ok(())
        },
        move || {
            std::fs::remove_file(&original)?;
            std::fs::rename(&restored, &original)?;
            Ok(())
        },
    );
    let error = result
        .expect_err("a link swapped in after the path check was read")
        .to_string();
    let refusal = if cfg!(windows) {
        "is not a real file"
    } else {
        "was replaced while it was opened"
    };
    assert!(error.contains(refusal), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn git_queries_enforce_the_callers_semantic_capture_limit() {
    let executable = std::env::current_exe().unwrap();
    let error = git_output_bytes_with(
        &std::env::current_dir().unwrap(),
        &["status", "--porcelain=v1"],
        32,
        "fixture Git query",
        |command, timeout, label, limit| {
            assert_eq!(
                command.get_program(),
                crate::git::resolved()
                    .unwrap()
                    .command(Path::new("."))
                    .get_program()
            );
            assert_eq!(timeout, PROBE_TIMEOUT);
            assert_eq!(limit, 32);
            let mut noisy = Command::new(&executable);
            noisy.args([
                "--exact",
                "bounded_process::tests::noisy_capture_fixture_child",
                "--ignored",
                "--nocapture",
            ]);
            crate::bounded_process::output_with_limit(noisy, timeout, label, limit)
        },
    )
    .unwrap_err();

    assert!(format!("{error:#}").contains("exceeded 32 bytes"));
}

#[test]
fn stale_source_identity_is_rejected_field_by_field() {
    let record = complete_record(Kind::Mutation);
    let current = SourceIdentity {
        commit: "9".repeat(40),
        tree: "b".repeat(40),
    };
    let error = require_source(&record, &current).unwrap_err().to_string();
    assert!(error.contains("another commit"), "{error}");

    let current = SourceIdentity {
        commit: "a".repeat(40),
        tree: "8".repeat(40),
    };
    let error = require_source(&record, &current).unwrap_err().to_string();
    assert!(error.contains("another tree"), "{error}");
}

#[test]
fn source_capture_requires_two_clean_views_of_one_identity() {
    let current = SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    require_stable_source_capture("", &current, "", &current).unwrap();

    let changed = SourceIdentity {
        commit: "9".repeat(40),
        tree: current.tree.clone(),
    };
    assert!(require_stable_source_capture("", &current, "", &changed).is_err());
    assert!(
        require_stable_source_capture(" M kernel/src/main.rs\n", &current, "", &current,).is_err()
    );
    assert!(
        require_stable_source_capture("", &current, "?? transient-input\n", &current,).is_err()
    );
}

#[test]
fn copied_support_bytes_must_match_their_inner_record() {
    let root = crate::test_support::reserve_temp_directory("wizard-qualification-support");
    let paths = Paths::new(root.join("repository"), None);
    let directory = root.join(Kind::Mutation.slug());
    std::fs::create_dir_all(&directory).unwrap();
    let report = directory.join(MUTATION_REPORT);
    std::fs::write(&report, "accepted").unwrap();
    let digest = digest_file(&report).unwrap();
    let mut record = complete_record(Kind::Mutation);
    record.fields.insert(
        "artifact.report.bytes".to_string(),
        digest.bytes.to_string(),
    );
    record
        .fields
        .insert("artifact.report.sha256".to_string(), digest.sha256);
    verify_support(&paths, &record, Kind::Mutation, &root).unwrap();

    std::fs::write(&report, "tampered").unwrap();
    assert!(verify_support(&paths, &record, Kind::Mutation, &root).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn model_support_rejects_omitted_substituted_and_reordered_runs() {
    let root = crate::test_support::reserve_temp_directory("wizard-model-support");
    let paths = Paths::new(root.join("repository"), None);
    let directory = root.join(Kind::Model.slug());
    std::fs::create_dir_all(&directory).unwrap();
    let transcripts = crate::model::fixture_transcripts();
    let mut record = complete_record(Kind::Model);
    for transcript in &transcripts {
        let artifact = transcript.artifact();
        std::fs::write(directory.join(artifact.name), transcript.bytes()).unwrap();
        let digest = digest_bytes(transcript.bytes());
        record.fields.insert(
            format!("{}.bytes", artifact.prefix),
            digest.bytes.to_string(),
        );
        record
            .fields
            .insert(format!("{}.sha256", artifact.prefix), digest.sha256);
    }
    verify_support(&paths, &record, Kind::Model, &root).unwrap();

    let first = transcripts[0].artifact();
    let second = transcripts[1].artifact();
    std::fs::remove_file(directory.join(first.name)).unwrap();
    assert!(verify_support(&paths, &record, Kind::Model, &root).is_err());

    std::fs::write(directory.join(first.name), transcripts[1].bytes()).unwrap();
    let donated = digest_bytes(transcripts[1].bytes());
    record
        .fields
        .insert(format!("{}.bytes", first.prefix), donated.bytes.to_string());
    record
        .fields
        .insert(format!("{}.sha256", first.prefix), donated.sha256);
    assert!(
        verify_support(&paths, &record, Kind::Model, &root).is_err(),
        "a valid clean transcript donated another run's identity"
    );

    std::fs::write(directory.join(first.name), transcripts[0].bytes()).unwrap();
    let restored = digest_bytes(transcripts[0].bytes());
    record.fields.insert(
        format!("{}.bytes", first.prefix),
        restored.bytes.to_string(),
    );
    record
        .fields
        .insert(format!("{}.sha256", first.prefix), restored.sha256);
    verify_support(&paths, &record, Kind::Model, &root).unwrap();
    let first_path = format!("{}.path", first.prefix);
    let second_path = format!("{}.path", second.prefix);
    record.fields.insert(first_path, second.name.to_string());
    record.fields.insert(second_path, first.name.to_string());
    assert!(verify_support(&paths, &record, Kind::Model, &root).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_snapshot_parent_link_cannot_substitute_qualification_bytes() {
    let root = crate::test_support::reserve_temp_directory("wizard-qualification-snapshot-link");
    write_snapshot_bundle(&root);
    let digests = BUNDLE_FILES
        .iter()
        .map(|spec| digest_file(&root.join(bundle_tail(spec).unwrap())).unwrap())
        .collect();
    let snapshot = Snapshot {
        root: root.clone(),
        digests,
        campaigns: CampaignStates::from_labels([PASS, PASS]).unwrap(),
        _mutation: RetainedMutationPass::new(
            CurrentPass::fixture(&root.join("mutation.lock")).unwrap(),
        ),
    };
    snapshot.verify_unchanged().unwrap();
    let model = root.join(Kind::Model.slug());
    let alternate = root.join("model-alternate");
    std::fs::rename(&model, &alternate).unwrap();
    create_directory_symlink(&alternate, &model);

    assert!(snapshot.verify_unchanged().is_err());
    drop(snapshot);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundle_directories_are_validated_before_any_redirected_creation() {
    let root = crate::test_support::reserve_temp_directory("wizard-qualification-install-link");
    let source = root.join("source");
    let evidence = root.join("evidence");
    let outside = root.join("outside");
    write_snapshot_bundle(&source);
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    create_directory_symlink(&outside, &evidence.join("qualification"));
    let digests = BUNDLE_FILES
        .iter()
        .map(|spec| digest_file(&source.join(bundle_tail(spec).unwrap())).unwrap())
        .collect();
    let mutation_lock = source.join("mutation.lock");
    let snapshot = Snapshot {
        root: source.clone(),
        digests,
        campaigns: CampaignStates::from_labels([PASS, PASS]).unwrap(),
        _mutation: RetainedMutationPass::new(CurrentPass::fixture(&mutation_lock).unwrap()),
    };
    assert!(CurrentPass::fixture(&mutation_lock).is_err());

    assert!(snapshot.install(&evidence).is_err());
    assert!(CurrentPass::fixture(&mutation_lock).is_err());
    assert!(
        crate::paths::directory_entries(&outside)
            .unwrap()
            .is_empty(),
        "bundle preparation created a directory through a redirected parent"
    );

    drop(snapshot);
    drop(CurrentPass::fixture(&mutation_lock).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_retains_mutation_lock_through_install_refusal() {
    let root = crate::test_support::reserve_temp_directory("wizard-qualification-install-lock");
    let source = root.join("source");
    let evidence = root.join("not-a-directory");
    write_snapshot_bundle(&source);
    std::fs::write(&evidence, "occupied").unwrap();
    let digests = BUNDLE_FILES
        .iter()
        .map(|spec| digest_file(&source.join(bundle_tail(spec).unwrap())).unwrap())
        .collect();
    let mutation_lock = source.join("mutation.lock");
    let snapshot = Snapshot {
        root: source,
        digests,
        campaigns: CampaignStates::from_labels([PASS, PASS]).unwrap(),
        _mutation: RetainedMutationPass::new(CurrentPass::fixture(&mutation_lock).unwrap()),
    };

    assert!(CurrentPass::fixture(&mutation_lock).is_err());
    assert!(snapshot.install(&evidence).is_err());
    assert!(CurrentPass::fixture(&mutation_lock).is_err());
    drop(snapshot);
    drop(CurrentPass::fixture(&mutation_lock).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn newest_mutation_and_tool_inputs_are_compared_field_by_field() {
    let mutation = complete_record(Kind::Mutation);
    let current = PassEvidence {
        commit: "a".repeat(40),
        registry: "c".repeat(64),
        source_digest: "d".repeat(64),
        index_digest: "e".repeat(64),
        mutation_count: 1,
        report_digest: "0".repeat(64),
        report_path: PathBuf::from("unused.report"),
    };
    require_mutation_inputs(&mutation, &current.registry, 1, Some(&current)).unwrap();
    assert!(require_mutation_inputs(&mutation, &"9".repeat(64), 1, None).is_err());
    assert!(require_mutation_inputs(&mutation, &current.registry, 2, None).is_err());
    for field in ["commit", "registry", "source", "index", "report", "count"] {
        let mut stale = current.clone();
        match field {
            "commit" => stale.commit = "9".repeat(40),
            "registry" => stale.registry = "9".repeat(64),
            "source" => stale.source_digest = "9".repeat(64),
            "index" => stale.index_digest = "9".repeat(64),
            "report" => stale.report_digest = "9".repeat(64),
            "count" => stale.mutation_count = 2,
            _ => unreachable!(),
        }
        assert!(
            require_mutation_inputs(&mutation, &current.registry, 1, Some(&stale)).is_err(),
            "stale mutation {field} was accepted"
        );
    }

    for (kind, keys) in [
        (
            Kind::Model,
            &[
                "input.java.sha256",
                "input.java.version",
                "input.models.sha256",
                "input.tlc-jar.sha256",
            ][..],
        ),
        (Kind::VmwareSelfTest, &["input.vmrun.sha256"][..]),
    ] {
        let recorded = complete_record(kind);
        for key in keys {
            let mut current = recorded.clone();
            current.fields.insert((*key).to_string(), "9".repeat(64));
            assert!(
                require_record_inputs(&recorded, &current).is_err(),
                "stale exceptional input {key} was accepted"
            );
        }
    }
}

#[test]
fn starting_an_attempt_invalidates_an_older_pass() {
    let root = crate::test_support::reserve_temp_directory("wizard-qualification-invalidation");
    let paths = Paths::new(root.join("repo"), Some(&root.join("target")));
    let directory = qualification_root(&paths).join(Kind::VmwareSelfTest.slug());
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(RECORD),
        complete_record(Kind::VmwareSelfTest).render(),
    )
    .unwrap();

    invalidate(&paths, Kind::VmwareSelfTest).unwrap();

    assert!(!directory.join(RECORD).exists());
    assert!(directory.is_dir());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_qualification_kind_directory_cannot_be_substituted() {
    let root = crate::test_support::reserve_temp_directory("wizard-qualification-parent");
    let paths = Paths::new(root.join("repository"), Some(&root.join("target")));
    let qualification = qualification_root(&paths);
    let model = qualification.join(Kind::Model.slug());
    let alternate = qualification.join("alternate");
    std::fs::create_dir_all(&model).unwrap();
    std::fs::create_dir(&alternate).unwrap();
    kind_directory(&paths, Kind::Model).unwrap();

    std::fs::remove_dir(&model).unwrap();
    create_directory_symlink(&alternate, &model);
    assert!(kind_directory(&paths, Kind::Model).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_qualification_root_cannot_be_substituted() {
    let root = crate::test_support::reserve_temp_directory("wizard-qualification-root");
    let target = root.join("target");
    let qualification = target.join("qualification");
    let alternate = target.join("alternate");
    std::fs::create_dir_all(alternate.join(Kind::Model.slug())).unwrap();
    create_directory_symlink(&alternate, &qualification);
    let paths = Paths::new(root.join("repository"), Some(&target));

    assert!(kind_directory(&paths, Kind::Model).is_err());

    let ancestor_target = target.join("ancestor-target");
    let ancestor_link = target.join("ancestor-link");
    std::fs::create_dir_all(ancestor_target.join("qualification")).unwrap();
    create_directory_symlink(&ancestor_target, &ancestor_link);
    assert!(require_real_root(&ancestor_link.join("qualification"), "fixture").is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_redirected_target_is_refused_before_qualification_root_creation() {
    let root = crate::test_support::reserve_temp_directory("wizard-qualification-target-link");
    let outside = root.join("outside");
    let target = root.join("target-link");
    std::fs::create_dir_all(&outside).unwrap();
    create_directory_symlink(&outside, &target);
    let paths = Paths::new(root.join("repository"), Some(&target));

    assert!(invalidate(&paths, Kind::Model).is_err());
    assert!(!outside.join("qualification").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_is_rechecked_immediately_before_a_record_commits() {
    let root = crate::test_support::reserve_temp_directory("wizard-qualification-source");
    let repository = root.join("repository");
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init", "--quiet"]);
    std::fs::write(repository.join("tracked"), "initial\n").unwrap();
    run_git(&repository, &["add", "tracked"]);
    run_git(
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
    let paths = Paths::new(repository.clone(), Some(&root.join("target")));
    let mut record = base_record(&paths, Kind::Model).unwrap();
    fixture_tool(&mut record, "input.java");
    record.insert("input.models.bytes", "1").unwrap();
    record.insert("input.models.files", "1").unwrap();
    record
        .insert("input.models.sha256", "d".repeat(64))
        .unwrap();
    record.insert("input.tlc-jar.bytes", "1").unwrap();
    record
        .insert("input.tlc-jar.sha256", "e".repeat(64))
        .unwrap();
    for artifact in crate::model::TRANSCRIPT_ARTIFACTS {
        fixture_artifact(&mut record, artifact.prefix, artifact.name);
    }
    record.insert("result", PASS).unwrap();
    invalidate(&paths, Kind::Model).unwrap();
    std::fs::write(repository.join("tracked"), "changed\n").unwrap();

    let error = commit_record(&paths, Kind::Model, &record).unwrap_err();
    assert!(
        format!("{error:#}").contains("qualification requires a stable clean checkout"),
        "{error:#}"
    );
    assert!(
        !kind_directory(&paths, Kind::Model)
            .unwrap()
            .join(RECORD)
            .exists()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn exceptional_tasks_commit_only_after_their_terminal_verdict() {
    let dispatcher = include_str!("../lib.rs");
    assert!(
        dispatcher.contains("Task::QualifyAdvisory => qualification::qualify_advisory(&paths),")
    );

    let qualification = PRODUCTION;
    let advisory_start = qualification.find("impl AdvisoryAttempt").unwrap();
    let advisory_end = qualification[advisory_start..]
        .find("fn advisory_command(")
        .unwrap()
        + advisory_start;
    let advisory = &qualification[advisory_start..advisory_end];
    let refresh = advisory
        .find("fresh RustSec advisory database population")
        .unwrap();
    let database = advisory
        .find("let database_identity = advisory_database_identity")
        .unwrap();
    let review = advisory
        .find("revision-pinned RustSec cargo-audit qualification")
        .unwrap();
    let unchanged: Vec<usize> = advisory
        .match_indices("advisory_database_identity(&directory, &database)? == database_identity")
        .map(|(offset, _)| offset)
        .collect();
    assert_eq!(unchanged.len(), 2);
    let transcript = advisory
        .find("let transcript = render_advisory_transcript")
        .unwrap();
    let advisory_result = advisory
        .find("self.record.insert(\"result\", PASS)?;")
        .unwrap();
    let advisory_commit = advisory.rfind("commit_record").unwrap();
    assert!(
        refresh < database
            && database < review
            && review < unchanged[0]
            && unchanged[0] < transcript
            && transcript < unchanged[1]
            && unchanged[1] < advisory_result
            && advisory_result < advisory_commit
    );

    let model = include_str!("../model.rs");
    let begin = model
        .find("let qualification_attempt = qualification::begin_model(paths)?;")
        .unwrap();
    let preflight = model.find("verify_jar(&jar)?;").unwrap();
    let runs = model.find("for run in RUNS {").unwrap();
    let capture_contract = "        transcripts.push(transcript(\n            run,\n            success,\n            report.as_bytes(),\n            error.as_bytes(),\n        )?);";
    let capture = model
        .find(capture_contract)
        .expect("model transcripts capture the classified stdout and stderr bytes");
    assert_eq!(
        model.rfind(capture_contract),
        Some(capture),
        "the model transcript capture contract is unique"
    );
    let pass = model
        .find("qualification_attempt.pass(&transcripts)?;")
        .unwrap();
    assert!(begin < preflight && preflight < runs && runs < capture && capture < pass);

    let mutation_source = include_str!("../mutate.rs");
    let mutation_start = mutation_source
        .rfind("\npub fn run(paths: &Paths) -> Result<()> {")
        .unwrap();
    let mutation_end = mutation_source[mutation_start..]
        .find("\nfn resume_interrupted(")
        .unwrap()
        + mutation_start;
    let mutation = &mutation_source[mutation_start..mutation_end];
    let unique = |marker| {
        let occurrences: Vec<_> = mutation.match_indices(marker).collect();
        assert_eq!(
            occurrences.len(),
            1,
            "mutation run marker `{marker}` is not unique"
        );
        occurrences[0].0
    };
    let begin = unique("let qualification_attempt = qualification::begin_mutation");
    let open = unique("let mut session = Session::open");
    let complete = unique("let outcomes = session.complete");
    let current = unique("let pass = workspace::current_pass");
    let pass = unique("qualification_attempt.pass(&pass)?;");
    assert!(begin < open && open < complete && complete < current && current < pass);

    let start = qualification.find("impl VmwareAttempt").unwrap();
    let end = qualification[start..]
        .find("pub(crate) fn invalidate_vmware_generation")
        .unwrap()
        + start;
    let body = &qualification[start..end];
    let observe = body.find("observe_running_vm").unwrap();
    let result = body.find("self.record.insert(\"result\", PASS)?;").unwrap();
    let commit = body.find("commit_record").unwrap();
    assert!(observe < result && result < commit);
}
