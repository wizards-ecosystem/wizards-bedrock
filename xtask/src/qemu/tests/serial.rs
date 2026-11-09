use super::*;

#[test]
fn a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal() {
    let mut observation = BootSerialObservation::default();
    let panicked = AtomicBool::new(false);
    let diagnosis = format!("[warn ] {NO_X2APIC_DIAGNOSIS}\r\n");
    observe_boot_line(&mut observation, &diagnosis)
        .unwrap()
        .publish(&panicked);
    observe_boot_line(
        &mut observation,
        "[error] KERNEL PANIC on thread 'boot': exact cause\\nat source.rs:1 \t\r\n",
    )
    .unwrap()
    .publish(&panicked);

    assert!(observation.no_x2apic_before_panic);
    assert!(!observation.panic_complete);
    assert!(!panicked.load(Ordering::Acquire));
    assert_eq!(
        observation.panic.as_deref(),
        Some("[error] KERNEL PANIC on thread 'boot': exact cause\\nat source.rs:1 \t")
    );
    assert!(completed_panic(observation.panic.clone(), observation.panic_complete).is_err());

    assert!(observe_boot_line(&mut observation, "continuation\r\n").is_err());
    assert!(
        observe_boot_line(
            &mut observation,
            "[error] KERNEL PANIC on thread 'second': duplicate\r\n"
        )
        .is_err()
    );

    for near_miss in [
        format!("[error] {COMPLETE_MARKER}"),
        format!("[warn ] {COMPLETE_MARKER}\r\n"),
        format!("[error] {COMPLETE_MARKER} trailing\r\n"),
        format!("[error] {COMPLETE_MARKER}\r\r\n"),
    ] {
        assert!(!is_complete_panic_record(&near_miss), "{near_miss:?}");
    }

    let terminal = format!("[error] {COMPLETE_MARKER}\r\n");
    let mut unopened = BootSerialObservation::default();
    assert!(observe_boot_line(&mut unopened, &terminal).is_err());
    let transition = observe_boot_line(&mut observation, &terminal).unwrap();
    assert_eq!(transition, PanicTransition::Completed);
    transition.publish(&panicked);
    assert!(observation.panic_complete);
    assert!(panicked.load(Ordering::Acquire));
    assert_eq!(
        completed_panic(observation.panic.clone(), observation.panic_complete).unwrap(),
        Some("[error] KERNEL PANIC on thread 'boot': exact cause\\nat source.rs:1 \t".to_string())
    );
    assert!(observe_boot_line(&mut observation, &terminal).is_err());
    assert!(observe_boot_line(&mut observation, "after terminal\r\n").is_err());

    let mut simultaneous = BootSerialObservation::default();
    observe_boot_line(
        &mut simultaneous,
        "[error] KERNEL PANIC on thread 'boot': no x2apic mode is not a prior diagnosis\r\n",
    )
    .unwrap();
    assert!(!simultaneous.no_x2apic_before_panic);

    for near_miss in [
        format!("[info ] {NO_X2APIC_DIAGNOSIS}\r\n"),
        format!("[warn ] prefix {NO_X2APIC_DIAGNOSIS}\r\n"),
        format!("[warn ] {NO_X2APIC_DIAGNOSIS} trailing\r\n"),
        "[warn ] no x2apic mode\r\n".to_string(),
        format!("[warn ] {NO_X2APIC_DIAGNOSIS}"),
    ] {
        let mut rejected = BootSerialObservation::default();
        observe_boot_line(&mut rejected, &near_miss).unwrap();
        assert!(!rejected.no_x2apic_before_panic, "accepted {near_miss:?}");
    }

    for near_miss in [
        "KERNEL PANIC on thread 'boot': missing severity\r\n",
        "[warn ] KERNEL PANIC on thread 'boot': wrong severity\r\n",
        "prefix [error] KERNEL PANIC on thread 'boot': not at column zero\r\n",
        "[error] KERNEL PANIC: missing thread prefix\r\n",
    ] {
        let mut rejected = BootSerialObservation::default();
        assert_eq!(
            observe_boot_line(&mut rejected, near_miss).unwrap(),
            PanicTransition::Unchanged
        );
        assert!(rejected.panic.is_none(), "accepted {near_miss:?}");
    }

    let mut unterminated = BootSerialObservation::default();
    assert!(
        observe_boot_line(
            &mut unterminated,
            "[error] KERNEL PANIC on thread 'boot': missing newline"
        )
        .is_err()
    );
    assert!(unterminated.panic.is_none());

    for malformed in [
        "[error] KERNEL PANIC on thread 'boot': internal\rreturn\r\n",
        "[error] KERNEL PANIC on thread 'boot': doubled ending\r\r\n",
    ] {
        let mut rejected = BootSerialObservation::default();
        assert!(observe_boot_line(&mut rejected, malformed).is_err());
        assert!(rejected.panic.is_none(), "accepted {malformed:?}");
    }
}

#[test]
fn the_degraded_no_x2apic_requirement_uses_the_exact_warning_record() {
    let required = NO_X2APIC_RECORD;
    assert!(DEGRADED_REQUIRED_MARKERS.contains(&required));
    assert!(
        !DEGRADED_REQUIRED_MARKERS
            .iter()
            .any(|record| record.message == "no x2apic mode")
    );

    let exact = format!("[warn ] {NO_X2APIC_DIAGNOSIS}\r\n");
    assert!(is_required_boot_marker(&exact, required));
    for near_miss in [
        format!("[info ] {NO_X2APIC_DIAGNOSIS}\r\n"),
        format!("[warn ] prefix {NO_X2APIC_DIAGNOSIS}\r\n"),
        format!("[warn ] {NO_X2APIC_DIAGNOSIS} trailing\r\n"),
        "[warn ] no x2apic mode\r\n".to_string(),
        format!("[warn ] {NO_X2APIC_DIAGNOSIS}"),
    ] {
        assert!(
            !is_required_boot_marker(&near_miss, required),
            "accepted {near_miss:?}"
        );
    }
}

