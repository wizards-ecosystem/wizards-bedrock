use super::*;

#[test]
fn worker_transition_receipts_and_ledger_are_canonical_and_source_bound() {
    let source = source();
    let permissions = owned_permission_digests();
    let ledger = test_worker_transition_ledger(&source, &permissions);
    assert!(
        ledger
            .bytes
            .starts_with(b"wizard-codex-security-worker-transitions-v3\n")
    );
    let verified = verify_worker_transition_ledger(&ledger.bytes, &source, &permissions).unwrap();
    assert_eq!(verified, ledger);

    let receipt = WorkerTransitionReceipt {
        kind: WorkerTransitionKind::ExecInitialAttenuated,
        stage: WorkerTransitionStage::Started,
        process_id: 41,
        source_commit: source.commit.clone(),
        provider_profile: permissions.provider_profile.clone(),
        effective_profile: permissions.effective_profile.clone(),
        applied_profile: permissions.effective_profile.clone(),
        worker: "5".repeat(64),
        execution_thread: Some("thread-41".to_string()),
        arguments: "4".repeat(64),
    };
    let bytes = render_worker_transition_receipt(&receipt).unwrap();
    assert_eq!(parse_worker_transition_receipt(&bytes).unwrap(), receipt);
    let mut wrong_applied = receipt.clone();
    wrong_applied.kind = WorkerTransitionKind::AppServerPassThrough;
    assert!(require_worker_transition_receipt(&wrong_applied, &source, &permissions).is_err());
    require_worker_transition_receipt(&receipt, &source, &permissions).unwrap();
    let mut wrong_profile = receipt.clone();
    wrong_profile.applied_profile = permissions.provider_profile.clone();
    assert!(require_worker_transition_receipt(&wrong_profile, &source, &permissions).is_err());
    assert_eq!(
        WorkerTransitionKind::AppServerPassThrough.applied_digest("provider", "effective"),
        "provider"
    );
    for kind in [
        WorkerTransitionKind::ExecInitialAttenuated,
        WorkerTransitionKind::ExecResumedAttenuated,
    ] {
        assert_eq!(kind.applied_digest("provider", "effective"), "effective");
    }
    let mut malformed = receipt.clone();
    malformed.stage = WorkerTransitionStage::Launched;
    assert!(render_worker_transition_receipt(&malformed).is_err());
    assert!(require_worker_transition_receipt(&malformed, &source, &permissions).is_err());
    let mut noncanonical = String::from_utf8(bytes.clone())
        .unwrap()
        .replace("wrapper.id=41\n", "wrapper.id=041\n");
    let checksum_offset = noncanonical.rfind("checksum.sha256=").unwrap();
    let checksum = sha256::hex(&sha256::digest(&noncanonical.as_bytes()[..checksum_offset]));
    noncanonical.replace_range(checksum_offset.., &format!("checksum.sha256={checksum}\n"));
    assert!(parse_worker_transition_receipt(noncanonical.as_bytes()).is_err());

    let text = String::from_utf8(ledger.bytes.clone()).unwrap();
    for changed in [
        text.replace(
            "transition.app-server-launched=2",
            "transition.app-server-launched=3",
        ),
        text.replace(
            "completion.execution-thread.0000=thread-1",
            "completion.execution-thread.0000=thread-2",
        ),
        text.replace(&source.commit, &"c".repeat(40)),
        text.replace(&permissions.effective_profile, &"d".repeat(64)),
        text.replace(&permissions.effective_state, &"e".repeat(64)),
        text.replace("preflight.allowed-reads=7", "preflight.allowed-reads=0"),
        text.replace("preflight.denied-reads=4", "preflight.denied-reads=3"),
        text.replace(
            "preflight.denied-write-creates=2",
            "preflight.denied-write-creates=1",
        ),
        text.replace(
            "preflight.denied-write-mutations=3",
            "preflight.denied-write-mutations=2",
        ),
        text.replace(
            "preflight.denied-network-listeners=1",
            "preflight.denied-network-listeners=0",
        ),
        text.replace(
            "preflight.denied-network-connections=1",
            "preflight.denied-network-connections=0",
        ),
        text.replace(
            "preflight.denied-descendants=1",
            "preflight.denied-descendants=0",
        ),
    ] {
        let changed = rebind_worker_transition_ledger(&changed);
        assert!(verify_worker_transition_ledger(&changed, &source, &permissions).is_err());
    }
}

