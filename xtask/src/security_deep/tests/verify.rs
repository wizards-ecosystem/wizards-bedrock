use super::*;

#[test]
fn canonical_complete_deep_scan_is_bound_to_the_requested_revision() {
    let source = source();
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    let scan = verify_scan(&manifest, &findings, &coverage, &report, &source).unwrap();
    assert_eq!(scan.id, TEST_SCAN_ID);
    assert_eq!(scan.critical, 0);
}

#[test]
fn report_bytes_must_match_the_exact_canonical_projection() {
    let source = source();
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    verify_scan(&manifest, &findings, &coverage, &report, &source).unwrap();

    let mut tampered = report.clone();
    let marker = b"| Scan outcome | completed |";
    let offset = tampered
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("canonical report contains its scan outcome");
    tampered[offset + marker.len() - 2] = b'!';
    let error = verify_scan(&manifest, &findings, &coverage, &tampered, &source).unwrap_err();
    assert!(
        format!("{error:#}")
            .contains("provider report is not the exact pinned zero-finding projection")
    );
}

#[test]
fn manifest_scope_accepts_official_optionality_and_rejects_unqualified_fields() {
    let source = source();
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    let manifest = String::from_utf8(manifest).unwrap();
    let minimal = manifest.replace(
        r#""scope":{"includePaths":["."],"excludePaths":[],"summary":"Review of the complete `wizards-bedrock` tree.","artifactsReviewed":["Rust *sources*","qualification harness"],"runtimeStatus":"Static and host-side validation completed.","validationMode":"independent | host","context":"Paths [owned] <local>","limitations":["No runtime boot; source-only."]}"#,
        r#""scope":{"includePaths":["."],"excludePaths":[]}"#,
    );
    let minimal_report = String::from_utf8(report.clone())
        .unwrap()
        .replace(
            "Review of the complete `wizards-bedrock` tree.",
            "The scan was configured for the include paths and exclusions listed below.",
        )
        .replace(
            "- Artifacts reviewed: Rust \\*sources\\*, qualification harness\n",
            "",
        )
        .replace("- Scan context: Paths \\[owned\\] \\<local\\>\n", "")
        .replace(
            "\nLimitations and exclusions:\n- No runtime boot; source-only.\n",
            "",
        )
        .replace("independent \\| host", "not recorded");
    verify_scan(
        minimal.as_bytes(),
        &findings,
        &coverage,
        minimal_report.as_bytes(),
        &source,
    )
    .expect("official optional scope fields may be absent");

    let unicode_list_prefix = manifest.replace(
        "Review of the complete `wizards-bedrock` tree.",
        "١. disguised list item",
    );
    let error = verify_scan(
        unicode_list_prefix.as_bytes(),
        &findings,
        &coverage,
        &report,
        &source,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("unsupported Unicode numeric list prefix"));

    let unexpected = manifest.replace(
        r#""runtimeStatus":"Static and host-side validation completed.""#,
        r#""runtimeStatus":"Static and host-side validation completed.","unexpected":"drift""#,
    );
    let error = verify_scan(
        unexpected.as_bytes(),
        &findings,
        &coverage,
        &report,
        &source,
    )
    .unwrap_err();
    assert!(
        format!("{error:#}")
            .contains("scan manifest scope differs from the qualified projection profile")
    );

    for (malformed, expected) in [
        (
            manifest.replace(r#""context":"Paths [owned] <local>""#, r#""context":7"#),
            "scan manifest scope `context` is not a string",
        ),
        (
            manifest.replace(
                r#""summary":"Review of the complete `wizards-bedrock` tree.""#,
                r#""summary":"""#,
            ),
            "scan manifest scope `summary` is empty",
        ),
    ] {
        let error =
            verify_scan(malformed.as_bytes(), &findings, &coverage, &report, &source).unwrap_err();
        assert!(format!("{error:#}").contains(expected));
    }
}

#[test]
fn manifest_threat_model_accepts_structured_optionality_and_rejects_unqualified_fields() {
    let source = source();
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    let manifest = String::from_utf8(manifest).unwrap();
    let optional_fields = r#","assets":["Kernel [state]"],"trustBoundaries":["Host > guest boundary"],"attackerCapabilities":["Can supply repository inputs."],"securityObjectives":["Keep authority local."],"assumptions":["Pinned tools are trusted."],"scope":{"includePaths":["."],"excludePaths":[],"summary":"Repository source."},"origin":"reconciled""#;
    let minimal = manifest.replace(optional_fields, "");
    assert_ne!(minimal, manifest, "fixture has structured optional fields");
    let structured_sections = "\n\n### Assets\n\n- Kernel \\[state\\]\n\n### Trust Boundaries\n\n- Host \\> guest boundary\n\n### Attacker Capabilities\n\n- Can supply repository inputs.\n\n### Security Objectives\n\n- Keep authority local.\n\n### Assumptions\n\n- Pinned tools are trusted.";
    let minimal_report = String::from_utf8(report.clone())
        .unwrap()
        .replace(structured_sections, "");
    verify_scan(
        minimal.as_bytes(),
        &findings,
        &coverage,
        minimal_report.as_bytes(),
        &source,
    )
    .expect("official structured threat-model fields may be absent");

    let blank_summary = manifest.replace(
        r#""summary":"Assets in `kernel` and *host*.""#,
        r#""summary":"   ""#,
    );
    let fallback_report = String::from_utf8(report.clone()).unwrap().replace(
        "Assets in `kernel` and \\*host\\*.",
        "No explicit canonical threat-model summary was recorded.",
    );
    verify_scan(
        blank_summary.as_bytes(),
        &findings,
        &coverage,
        fallback_report.as_bytes(),
        &source,
    )
    .expect("the pinned renderer fallback for a whitespace-only structured summary");

    let unexpected = manifest.replace(
        r#""origin":"reconciled""#,
        r#""origin":"reconciled","unexpected":"drift""#,
    );
    let error = verify_scan(
        unexpected.as_bytes(),
        &findings,
        &coverage,
        &report,
        &source,
    )
    .unwrap_err();
    assert!(
        format!("{error:#}")
            .contains("scan manifest threat model differs from the qualified projection profile")
    );
}

#[test]
fn markdown_threat_model_is_embedded_verbatim_and_strictly_typed() {
    let source = source();
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    let rich_threat_model = r#""threatModel":{"summary":"Assets in `kernel` and *host*.","assets":["Kernel [state]"],"trustBoundaries":["Host > guest boundary"],"attackerCapabilities":["Can supply repository inputs."],"securityObjectives":["Keep authority local."],"assumptions":["Pinned tools are trusted."],"scope":{"includePaths":["."],"excludePaths":[],"summary":"Repository source."},"origin":"reconciled"}"#;
    let markdown_threat_model = r####""threatModel":{"format":"markdown","content":"### Model\n\nRaw *Markdown* with `code`.","origin":"provided","scope":{"includePaths":["."],"summary":"Modeled source."}}"####;
    let manifest = String::from_utf8(manifest)
        .unwrap()
        .replace(rich_threat_model, markdown_threat_model);
    let structured = "Assets in `kernel` and \\*host\\*.\n\n### Assets\n\n- Kernel \\[state\\]\n\n### Trust Boundaries\n\n- Host \\> guest boundary\n\n### Attacker Capabilities\n\n- Can supply repository inputs.\n\n### Security Objectives\n\n- Keep authority local.\n\n### Assumptions\n\n- Pinned tools are trusted.";
    let report = String::from_utf8(report)
        .unwrap()
        .replace(structured, "### Model\n\nRaw *Markdown* with `code`.");
    verify_scan(
        manifest.as_bytes(),
        &findings,
        &coverage,
        report.as_bytes(),
        &source,
    )
    .expect("the official Markdown threat-model form");

    let unexpected = manifest.replace(
        r#""summary":"Modeled source.""#,
        r#""summary":"Modeled source.","unexpected":"drift""#,
    );
    let error = verify_scan(
        unexpected.as_bytes(),
        &findings,
        &coverage,
        report.as_bytes(),
        &source,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains(
        "scan manifest threat model scope differs from the qualified projection profile"
    ));

    for (malformed, expected) in [
        (
            manifest.replace(
                r#""scope":{"includePaths":["."],"summary":"Modeled source."}"#,
                r#""scope":{"includePaths":["src"],"summary":"Modeled source."}"#,
            ),
            "scan manifest threat model scope is not the complete repository",
        ),
        (
            manifest.replace(
                r#""scope":{"includePaths":["."],"summary":"Modeled source."}"#,
                r#""scope":{"includePaths":["."],"excludePaths":["generated"],"summary":"Modeled source."}"#,
            ),
            "scan manifest threat model scope excludes paths",
        ),
    ] {
        let error = verify_scan(
            malformed.as_bytes(),
            &findings,
            &coverage,
            report.as_bytes(),
            &source,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains(expected));
    }

    for (malformed, expected) in [
        (
            manifest.replace(r#""format":"markdown""#, r#""format":"html""#),
            "scan manifest threat model format is unsupported",
        ),
        (
            manifest.replace(
                r####""content":"### Model\n\nRaw *Markdown* with `code`.""####,
                r#""content":"   ""#,
            ),
            "scan manifest threat model content is blank",
        ),
        (
            manifest.replace(r#""origin":"provided""#, r#""origin":"inherited""#),
            "scan manifest threat model origin is unsupported",
        ),
        (
            manifest.replace(
                r#""scope":{"includePaths":["."],"summary":"Modeled source."}"#,
                r#""scope":{"includePaths":".","summary":"Modeled source."}"#,
            ),
            "scan manifest threat model scope is not an array",
        ),
    ] {
        let error = verify_scan(
            malformed.as_bytes(),
            &findings,
            &coverage,
            report.as_bytes(),
            &source,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains(expected));
    }
}

#[test]
fn manifest_preserved_sources_are_canonical_sha256_bindings() {
    let source = source();
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    let manifest = String::from_utf8(manifest).unwrap();
    for invalid in [
        manifest.replace(
            &format!(
                r#""preservedSources":{{"workers/discovery/checkpoint.json":"{}"}}"#,
                "2".repeat(64)
            ),
            r#""preservedSources":{}"#,
        ),
        manifest.replace(
            "workers/discovery/checkpoint.json",
            "workers/../checkpoint.json",
        ),
        manifest.replace(&"2".repeat(64), &"G".repeat(64)),
    ] {
        assert!(
            verify_scan(invalid.as_bytes(), &findings, &coverage, &report, &source).is_err(),
            "accepted a malformed preserved-source binding"
        );
    }
}

#[test]
fn unsigned_zero_finding_artifacts_are_diagnostic_only() {
    let source = source();
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    verify_scan(&manifest, &findings, &coverage, &report, &source).unwrap();

    let implementation = include_str!("../verify.rs");
    let verify = implementation.find("fn verify(").unwrap();
    let semantic_parser = implementation[verify..].find("fn verify_scan(").unwrap() + verify;
    let authority_body = &implementation[verify..semantic_parser];
    assert!(!authority_body.contains("pass_observed_mutants"));
    assert!(!authority_body.contains("commit_record"));
    assert!(authority_body.contains("no qualification PASS was created"));
}

#[test]
fn foreign_partial_and_high_finding_scans_are_refused() {
    let source = source();
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    let foreign_manifest = String::from_utf8(manifest)
        .unwrap()
        .replace(&source.commit, &"c".repeat(40));
    let error = verify_scan(
        foreign_manifest.as_bytes(),
        &findings,
        &coverage,
        &report,
        &source,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("scan belongs to another commit"));
    let (manifest, findings, coverage, report) = artifacts(None, "partial", &source.commit);
    assert!(verify_scan(&manifest, &findings, &coverage, &report, &source).is_err());
    let (manifest, findings, coverage, report) =
        artifacts(Some("high"), "complete", &source.commit);
    assert!(verify_scan(&manifest, &findings, &coverage, &report, &source).is_err());
}

#[test]
fn strict_profile_rejects_provider_drift_reversed_time_and_caveats() {
    let source = source();
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    let manifest = String::from_utf8(manifest).unwrap();
    verify_scan(manifest.as_bytes(), &findings, &coverage, &report, &source)
        .expect("current qualified producer");
    let foreign = manifest.replace("codex-security-plugin", "other-provider");
    assert!(verify_scan(foreign.as_bytes(), &findings, &coverage, &report, &source).is_err());
    let stale = manifest.replace("0.1.32", "0.1.31");
    assert!(verify_scan(stale.as_bytes(), &findings, &coverage, &report, &source).is_err());
    let reversed = manifest.replace("2026-09-28T00:00:00Z", "2026-09-28T02:00:00Z");
    assert!(verify_scan(reversed.as_bytes(), &findings, &coverage, &report, &source).is_err());
    let limited = manifest.replace(
        r#""scope":{"includePaths"#,
        r#""limitations":["partial"],"scope":{"includePaths"#,
    );
    assert!(verify_scan(limited.as_bytes(), &findings, &coverage, &report, &source).is_err());
    let limited_scope = manifest.replace(
        r#""validationMode":"independent | host""#,
        r#""validationMode":"sampled""#,
    );
    assert!(
        verify_scan(
            limited_scope.as_bytes(),
            &findings,
            &coverage,
            &report,
            &source,
        )
        .is_err()
    );
}

/// Replace one coverage fragment and re-bind the manifest to the new bytes,
/// so only the coverage rule under test can refuse the scan.
fn with_coverage(from: &str, to: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let source = source();
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    let old = String::from_utf8(coverage).unwrap();
    assert!(old.contains(from), "fixture lacks {from}");
    let coverage = old.replacen(from, to, 1).into_bytes();
    let manifest = String::from_utf8(manifest)
        .unwrap()
        .replace(
            &sha256::hex(&sha256::digest(old.as_bytes())),
            &sha256::hex(&sha256::digest(&coverage)),
        )
        .into_bytes();
    (manifest, findings, coverage, report)
}

#[test]
fn coverage_must_be_the_complete_unqualified_repository_workflow() {
    let source = source();
    let (manifest, findings, coverage, report) =
        with_coverage(r#""mode":"deep_repository""#, r#""mode":"deep_repository""#);
    verify_scan(&manifest, &findings, &coverage, &report, &source).unwrap();
    for (from, to) in [
        (
            r#""mode":"deep_repository""#,
            r#""mode":"quick_repository""#,
        ),
        (
            r#""inventoryStrategy":"repository""#,
            r#""inventoryStrategy":"sampled""#,
        ),
        (
            r#""explicitExclusions":[]"#,
            r#""explicitExclusions":["vm"]"#,
        ),
        (r#""deferred":[]"#, r#""deferred":["kernel"]"#),
        (
            r#""openQuestions":[]"#,
            r#""openQuestions":["unresolved authority"]"#,
        ),
        (r#""surfaces":[]"#, r#""surfaces":[{}]"#),
    ] {
        let (manifest, findings, coverage, report) = with_coverage(from, to);
        assert!(
            verify_scan(&manifest, &findings, &coverage, &report, &source).is_err(),
            "accepted coverage with {to}"
        );
    }
}

#[test]
fn manifest_hashes_are_semantic_not_decorative() {
    let source = source();
    let (manifest, mut findings, coverage, report) = artifacts(None, "complete", &source.commit);
    findings.push(b' ');
    assert!(verify_scan(&manifest, &findings, &coverage, &report, &source).is_err());
}

#[test]
fn request_is_exactly_source_and_workflow_bound() {
    let source = source();
    let request = render_request(&source);
    verify_request(&request, &source).unwrap();
    assert!(
        verify_request(
            &request,
            &SourceIdentity {
                commit: "c".repeat(40),
                tree: source.tree
            }
        )
        .is_err()
    );
}

#[test]
fn owned_request_binds_every_runtime_and_authority_field() {
    let source = source();
    let tools = owned_tools();
    let git = owned_git();
    let request = render_owned_request(
        &source,
        &tools,
        &git,
        &owned_package(),
        &owned_permission_digests(),
    );
    let fields = verify_owned_request(&request, &source, &tools.python_runtime, &git).unwrap();
    let fields: BTreeMap<_, _> = fields.into_iter().collect();
    assert_eq!(fields["execution.protocol"], MCP_PROTOCOL);
    assert_eq!(
        fields["environment.policy"],
        "xtask-owned-codex-security-hermetic-v9"
    );
    assert_eq!(
        fields["execution.authority"],
        "xtask-mcp-wrapper-windows-job-v8"
    );
    assert_eq!(
        fields["execution.worker-permission"],
        "provider-envelope=managed-root-read+explicit-deny;effective-worker=exec-only:backend=mxc,root=deny,minimal+owned-repository+scan+plugin+qualified-runtime=read,repository-metadata+credential=deny,outside-canary=default-deny-probed,network=false"
    );
    assert_eq!(
        fields["execution.worker-parent-state.sha256"],
        "1".repeat(64)
    );
    assert_eq!(
        fields["execution.worker-provider-profile.sha256"],
        "2".repeat(64)
    );
    assert_eq!(
        fields["execution.worker-effective-profile.sha256"],
        "3".repeat(64)
    );
    assert_eq!(
        fields["execution.worker-effective-state.sha256"],
        "4".repeat(64)
    );
    assert_eq!(fields["tool.node.sha256"], NODE_SHA256);
    assert_eq!(fields["tool.python.sha256"], PYTHON_SHA256);
    assert_eq!(fields["tool.codex.sha256"], CODEX_SHA256);
    assert_eq!(fields["tool.codex.runtime.sha256"], CODEX_RUNTIME_SHA256);

    let text = String::from_utf8(request).unwrap();
    for line in text
        .lines()
        .skip(1)
        .filter(|line| !line.starts_with("checksum.sha256="))
    {
        let (key, value) = line.split_once('=').unwrap();
        let replacement = format!("{key}={value}x");
        let changed = text.replacen(line, &replacement, 1);
        assert!(
            verify_owned_request(
                &rebind_owned_request(&changed),
                &source,
                &tools.python_runtime,
                &git,
            )
            .is_err(),
            "accepted substitution of owned request field {key}"
        );
    }
}

#[test]
fn record_fields_bind_exact_permission_digests() {
    let source = source();
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    let scan = verify_scan(&manifest, &findings, &coverage, &report, &source).unwrap();
    let permissions = owned_permission_digests();
    let fields: BTreeMap<_, _> = fields(
        &scan,
        &owned_tools(),
        &owned_package(),
        &permissions,
        &owned_transition_ledger(),
    )
    .into_iter()
    .collect();
    assert_eq!(
        fields["execution.worker-effective-profile.sha256"],
        permissions.effective_profile
    );
    assert_eq!(
        fields["execution.worker-effective-state.sha256"],
        permissions.effective_state
    );
    assert_eq!(
        fields["execution.worker-parent-state.sha256"],
        permissions.parent_state
    );
    assert_eq!(
        fields["execution.worker-provider-profile.sha256"],
        permissions.provider_profile
    );
    assert_eq!(fields["execution.worker-transitions.app-server"], "1");
    assert_eq!(fields["execution.worker-transitions.exec-initial"], "2");
    assert_eq!(fields["execution.worker-transitions.exec-resumed"], "3");
    assert_eq!(fields["execution.worker-preflight.allowed-reads"], "7");
    assert_eq!(fields["execution.worker-preflight.denied-reads"], "4");
    assert_eq!(
        fields["execution.worker-preflight.denied-write-creates"],
        "2"
    );
    assert_eq!(
        fields["execution.worker-preflight.denied-write-mutations"],
        "3"
    );
    assert_eq!(
        fields["execution.worker-preflight.denied-network-listeners"],
        "1"
    );
    assert_eq!(
        fields["execution.worker-preflight.denied-network-connections"],
        "1"
    );
    assert_eq!(fields["execution.worker-preflight.denied-descendants"], "1");
}

#[test]
fn worker_transition_fields_bind_pairwise_distinct_counts() {
    let transitions = WorkerTransitionLedger {
        bytes: Vec::new(),
        app_server: 37,
        exec_initial: 41,
        exec_resumed: 43,
        exec_exited_zero: 47,
        thread_ids: BTreeSet::from(["thread-1".to_string()]),
        execution_threads: BTreeSet::from(["thread-1".to_string()]),
        probes: mapping_probe_summary(),
    };
    let fields: BTreeMap<_, _> = worker_transition_fields(&transitions).into_iter().collect();
    let expected: BTreeMap<_, _> = [
        ("execution.worker-preflight.allowed-reads", "11"),
        ("execution.worker-preflight.denied-reads", "13"),
        ("execution.worker-preflight.denied-write-creates", "17"),
        ("execution.worker-preflight.denied-write-mutations", "19"),
        ("execution.worker-preflight.denied-network-listeners", "23"),
        (
            "execution.worker-preflight.denied-network-connections",
            "29",
        ),
        ("execution.worker-preflight.denied-descendants", "31"),
        ("execution.worker-transitions.app-server", "37"),
        ("execution.worker-transitions.exec-initial", "41"),
        ("execution.worker-transitions.exec-resumed", "43"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect();
    assert_eq!(fields, expected);
}

#[test]
fn owned_request_rejects_source_and_tool_substitution_after_rebinding() {
    let source = source();
    let tools = owned_tools();
    let git = owned_git();
    let request = render_owned_request(
        &source,
        &tools,
        &git,
        &owned_package(),
        &owned_permission_digests(),
    );
    let text = String::from_utf8(request).unwrap();
    for changed in [
        text.replace(&source.commit, &"c".repeat(40)),
        text.replace(NODE_SHA256, &"d".repeat(64)),
        text.replace(AUTHENTICATION_CHANNEL, "api-key"),
    ] {
        assert!(
            verify_owned_request(
                &rebind_owned_request(&changed),
                &source,
                &tools.python_runtime,
                &git,
            )
            .is_err(),
            "accepted re-bound owned request substitution"
        );
    }
}

#[test]
fn workspace_identity_and_time_are_strict() {
    for accepted in [
        "123e4567-e89b-12d3-a456-426614174000",
        "123E4567-E89B-82D3-B456-426614174000",
    ] {
        require_uuid(accepted, "test UUID").unwrap();
    }
    for rejected in [
        "123e4567-e89b-02d3-a456-426614174000",
        "123e4567-e89b-12d3-7456-426614174000",
        "123e4567e89b12d3a456426614174000",
        "zzze4567-e89b-12d3-a456-426614174000",
    ] {
        assert!(require_uuid(rejected, "test UUID").is_err());
    }
    crate::mutants_grade::parse_utc_timestamp("2026-10-14T01:02:03.123Z", "test timestamp")
        .unwrap();
    for rejected in [
        "2026-10-14Z",
        "2026-10-14T25:02:03Z",
        "2026-10-14T01:02:03+00:00",
        "garbageZ",
    ] {
        assert!(crate::mutants_grade::parse_utc_timestamp(rejected, "test timestamp").is_err());
    }
}

#[test]
fn sealed_verifier_derives_only_owned_request_and_scan_fields() {
    let source = source();
    let root = unique_test_directory("wizard-security-deep-sealed-test");
    let tools = owned_tools();
    let git = owned_git();
    let permissions = owned_permission_digests();
    let transitions = test_worker_transition_ledger(&source, &permissions);
    let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
    for (name, bytes) in [
        (
            REQUEST,
            render_owned_request(&source, &tools, &git, &owned_package(), &permissions),
        ),
        (MANIFEST, manifest),
        (FINDINGS, findings),
        (COVERAGE, coverage),
        (REPORT, report),
        (WORKER_TRANSITIONS, transitions.bytes),
    ] {
        std::fs::write(root.join(name), bytes).unwrap();
    }
    let fields =
        verify_sealed_with_identities(&root, &source, &tools.python_runtime, &git).unwrap();
    let fields: BTreeMap<_, _> = fields.into_iter().collect();
    let expected: BTreeMap<String, String> = [
        ("authentication.channel", AUTHENTICATION_CHANNEL.to_string()),
        ("environment.policy", ENVIRONMENT_POLICY.to_string()),
        ("execution.authority", EXECUTION_AUTHORITY.to_string()),
        ("execution.complete-tool", COMPLETE_TOOL.to_string()),
        ("execution.model", MODEL.to_string()),
        ("execution.protocol", MCP_PROTOCOL.to_string()),
        ("execution.reasoning", REASONING.to_string()),
        ("execution.server-name", MCP_SERVER_NAME.to_string()),
        ("execution.server-version", MCP_SERVER_VERSION.to_string()),
        ("execution.start-tool", START_TOOL.to_string()),
        ("execution.tools.bytes", MCP_TOOLS_BYTES.to_string()),
        ("execution.tools.count", MCP_TOOLS_COUNT.to_string()),
        ("execution.tools.sha256", MCP_TOOLS_SHA256.to_string()),
        ("execution.worker-permission", WORKER_PERMISSION.to_string()),
        ("execution.worker-effective-profile.sha256", "3".repeat(64)),
        ("execution.worker-effective-state.sha256", "4".repeat(64)),
        ("execution.worker-parent-state.sha256", "1".repeat(64)),
        ("execution.worker-provider-profile.sha256", "2".repeat(64)),
        ("execution.worker-preflight.allowed-reads", "7".to_string()),
        ("execution.worker-preflight.denied-reads", "4".to_string()),
        (
            "execution.worker-preflight.denied-write-creates",
            "2".to_string(),
        ),
        (
            "execution.worker-preflight.denied-write-mutations",
            "3".to_string(),
        ),
        (
            "execution.worker-preflight.denied-network-listeners",
            "1".to_string(),
        ),
        (
            "execution.worker-preflight.denied-network-connections",
            "1".to_string(),
        ),
        (
            "execution.worker-preflight.denied-descendants",
            "1".to_string(),
        ),
        ("execution.worker-transitions.app-server", "2".to_string()),
        ("execution.worker-transitions.exec-initial", "1".to_string()),
        ("execution.worker-transitions.exec-resumed", "1".to_string()),
        ("findings.critical", "0".to_string()),
        ("findings.high", "0".to_string()),
        ("findings.informational", "0".to_string()),
        ("findings.low", "0".to_string()),
        ("findings.medium", "0".to_string()),
        ("findings.total", "0".to_string()),
        ("scan.id", TEST_SCAN_ID.to_string()),
        ("scan.producer.name", PRODUCER_NAME.to_string()),
        ("scan.producer.version", PRODUCER_VERSION.to_string()),
        ("scan.surfaces", "0".to_string()),
        (
            "tool.codex-security.integrity.bytes",
            PLUGIN_INTEGRITY_BYTES.to_string(),
        ),
        (
            "tool.codex-security.integrity.sha256",
            PLUGIN_INTEGRITY_SHA256.to_string(),
        ),
        ("tool.codex-security.tree.files", PLUGIN_FILES.to_string()),
        ("tool.codex-security.version", PRODUCER_VERSION.to_string()),
        ("tool.node.bytes", NODE_BYTES.to_string()),
        ("tool.node.sha256", NODE_SHA256.to_string()),
        ("tool.node.version", NODE_VERSION.to_string()),
        ("tool.python.bytes", PYTHON_BYTES.to_string()),
        ("tool.python.sha256", PYTHON_SHA256.to_string()),
        ("tool.python.version", PYTHON_VERSION.to_string()),
        ("tool.python.runtime.bytes", "1".to_string()),
        ("tool.python.runtime.files", "1".to_string()),
        (
            "tool.python.runtime.root",
            "parent-of-tool.python".to_string(),
        ),
        ("tool.python.runtime.sha256", "0".repeat(64)),
        ("tool.codex.bytes", CODEX_BYTES.to_string()),
        ("tool.codex.sha256", CODEX_SHA256.to_string()),
        ("tool.codex.version", CODEX_VERSION.to_string()),
        ("tool.codex.runtime.bytes", CODEX_RUNTIME_BYTES.to_string()),
        (
            "tool.codex.runtime.files",
            CODEX_RUNTIME_FILES.len().to_string(),
        ),
        (
            "tool.codex.runtime.root",
            "parent-of-tool.codex".to_string(),
        ),
        (
            "tool.codex.runtime.sha256",
            CODEX_RUNTIME_SHA256.to_string(),
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect();
    assert_eq!(fields, expected);
    std::fs::write(root.join(WORKER_TRANSITIONS), b"forged transitions\n").unwrap();
    assert!(verify_sealed_with_identities(&root, &source, &tools.python_runtime, &git).is_err());
    let metadata = std::fs::symlink_metadata(&root).unwrap();
    remove_entry_no_follow(&root, &metadata).unwrap();
}