#[test]
fn required_boot_records_refuse_framing_and_message_near_misses() {
    for record in SHIPPED_REQUIRED_MARKERS
        .iter()
        .chain(DEGRADED_REQUIRED_MARKERS)
        .copied()
    {
        for ending in ["\n", "\r\n"] {
            let exact = format!("{}{}{ending}", record.severity.prefix(), record.message);
            assert!(
                is_required_boot_marker(&exact, record),
                "rejected {exact:?}"
            );
        }

        let wrong_severity = if record.severity == LogSeverity::Info {
            LogSeverity::Warning
        } else {
            LogSeverity::Info
        };
        for near_miss in [
            format!("{}{}\r\n", wrong_severity.prefix(), record.message),
            format!("prefix {}{}\r\n", record.severity.prefix(), record.message),
            format!("{}{} suffix\r\n", record.severity.prefix(), record.message),
            format!("{}{} \r\n", record.severity.prefix(), record.message),
            format!("{}{}\t\r\n", record.severity.prefix(), record.message),
            format!("{}{}\u{a0}\r\n", record.severity.prefix(), record.message),
            format!("{}{}", record.severity.prefix(), record.message),
            format!("{}{}\r\r\n", record.severity.prefix(), record.message),
            format!("{}{}\rinside\r\n", record.severity.prefix(), record.message),
        ] {
            assert!(
                !is_required_boot_marker(&near_miss, record),
                "accepted {near_miss:?} for {record:?}"
            );
        }
    }
}

#[test]
fn failure_records_cannot_satisfy_required_boot_evidence() {
    let cases = [
        (
            RequiredLogRecord::info("cpu: user isolation shared kernel mappings: qemu_tcg"),
            "[warn ] cpu: user isolation shared kernel mappings: refused; ring 3 stays closed\r\n",
        ),
        (
            RequiredLogRecord::info("nvme: candidate 1 retired cleanly; trying next"),
            "[warn ] nvme: candidate 1 left uncertain controller state after reset\r\n",
        ),
        (
            RequiredLogRecord::warning(
                "local apic: this processor has no x2apic mode, and xapic needs page tables; interrupts stay off",
            ),
            "[warn ] local apic: no x2apic mode\r\n",
        ),
        (
            RequiredLogRecord::warning("pci: the configuration ports are already claimed; no bus"),
            "[warn ] pci: no storage controllers found\r\n",
        ),
        (
            RequiredLogRecord::warning("storage: no DMA isolation"),
            "[info ] storage: no DMA isolation\r\n",
        ),
        (
            RequiredLogRecord::info("tmpfs: 1 file(s) and 1 directory(ies) at the root"),
            "[warn ] tmpfs: the root would not list: NotFound\r\n",
        ),
        (
            RequiredLogRecord::info("devfs: storage is a block device"),
            "[warn ] devfs: a device would not resolve: NotFound\r\n",
        ),
        (
            RequiredLogRecord::info("devfs: console is a character device"),
            "[warn ] devfs: a device resolved to something with bytes in it\r\n",
        ),
        (
            RequiredLogRecord::info("cpu: protections wp nx"),
            "[info ] cpu: protections wp smep nx\r\n",
        ),
        (
            RequiredLogRecord::warning("cpu: extended state unavailable; ring 3 stays closed"),
            "[info ] cpu: extended state 832 bytes, xcr0 0x7, compacted\r\n",
        ),
        (
            RequiredLogRecord::warning(
                "user: no program: maskable interrupts are not admitted; ring 3 is closed",
            ),
            "[info ] user: program running in slot 0\r\n",
        ),
    ];

    for (record, failure) in cases {
        assert!(
            !is_required_boot_marker(failure, record),
            "accepted failure producer {failure:?} for {record:?}"
        );
    }
}

#[test]
fn steady_state_requires_one_exact_dynamic_info_record() {
    for exact in [
        "[info ] kernel: reading the keyboard, uptime 0 ms\n",
        "[info ] kernel: reading the keyboard, uptime 1 ms\r\n",
        "[info ] kernel: reading the keyboard, uptime 18446744073709551615 ms\r\n",
    ] {
        assert!(is_settled_record(exact), "rejected {exact:?}");
    }

    for near_miss in [
        "[warn ] kernel: reading the keyboard, uptime 1 ms\r\n",
        "prefix [info ] kernel: reading the keyboard, uptime 1 ms\r\n",
        "[info ] prefix kernel: reading the keyboard, uptime 1 ms\r\n",
        "[info ] kernel: reading the keyboard, uptime 1 ms trailing\r\n",
        "[info ] kernel: reading the keyboard, uptime 1 ms",
        "[info ] kernel: reading the keyboard, uptime 1 ms \r\n",
        "[info ] kernel: reading the keyboard, uptime 1 ms\t\r\n",
        "[info ] kernel: reading the keyboard, uptime 1 ms\u{a0}\r\n",
        "[info ] kernel: reading the keyboard, uptime 01 ms\r\n",
        "[info ] kernel: reading the keyboard, uptime +1 ms\r\n",
        "[info ] kernel: reading the keyboard, uptime -1 ms\r\n",
        "[info ] kernel: reading the keyboard, uptime \u{0661} ms\r\n",
        "[info ] kernel: reading the keyboard, uptime 18446744073709551616 ms\r\n",
        "[info ] kernel: reading the keyboard, uptime  ms\r\n",
        "[info ] kernel: reading the keyboard, uptime 1\r\n",
        "[info ] kernel: reading the keyboard, uptime 1\r ms\r\n",
        "[info ] kernel: entering idle loop\r\n",
    ] {
        assert!(!is_settled_record(near_miss), "accepted {near_miss:?}");
    }
}