#[test]
fn exec_jsonl_observation_binds_thread_identity_and_preserves_other_events() {
    let started = b"{\"type\":\"thread.started\",\"thread_id\":\"thread-1\"}\n";
    let mut observed = None;
    assert_eq!(
        observe_private_exec_line(started, Some("thread-1"), &mut observed).unwrap(),
        Some("thread-1".to_string())
    );
    assert_eq!(observed.as_deref(), Some("thread-1"));
    assert!(observe_private_exec_line(started, Some("thread-1"), &mut observed).is_err());

    let mut observed = None;
    assert!(observe_private_exec_line(started, Some("thread-2"), &mut observed).is_err());
    assert!(observed.is_none());
    assert!(observe_private_exec_line(b"not-json\n", None, &mut observed).is_err());
    assert!(
        observe_private_exec_line(
            b"{\"type\":\"thread.started\",\"thread_id\":\"bad thread\"}\n",
            None,
            &mut observed,
        )
        .is_err()
    );

    let invocation = InjectedCodexInvocation {
        arguments: Vec::new(),
        transition: WorkerTransitionKind::ExecInitialAttenuated,
        worker: None,
        expected_thread: None,
    };
    let event = b"{\"type\":\"item.completed\",\"item\":{\"text\":\"exact bytes\"}}\n";
    let mut output = Vec::new();
    relay_private_exec_line(
        &"a".repeat(40),
        &invocation,
        Path::new("unused"),
        event,
        &mut None,
        &mut output,
    )
    .unwrap();
    assert_eq!(output, event);
}

#[test]
fn worker_transition_ledger_requires_coverage_ancestry_and_lifecycle_pairing() {
    let source = source();
    let permissions = owned_permission_digests();
    let receipt =
        |kind, stage, process_id, worker: char, thread: Option<&str>| WorkerTransitionReceipt {
            kind,
            stage,
            process_id,
            source_commit: source.commit.clone(),
            provider_profile: permissions.provider_profile.clone(),
            effective_profile: permissions.effective_profile.clone(),
            applied_profile: kind
                .applied_digest(
                    &permissions.provider_profile,
                    &permissions.effective_profile,
                )
                .to_string(),
            worker: worker.to_string().repeat(64),
            execution_thread: thread.map(str::to_string),
            arguments: format!("{process_id:064x}"),
        };
    let threads = BTreeSet::from([
        "thread-1".to_string(),
        "thread-2".to_string(),
        "thread-extra".to_string(),
    ]);
    let execution = BTreeSet::from(["thread-1".to_string()]);
    let base = vec![
        receipt(
            WorkerTransitionKind::AppServerPassThrough,
            WorkerTransitionStage::Launched,
            1,
            'a',
            None,
        ),
        receipt(
            WorkerTransitionKind::AppServerPassThrough,
            WorkerTransitionStage::Launched,
            2,
            'a',
            None,
        ),
        receipt(
            WorkerTransitionKind::ExecInitialAttenuated,
            WorkerTransitionStage::Started,
            3,
            'a',
            Some("thread-1"),
        ),
        receipt(
            WorkerTransitionKind::ExecResumedAttenuated,
            WorkerTransitionStage::Started,
            4,
            'a',
            Some("thread-1"),
        ),
    ];
    render_worker_transition_ledger(
        &base,
        &threads,
        &execution,
        &source,
        &permissions,
        &owned_probe_summary(),
    )
    .unwrap();

    let uncovered = BTreeSet::from(["thread-2".to_string()]);
    assert!(
        render_worker_transition_ledger(
            &base,
            &threads,
            &uncovered,
            &source,
            &permissions,
            &owned_probe_summary(),
        )
        .is_err()
    );

    let mut unrelated_resume = base.clone();
    unrelated_resume[3].execution_thread = Some("thread-2".to_string());
    assert!(
        render_worker_transition_ledger(
            &unrelated_resume,
            &threads,
            &execution,
            &source,
            &permissions,
            &owned_probe_summary(),
        )
        .is_err()
    );

    let mut missing_preflight = base.clone();
    missing_preflight[1].worker = "b".repeat(64);
    assert!(
        render_worker_transition_ledger(
            &missing_preflight,
            &threads,
            &execution,
            &source,
            &permissions,
            &owned_probe_summary(),
        )
        .is_err()
    );

    let mut unmatched_exit = base.clone();
    unmatched_exit.push(receipt(
        WorkerTransitionKind::ExecInitialAttenuated,
        WorkerTransitionStage::ExitedZero,
        99,
        'a',
        Some("thread-1"),
    ));
    assert!(
        render_worker_transition_ledger(
            &unmatched_exit,
            &threads,
            &execution,
            &source,
            &permissions,
            &owned_probe_summary(),
        )
        .is_err()
    );

    let mut with_matched_exit = base.clone();
    let mut exited = with_matched_exit[2].clone();
    exited.stage = WorkerTransitionStage::ExitedZero;
    with_matched_exit.push(exited);
    render_worker_transition_ledger(
        &with_matched_exit,
        &threads,
        &execution,
        &source,
        &permissions,
        &owned_probe_summary(),
    )
    .unwrap();

    let mut abandoned_retry = base;
    abandoned_retry.push(receipt(
        WorkerTransitionKind::AppServerPassThrough,
        WorkerTransitionStage::Launched,
        5,
        'a',
        None,
    ));
    abandoned_retry.push(receipt(
        WorkerTransitionKind::ExecInitialAttenuated,
        WorkerTransitionStage::Started,
        6,
        'a',
        Some("thread-extra"),
    ));
    render_worker_transition_ledger(
        &abandoned_retry,
        &threads,
        &execution,
        &source,
        &permissions,
        &owned_probe_summary(),
    )
    .unwrap();
}