#[test]
fn interactive_input_requires_exact_terminated_info_records() {
    for record in INTERACTIVE_MARKERS {
        let exact = format!("[info ] {}\r\n", record.message);
        let mut next = INTERACTIVE_MARKERS
            .iter()
            .position(|candidate| candidate == record)
            .unwrap();
        assert!(
            observe_interactive_marker(&exact, &mut next, InteractiveMarkerOrder::Strict).unwrap()
        );

        for near_miss in [
            format!("[warn ] {}\r\n", record.message),
            format!("[error] {}\r\n", record.message),
            format!("prefix [info ] {}\r\n", record.message),
            format!("[info ] prefix {}\r\n", record.message),
            format!("[info ] {} suffix\r\n", record.message),
            format!("[info ] {} \r\n", record.message),
            format!("[info ] {}\t\r\n", record.message),
            format!("[info ] {}\u{a0}\r\n", record.message),
            format!("[info ] {}", record.message),
            format!("[info ] {}\r\r\n", record.message),
        ] {
            let before = next;
            assert!(
                !observe_interactive_marker(&near_miss, &mut next, InteractiveMarkerOrder::Strict,)
                    .unwrap(),
                "accepted {near_miss:?}"
            );
            assert_eq!(next, before);
        }
    }
}

#[test]
fn interactive_input_markers_are_unique_and_ordered() {
    let records: Vec<String> = INTERACTIVE_MARKERS
        .iter()
        .map(|record| format!("[info ] {}\r\n", record.message))
        .collect();
    let mut next = 0;
    assert!(
        observe_interactive_marker(&records[1], &mut next, InteractiveMarkerOrder::Strict,)
            .is_err()
    );
    assert_eq!(next, 0);
    assert!(
        observe_interactive_marker(&records[0], &mut next, InteractiveMarkerOrder::Strict,)
            .unwrap()
    );
    assert_eq!(next, 1);
    assert!(
        observe_interactive_marker(&records[0], &mut next, InteractiveMarkerOrder::Strict,)
            .is_err()
    );
    assert_eq!(next, 1);
    assert!(
        observe_interactive_marker(&records[1], &mut next, InteractiveMarkerOrder::Strict,)
            .unwrap()
    );
    assert!(
        observe_interactive_marker(&records[2], &mut next, InteractiveMarkerOrder::Strict,)
            .unwrap()
    );
    assert_eq!(next, INTERACTIVE_MARKERS.len());
    assert!(
        !observe_interactive_marker(
            "[info ] unrelated\r\n",
            &mut next,
            InteractiveMarkerOrder::Strict,
        )
        .unwrap()
    );

    let mut mutation_next = 0;
    assert!(
        observe_interactive_marker(
            &records[1],
            &mut mutation_next,
            InteractiveMarkerOrder::Mutation,
        )
        .unwrap()
    );
    assert_eq!(mutation_next, 2);
    assert!(
        observe_interactive_marker(
            &records[0],
            &mut mutation_next,
            InteractiveMarkerOrder::Mutation,
        )
        .is_err()
    );
    assert!(
        observe_interactive_marker(
            &records[2],
            &mut mutation_next,
            InteractiveMarkerOrder::Mutation,
        )
        .unwrap()
    );
    assert_eq!(injection_action(0, 1), InjectionAction::Wait);
    assert_eq!(injection_action(1, 1), InjectionAction::Inject);
    assert_eq!(injection_action(2, 1), InjectionAction::Skip);
}

#[test]
fn automated_boots_require_the_exact_producer_records() {
    assert_eq!(
        INTERACTIVE_MARKERS,
        &[
            RequiredLogRecord::info("selftest: awaiting storage-delay keystroke"),
            RequiredLogRecord::info("selftest: awaiting injected keystroke"),
            RequiredLogRecord::info("selftest: awaiting translated keystroke"),
        ]
    );
    assert_eq!(
        SHIPPED_REQUIRED_MARKERS,
        &[
            RequiredLogRecord::info("iommu: entry DMA fence policy strict"),
            RequiredLogRecord::info("cpu: user isolation shared kernel mappings: qemu_tcg"),
            RequiredLogRecord::info("nvme: candidate 1 retired cleanly; trying next"),
            RequiredLogRecord::info(
                "keyboard: PCI interrupt isolation keeps every io-apic line masked; polling",
            ),
            RequiredLogRecord::info("keyboard: polled every 10 ms"),
            RequiredLogRecord::info("user: ring 3 call observed"),
        ]
    );
    assert_eq!(
        DEGRADED_REQUIRED_MARKERS,
        &[
            RequiredLogRecord::info("iommu: entry DMA fence policy strict"),
            RequiredLogRecord::info("cpu: user isolation shared kernel mappings: qemu_tcg"),
            RequiredLogRecord::warning(
                "local apic: this processor has no x2apic mode, and xapic needs page tables; interrupts stay off"
            ),
            RequiredLogRecord::info("tmpfs: 1 file(s) and 1 directory(ies) at the root"),
            RequiredLogRecord::info("devfs: storage is a block device"),
            RequiredLogRecord::info("devfs: console is a character device"),
            RequiredLogRecord::info("cpu: protections wp nx"),
            RequiredLogRecord::warning("cpu: extended state unavailable; ring 3 stays closed"),
            RequiredLogRecord::warning(
                "user: no program: maskable interrupts are not admitted; ring 3 is closed"
            ),
        ]
    );
    assert_eq!(
        UNISOLATED_REQUIRED_MARKERS,
        &[
            RequiredLogRecord::info("iommu: entry DMA fence policy strict"),
            RequiredLogRecord::info("cpu: user isolation shared kernel mappings: qemu_tcg"),
            RequiredLogRecord::warning("storage: no DMA isolation"),
            RequiredLogRecord::info("nvme: candidate 1 retired cleanly; trying next"),
            RequiredLogRecord::info("nvme: no usable controller"),
        ]
    );
    assert_eq!(
        NO_PCI_HANDOFF_PANIC,
        "DMA-safe boot handoff refused: DMA handoff fence: PCI configuration is unavailable"
    );
}

#[test]
fn automated_serial_uses_a_private_backpressured_transport() {
    let fixture = QemuFixture::create();
    let first = SerialChannel::create(&fixture.root).unwrap();
    let second = SerialChannel::create(&fixture.root).unwrap();
    let arguments = first.arguments();
    let backend = arguments[1].to_string_lossy();
    assert_eq!(arguments[0], "-chardev");
    assert_eq!(arguments[2], "-serial");
    assert_eq!(arguments[3], "chardev:serial0");
    assert!(!backend.starts_with("file,"), "serial regained a disk sink");
    #[cfg(windows)]
    {
        assert!(backend.starts_with("pipe,id=serial0,path=wizard-bedrock-serial-"));
        assert_ne!(first.name, second.name, "serial pipe names repeated");
    }
    #[cfg(unix)]
    {
        assert!(backend.starts_with("socket,id=serial0,path="));
        assert_ne!(first.socket, second.socket, "serial socket names repeated");
    }
}

#[test]
fn a_serial_reader_reads_every_piped_byte_and_ends_after_close() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let closed = Arc::new(AtomicBool::new(false));
    let mut follower = SerialReader {
        inner: Box::new(reader),
        closed: Arc::clone(&closed),
    };
    let writer = std::thread::spawn(move || {
        for chunk in [b"first ".as_slice(), b"second\n", b"late tail"] {
            writer.write_all(chunk).unwrap();
            writer.flush().unwrap();
            std::thread::sleep(Duration::from_millis(40));
        }
    });
    let reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        follower.read_to_end(&mut text).unwrap();
        text
    });
    writer.join().unwrap();
    std::thread::sleep(Duration::from_millis(60));
    assert!(
        !reader.is_finished(),
        "the follower ended before QEMU exited"
    );
    closed.store(true, Ordering::Release);
    assert_eq!(reader.join().unwrap(), b"first second\nlate tail");
}

#[test]
fn automated_serial_capture_bounds_each_record_and_the_whole_run() {
    let mut reader = BufReader::new(std::io::Cursor::new(b"abc\ndef\n"));
    let mut line = Vec::new();
    let mut total = 0;
    assert_eq!(
        read_serial_line_with_limits(&mut reader, &mut line, &mut total, 4, 8).unwrap(),
        4
    );
    assert_eq!(line, b"abc\n");
    assert_eq!(
        read_serial_line_with_limits(&mut reader, &mut line, &mut total, 4, 8).unwrap(),
        4
    );
    assert_eq!(total, 8);

    let mut long = BufReader::new(std::io::Cursor::new(b"abcd\n"));
    let error = read_serial_line_with_limits(&mut long, &mut line, &mut 0, 4, 32)
        .expect_err("oversized record");
    assert!(error.to_string().contains("serial line"));

    let mut aggregate = BufReader::new(std::io::Cursor::new(b"a\nb\n"));
    let mut total = 0;
    read_serial_line_with_limits(&mut aggregate, &mut line, &mut total, 8, 3).unwrap();
    let error = read_serial_line_with_limits(&mut aggregate, &mut line, &mut total, 8, 3)
        .expect_err("aggregate overrun");
    assert!(error.to_string().contains("transcript"));
}

#[test]
fn automated_serial_display_escapes_guest_terminal_controls() {
    assert_eq!(
        escaped_serial_line(b"ok\x1b[2J\r\x08\0\x80\n"),
        r"ok\x1b[2J\x0d\x08\x00\x80"
    );
    assert_eq!(escaped_serial_line(b"plain ASCII\n"), "plain ASCII");
}

#[test]
fn a_serial_reader_refuses_an_early_transport_close() {
    let (reader, writer) = std::io::pipe().unwrap();
    let mut follower = SerialReader {
        inner: Box::new(reader),
        closed: Arc::new(AtomicBool::new(false)),
    };
    drop(writer);
    let error = follower
        .read(&mut [0u8; 1])
        .expect_err("premature serial close");
    assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
}

fn passing_verdict() -> SelftestVerdict {
    SelftestVerdict {
        status: VerdictStatus::Pass,
        passed: EXPECTED_SELFTEST_CHECKS,
        failed: 0,
        expected: EXPECTED_SELFTEST_CHECKS,
    }
}

fn passing_stack_sample() -> StackSample {
    StackSample {
        used: 22_000,
        capacity: NVME_STACK_CAPACITY,
        headroom: NVME_STACK_CAPACITY - 22_000,
        leaf: 21_000,
    }
}