#[test]
fn transition_aggregation_requires_every_crossing_and_rejects_unknown_files() {
    let (outer, scratch) = test_owned_scratch();
    let source = source();
    let permissions = owned_permission_digests();
    let receipt =
        |kind, stage, process_id, execution_thread: Option<&str>| WorkerTransitionReceipt {
            kind,
            stage,
            process_id,
            source_commit: source.commit.clone(),
            provider_profile: permissions.provider_profile.clone(),
            effective_profile: permissions.effective_profile.clone(),
            applied_profile: kind
                .applied_digest(
                    &permissions.provider_profile,
                    &permissions.effective_profile,
                )
                .to_string(),
            worker: "a".repeat(64),
            execution_thread: execution_thread.map(str::to_string),
            arguments: format!("{process_id:064x}"),
        };
    for transition in [
        receipt(
            WorkerTransitionKind::AppServerPassThrough,
            WorkerTransitionStage::Launched,
            11,
            None,
        ),
        receipt(
            WorkerTransitionKind::AppServerPassThrough,
            WorkerTransitionStage::Launched,
            14,
            None,
        ),
        receipt(
            WorkerTransitionKind::ExecInitialAttenuated,
            WorkerTransitionStage::Started,
            12,
            Some("thread-1"),
        ),
        receipt(
            WorkerTransitionKind::ExecResumedAttenuated,
            WorkerTransitionStage::Started,
            13,
            Some("thread-1"),
        ),
    ] {
        write_new_file(
            &scratch.transitions.join(format!(
                "{}-{}-{}.receipt",
                transition.kind.as_str(),
                transition.stage.as_str(),
                transition.process_id
            )),
            &render_worker_transition_receipt(&transition).unwrap(),
        )
        .unwrap();
    }
    let thread_ids = BTreeSet::from(["thread-1".to_string()]);
    let ledger = aggregate_worker_transitions(
        &scratch.transitions,
        &thread_ids,
        &thread_ids,
        &source,
        &permissions,
        &owned_probe_summary(),
    )
    .unwrap();
    assert_eq!(
        (ledger.app_server, ledger.exec_initial, ledger.exec_resumed),
        (2, 1, 1)
    );

    std::fs::write(scratch.transitions.join("unknown"), b"unknown").unwrap();
    assert!(
        aggregate_worker_transitions(
            &scratch.transitions,
            &thread_ids,
            &thread_ids,
            &source,
            &permissions,
            &owned_probe_summary(),
        )
        .is_err()
    );
    let metadata = std::fs::symlink_metadata(&outer).unwrap();
    remove_entry_no_follow(&outer, &metadata).unwrap();

    let no_resume = [
        receipt(
            WorkerTransitionKind::AppServerPassThrough,
            WorkerTransitionStage::Launched,
            15,
            None,
        ),
        receipt(
            WorkerTransitionKind::ExecInitialAttenuated,
            WorkerTransitionStage::Started,
            16,
            Some("thread-1"),
        ),
    ];
    let no_resume_ledger = render_worker_transition_ledger(
        &no_resume,
        &thread_ids,
        &thread_ids,
        &source,
        &permissions,
        &owned_probe_summary(),
    )
    .unwrap();
    assert_eq!(
        (
            no_resume_ledger.app_server,
            no_resume_ledger.exec_initial,
            no_resume_ledger.exec_resumed,
        ),
        (1, 1, 0)
    );
    let verified_no_resume =
        verify_worker_transition_ledger(&no_resume_ledger.bytes, &source, &permissions).unwrap();
    assert_eq!(verified_no_resume.exec_resumed, 0);

    let no_initial = [receipt(
        WorkerTransitionKind::AppServerPassThrough,
        WorkerTransitionStage::Launched,
        17,
        None,
    )];
    assert!(
        render_worker_transition_ledger(
            &no_initial,
            &thread_ids,
            &thread_ids,
            &source,
            &permissions,
            &owned_probe_summary(),
        )
        .is_err()
    );
}