fn stack_line() -> &'static str {
    "[info ] selftest: nvme-stack v2 used=22000 capacity=32768 headroom=10768 leaf=21000\r\n"
}

#[test]
fn a_failed_check_requires_one_exact_terminated_error_record() {
    for exact in [
        "[error] selftest: a bound space is not reclaimed FAILED\n",
        "[error] selftest: a bound space is not reclaimed FAILED\r\n",
    ] {
        assert_eq!(
            parse_failed_check(exact),
            Some("a bound space is not reclaimed")
        );
    }
    for near_miss in [
        "[info ] selftest: a thing FAILED\r\n",
        "[warn ] selftest: a thing FAILED\r\n",
        "prefix [error] selftest: a thing FAILED\r\n",
        "[error] prefix selftest: a thing FAILED\r\n",
        "[error] selftest: a thing FAILED suffix\r\n",
        "[error] selftest: a thing FAILED \r\n",
        "[error] selftest: a thing FAILED\t\r\n",
        "[error] selftest: a thing FAILED\u{a0}\r\n",
        "[error] selftest: a thing FAILED",
        "[error] selftest: a thing FAILED\r\r\n",
        "[error] selftest: a thing FAILED\rinside\r\n",
        "[error] selftest:  FAILED\r\n",
        "[error] selftest: verdict v1 FAIL passed=1 failed=0 expected=1\r\n",
        "[info ] selftest: a thing\r\n",
        "[info ] keyboard: up\r\n",
    ] {
        assert_eq!(
            parse_failed_check(near_miss),
            None,
            "accepted {near_miss:?}"
        );
    }
}

#[test]
fn verdict_records_require_exact_framing_and_status_severity() {
    let pass = format!(
        "[info ] selftest: verdict v1 PASS passed={} failed=0 expected={}\r\n",
        EXPECTED_SELFTEST_CHECKS, EXPECTED_SELFTEST_CHECKS
    );
    assert_eq!(
        parse_selftest_verdict(&pass).unwrap().unwrap(),
        passing_verdict()
    );
    let fail = format!(
        "[error] selftest: verdict v1 FAIL passed={} failed=1 expected={}\n",
        EXPECTED_SELFTEST_CHECKS - 1,
        EXPECTED_SELFTEST_CHECKS
    );
    assert_eq!(
        parse_selftest_verdict(&fail).unwrap().unwrap(),
        SelftestVerdict {
            status: VerdictStatus::Fail,
            passed: EXPECTED_SELFTEST_CHECKS - 1,
            failed: 1,
            expected: EXPECTED_SELFTEST_CHECKS,
        }
    );

    for near_miss in [
        pass.trim_start_matches("[info ] ").to_string(),
        pass.replacen("[info ]", "[warn ]", 1),
        format!("prefix {pass}"),
        pass.replacen("[info ] selftest:", "[info ] prefix selftest:", 1),
        pass.trim_end_matches(['\r', '\n']).to_string(),
        pass.replace("\r\n", "\r\r\n"),
        pass.replacen("[info ]", "[error]", 1),
        fail.replacen("[error]", "[info ]", 1),
    ] {
        assert!(
            !matches!(parse_selftest_verdict(&near_miss), Some(Ok(_))),
            "accepted {near_miss:?}"
        );
    }
}

#[test]
fn verdict_fields_require_single_ascii_spaces() {
    let exact = "[info ] selftest: verdict v1 PASS passed=1 failed=0 expected=1\r\n";
    assert!(matches!(parse_selftest_verdict(exact), Some(Ok(_))));
    for near_miss in [
        exact.replace("v1 PASS", "v1  PASS"),
        exact.replace("v1 PASS", "v1\tPASS"),
        exact.replace("v1 PASS", "v1\u{a0}PASS"),
        exact.replace(" expected=1", "  expected=1"),
        exact.replace("\r\n", " \r\n"),
        exact.replace("\r\n", "\t\r\n"),
        exact.replace("\r\n", "\u{a0}\r\n"),
    ] {
        assert!(
            !matches!(parse_selftest_verdict(&near_miss), Some(Ok(_))),
            "accepted {near_miss:?}"
        );
    }
}

#[test]
fn verdict_counts_use_canonical_decimals() {
    for near_miss in [
        "[info ] selftest: verdict v1 PASS passed=01 failed=0 expected=1\r\n",
        "[info ] selftest: verdict v1 PASS passed=+1 failed=0 expected=1\r\n",
        "[info ] selftest: verdict v1 PASS passed=-1 failed=0 expected=1\r\n",
        "[info ] selftest: verdict v1 PASS passed=\u{0661} failed=0 expected=1\r\n",
        "[info ] selftest: verdict v1 PASS passed=18446744073709551616 failed=0 expected=1\r\n",
    ] {
        assert!(
            matches!(parse_selftest_verdict(near_miss), Some(Err(_))),
            "accepted {near_miss:?}"
        );
    }
}

fn baseline_line() -> &'static str {
    "[info ] selftest: baseline v1 boot_ns=11 frames_free=12 heap_slab=13 heap_large=14 heap_live=15 handles_committed=16 handles_live=17 trap_ticks=18 storage_bytes=4096 storage_ticks=19 counter_hz=20\r\n"
}

#[test]
fn baseline_record_is_versioned_ordered_and_unique() {
    assert_eq!(
        parse_performance_sample(baseline_line()).unwrap().unwrap(),
        PerformanceSample {
            boot_ns: 11,
            frames_free: 12,
            heap_slab: 13,
            heap_large: 14,
            heap_live: 15,
            handles_committed: 16,
            handles_live: 17,
            trap_ticks: 18,
            storage_bytes: 4096,
            storage_ticks: 19,
            counter_hz: 20,
        }
    );
    for line in [
        baseline_line().replace("v1", "v2"),
        baseline_line().replace("boot_ns=11 frames_free=12", "frames_free=12 boot_ns=11"),
        baseline_line().replace(" counter_hz=20", ""),
        baseline_line().replace("\r\n", " trailing=true\r\n"),
    ] {
        assert!(parse_performance_sample(&line).unwrap().is_err(), "{line}");
    }

    let mut observation = SerialObservation::default();
    observe_selftest_line(&mut observation, baseline_line()).unwrap();
    assert!(observe_selftest_line(&mut observation, baseline_line()).is_err());
}

#[test]
fn baseline_records_require_exact_info_framing() {
    for near_miss in [
        baseline_line().trim_start_matches("[info ] ").to_string(),
        baseline_line().replacen("[info ]", "[warn ]", 1),
        baseline_line().replacen("[info ]", "[error]", 1),
        format!("prefix {}", baseline_line()),
        baseline_line().replacen("selftest:", "prefix selftest:", 1),
        baseline_line().trim_end_matches(['\r', '\n']).to_string(),
        baseline_line().replace("\r\n", "\r\r\n"),
    ] {
        assert!(
            !matches!(parse_performance_sample(&near_miss), Some(Ok(_))),
            "accepted {near_miss:?}"
        );
    }
}

#[test]
fn baseline_fields_require_single_ascii_spaces() {
    for near_miss in [
        baseline_line().replace("v1 boot_ns", "v1  boot_ns"),
        baseline_line().replace("v1 boot_ns", "v1\tboot_ns"),
        baseline_line().replace("v1 boot_ns", "v1\u{a0}boot_ns"),
        baseline_line().replace(" frames_free", "  frames_free"),
        baseline_line().replace("\r\n", " \r\n"),
        baseline_line().replace("\r\n", "\t\r\n"),
        baseline_line().replace("\r\n", "\u{a0}\r\n"),
    ] {
        assert!(
            matches!(parse_performance_sample(&near_miss), Some(Err(_))),
            "accepted {near_miss:?}"
        );
    }
}

#[test]
fn baseline_values_use_canonical_decimals() {
    for near_miss in [
        baseline_line().replace("boot_ns=11", "boot_ns=011"),
        baseline_line().replace("boot_ns=11", "boot_ns=+11"),
        baseline_line().replace("boot_ns=11", "boot_ns=-11"),
        baseline_line().replace("boot_ns=11", "boot_ns=\u{0661}\u{0661}"),
        baseline_line().replace("boot_ns=11", "boot_ns=18446744073709551616"),
    ] {
        assert!(
            matches!(parse_performance_sample(&near_miss), Some(Err(_))),
            "accepted {near_miss:?}"
        );
    }
}

#[test]
fn nvme_stack_record_is_versioned_ordered_canonical_and_unique() {
    assert_eq!(
        parse_stack_sample(stack_line()).unwrap().unwrap(),
        passing_stack_sample()
    );
    for line in [
        stack_line().replace("v2", "v1"),
        stack_line().replace(" leaf=21000", ""),
        stack_line().replace("leaf=21000", "leaf=021000"),
        stack_line().replace("headroom=10768 leaf=21000", "leaf=21000 headroom=10768"),
        stack_line().replace("used=22000 capacity=32768", "capacity=32768 used=22000"),
        stack_line().replace("used=22000", "used=022000"),
        stack_line().replace(" headroom=10768", ""),
        stack_line().replace("\r\n", " trailing=true\r\n"),
        stack_line().replacen("[info ]", "[warn ]", 1),
        stack_line().trim_end_matches(['\r', '\n']).to_string(),
    ] {
        assert!(
            !matches!(parse_stack_sample(&line), Some(Ok(_))),
            "accepted {line:?}"
        );
    }

    let mut observation = SerialObservation::default();
    observe_selftest_line(&mut observation, stack_line()).unwrap();
    assert!(observe_selftest_line(&mut observation, stack_line()).is_err());
}

#[test]
fn a_panic_outcome_preserves_earlier_failures_and_its_exact_transcript() {
    let failed = "[error] selftest: an earlier ownership check FAILED\r\n";
    let line = "[error] KERNEL PANIC on thread 'boot': panicked at source\\nexact cause \t\r\n";
    let mut observation = SerialObservation::default();
    let panicked = AtomicBool::new(false);
    observe_selftest_line(&mut observation, failed)
        .unwrap()
        .publish(&panicked);
    observe_selftest_line(&mut observation, line)
        .unwrap()
        .publish(&panicked);
    assert!(!observation.panic_complete);
    assert!(!panicked.load(Ordering::Acquire));
    assert!(correlate_selftest(None, observation.clone()).is_err());
    assert!(observe_selftest_line(&mut observation, "continuation\r\n").is_err());

    let terminal = format!("[error] {COMPLETE_MARKER}\r\n");
    let transition = observe_selftest_line(&mut observation, &terminal).unwrap();
    assert_eq!(transition, PanicTransition::Completed);
    transition.publish(&panicked);
    assert!(panicked.load(Ordering::Acquire));
    assert_eq!(
        correlate_selftest(None, observation).unwrap(),
        SelftestOutcome::Panicked {
            transcript:
                "[error] KERNEL PANIC on thread 'boot': panicked at source\\nexact cause \t"
                    .to_string(),
            failed: vec!["an earlier ownership check".to_string()],
        }
    );
}

#[test]
fn a_failed_verdict_must_name_every_failed_check() {
    let observation = SerialObservation {
        verdict: Some(SelftestVerdict {
            status: VerdictStatus::Fail,
            passed: EXPECTED_SELFTEST_CHECKS - 1,
            failed: 1,
            expected: EXPECTED_SELFTEST_CHECKS,
        }),
        baseline: None,
        nvme_stack: None,
        qemu_tcg_isolation: true,
        strict_dma_handoff: true,
        failed: Vec::new(),
        panic: None,
        panic_complete: false,
    };
    let error = correlate_selftest(Some(SELFTEST_FAILED), observation).unwrap_err();
    assert!(error.to_string().contains("counted 1 failures but named 0"));
}

#[test]
fn malformed_or_unknown_verdicts_are_rejected() {
    for line in [
        "[info ] selftest: verdict v2 PASS passed=76 failed=0 expected=76\r\n",
        "[info ] selftest: verdict v1 MAYBE passed=76 failed=0 expected=76\r\n",
        "[info ] selftest: verdict v1 PASS passed=many failed=0 expected=76\r\n",
        "[info ] selftest: verdict v1 PASS passed=76 failed=0 expected=76 trailing\r\n",
    ] {
        assert!(parse_selftest_verdict(line).unwrap().is_err(), "{line}");
    }
}

#[test]
fn a_second_final_verdict_is_rejected() {
    let line = format!(
        "[info ] selftest: verdict v1 PASS passed={} failed=0 expected={}\r\n",
        EXPECTED_SELFTEST_CHECKS, EXPECTED_SELFTEST_CHECKS
    );
    let mut observation = SerialObservation::default();
    observe_selftest_line(&mut observation, &line).unwrap();
    assert!(observe_selftest_line(&mut observation, &line).is_err());
}

#[test]
fn failed_check_names_are_unique_and_precede_the_terminal_verdict() {
    let failed = "[error] selftest: exact failed check FAILED\r\n";
    let mut duplicate = SerialObservation::default();
    observe_selftest_line(&mut duplicate, failed).unwrap();
    assert!(observe_selftest_line(&mut duplicate, failed).is_err());

    let verdict = format!(
        "[info ] selftest: verdict v1 PASS passed={} failed=0 expected={}\r\n",
        EXPECTED_SELFTEST_CHECKS, EXPECTED_SELFTEST_CHECKS
    );
    let mut terminal = SerialObservation::default();
    observe_selftest_line(&mut terminal, &verdict).unwrap();
    assert!(observe_selftest_line(&mut terminal, failed).is_err());
    assert!(observe_selftest_line(&mut terminal, "[info ] after verdict\r\n").is_err());
}

#[test]
fn a_valid_serial_pass_must_match_debug_exit() {
    let observation = SerialObservation {
        failed: Vec::new(),
        verdict: Some(passing_verdict()),
        baseline: None,
        nvme_stack: Some(passing_stack_sample()),
        qemu_tcg_isolation: true,
        strict_dma_handoff: true,
        panic: None,
        panic_complete: false,
    };
    assert!(correlate_selftest(Some(SELFTEST_PASSED), observation).is_ok());

    let observation = SerialObservation {
        failed: Vec::new(),
        verdict: Some(passing_verdict()),
        baseline: None,
        nvme_stack: Some(passing_stack_sample()),
        qemu_tcg_isolation: true,
        strict_dma_handoff: true,
        panic: None,
        panic_complete: false,
    };
    let error = correlate_selftest(Some(SELFTEST_FAILED), observation).unwrap_err();
    assert!(error.to_string().contains("debug-exit"));
}

#[test]
fn a_passing_selftest_requires_bounded_conservative_nvme_stack_evidence() {
    for (sample, expected) in [
        (None, "without an NVMe stack record"),
        (
            Some(StackSample {
                capacity: NVME_STACK_CAPACITY - 1,
                ..passing_stack_sample()
            }),
            "expected 32768",
        ),
        (
            Some(StackSample {
                headroom: passing_stack_sample().headroom - 1,
                ..passing_stack_sample()
            }),
            "does not conserve",
        ),
        (
            Some(StackSample {
                leaf: passing_stack_sample().used + 1,
                ..passing_stack_sample()
            }),
            "exceeds its painted high-water mark",
        ),
        (
            Some(StackSample {
                used: NVME_STACK_CAPACITY - NVME_STACK_MIN_HEADROOM + 1,
                headroom: NVME_STACK_MIN_HEADROOM - 1,
                ..passing_stack_sample()
            }),
            "retained only",
        ),
    ] {
        let observation = SerialObservation {
            failed: Vec::new(),
            verdict: Some(passing_verdict()),
            baseline: None,
            nvme_stack: sample,
            qemu_tcg_isolation: true,
            strict_dma_handoff: true,
            panic: None,
            panic_complete: false,
        };
        let error = correlate_selftest(Some(SELFTEST_PASSED), observation).unwrap_err();
        assert!(error.to_string().contains(expected), "{error:#}");
    }
}

#[test]
fn a_passing_selftest_requires_the_exact_consumed_tcg_policy() {
    let without_policy = SerialObservation {
        failed: Vec::new(),
        verdict: Some(passing_verdict()),
        baseline: None,
        nvme_stack: Some(passing_stack_sample()),
        qemu_tcg_isolation: false,
        strict_dma_handoff: true,
        panic: None,
        panic_complete: false,
    };
    let error = correlate_selftest(Some(SELFTEST_PASSED), without_policy).unwrap_err();
    assert!(error.to_string().contains("without consuming"));

    let without_dma_policy = SerialObservation {
        failed: Vec::new(),
        verdict: Some(passing_verdict()),
        baseline: None,
        nvme_stack: Some(passing_stack_sample()),
        qemu_tcg_isolation: true,
        strict_dma_handoff: false,
        panic: None,
        panic_complete: false,
    };
    let error = correlate_selftest(Some(SELFTEST_PASSED), without_dma_policy).unwrap_err();
    assert!(error.to_string().contains("strict QEMU DMA-handoff"));

    for near_miss in [
        SELFTEST_QEMU_TCG_ISOLATION_MARKER.to_string(),
        format!("[warn ] {SELFTEST_QEMU_TCG_ISOLATION_MARKER}"),
        format!("[info ] {SELFTEST_QEMU_TCG_ISOLATION_MARKER} trailing"),
        "[info ] cpu: user isolation paired roots: forced for self-test; shared proof retained: qemu-tcg"
            .to_string(),
    ] {
        assert!(!is_exact_log_message(
            &near_miss,
            LogSeverity::Info,
            SELFTEST_QEMU_TCG_ISOLATION_MARKER
        ));
    }

    for near_miss in [
        STRICT_DMA_HANDOFF_MARKER.to_string(),
        format!("[warn ] {STRICT_DMA_HANDOFF_MARKER}"),
        format!("[info ] {STRICT_DMA_HANDOFF_MARKER} trailing"),
    ] {
        assert!(!is_exact_log_message(
            &near_miss,
            LogSeverity::Info,
            STRICT_DMA_HANDOFF_MARKER
        ));
    }

    let exact = format!("[info ] {SELFTEST_QEMU_TCG_ISOLATION_MARKER}\r\n");
    let strict = format!("[info ] {STRICT_DMA_HANDOFF_MARKER}\r\n");
    let mut observation = SerialObservation::default();
    observe_selftest_line(&mut observation, &strict).unwrap();
    observe_selftest_line(&mut observation, &exact).unwrap();
    observe_selftest_line(&mut observation, stack_line()).unwrap();
    observation.verdict = Some(passing_verdict());
    assert_eq!(
        correlate_selftest(Some(SELFTEST_PASSED), observation).unwrap(),
        SelftestOutcome::Passed
    );

    let mut duplicate = SerialObservation::default();
    observe_selftest_line(&mut duplicate, &exact).unwrap();
    assert!(observe_selftest_line(&mut duplicate, &exact).is_err());
}

#[test]
fn zero_or_skipped_checks_cannot_pass() {
    for passed in [0, EXPECTED_SELFTEST_CHECKS - 1] {
        let observation = SerialObservation {
            failed: Vec::new(),
            verdict: Some(SelftestVerdict {
                status: VerdictStatus::Pass,
                passed,
                failed: 0,
                expected: EXPECTED_SELFTEST_CHECKS,
            }),
            baseline: None,
            nvme_stack: Some(passing_stack_sample()),
            qemu_tcg_isolation: true,
            strict_dma_handoff: true,
            panic: None,
            panic_complete: false,
        };
        let error = correlate_selftest(Some(SELFTEST_PASSED), observation).unwrap_err();
        assert!(error.to_string().contains("counted"));
    }
}

#[test]
fn harness_and_kernel_must_agree_on_the_expected_count() {
    let observation = SerialObservation {
        failed: Vec::new(),
        verdict: Some(SelftestVerdict {
            expected: EXPECTED_SELFTEST_CHECKS - 1,
            passed: EXPECTED_SELFTEST_CHECKS - 1,
            ..passing_verdict()
        }),
        baseline: None,
        nvme_stack: Some(passing_stack_sample()),
        qemu_tcg_isolation: true,
        strict_dma_handoff: true,
        panic: None,
        panic_complete: false,
    };
    let error = correlate_selftest(Some(SELFTEST_PASSED), observation).unwrap_err();
    assert!(error.to_string().contains("harness requires"));
}

#[test]
fn serial_status_must_agree_with_its_counts() {
    let observation = SerialObservation {
        failed: Vec::new(),
        verdict: Some(SelftestVerdict {
            status: VerdictStatus::Fail,
            ..passing_verdict()
        }),
        baseline: None,
        nvme_stack: Some(passing_stack_sample()),
        qemu_tcg_isolation: true,
        strict_dma_handoff: true,
        panic: None,
        panic_complete: false,
    };
    let error = correlate_selftest(Some(SELFTEST_FAILED), observation).unwrap_err();
    assert!(error.to_string().contains("contradicts its counts"));
}

#[test]
fn a_debug_exit_without_a_serial_verdict_is_rejected() {
    let error =
        correlate_selftest(Some(SELFTEST_PASSED), SerialObservation::default()).unwrap_err();
    assert!(error.to_string().contains("without a serial"));
}

#[test]
fn irq_statistics_sum_matching_controllers() {
    let report = "IRQ statistics for i8259:\r\n 0: 12\r\n 1: 2\r\n\
                      IRQ statistics for ioapic:\r\n IRQ1: 3\r\n 12: 99\r\n(qemu)";
    assert_eq!(irq_count(report, 1), Some(5));
}

#[test]
fn non_irq_monitor_output_is_not_a_zero_count() {
    assert_eq!(irq_count("unknown command: info irq\r\n(qemu)", 1), None);
}
