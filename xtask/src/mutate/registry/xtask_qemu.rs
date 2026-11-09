use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 394,
        file: "xtask/src/qemu/serial.rs",
        find: "    if panic.is_some() && !*panic_complete {\n        bail!(\"the kernel continued a single-line panic record\");\n    }",
        replace: "    if panic.is_some() && !*panic_complete {\n        return Ok(Some(PanicTransition::Unchanged));\n    }",
        breaks: "the single-line panic grammar accepts a continuation or second opening record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_panic_outcome_preserves_earlier_failures_and_its_exact_transcript",
        },
    },
    Mutation {
        id: 395,
        file: "xtask/src/qemu/serial.rs",
        find: "        if panic.is_none() {\n            bail!(\"the kernel completed a panic record it never started\");\n        }",
        replace: "        if false && panic.is_none() {\n            bail!(\"the kernel completed a panic record it never started\");\n        }",
        breaks: "a completion record is accepted without an opening record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        id: 396,
        file: "xtask/src/qemu/serial.rs",
        find: "        if *panic_complete {\n            bail!(\"the kernel completed its panic record more than once\");\n        }",
        replace: "        if false && *panic_complete {\n            bail!(\"the kernel completed its panic record more than once\");\n        }",
        breaks: "a panic record is allowed to publish completion more than once",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        id: 397,
        file: "xtask/src/qemu/serial.rs",
        find: "    if *panic_complete {\n        bail!(\"the kernel printed serial data after its terminal panic record\");\n    }",
        replace: "    if false && *panic_complete {\n        bail!(\"the kernel printed serial data after its terminal panic record\");\n    }",
        breaks: "serial data is accepted after a terminal panic record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        id: 398,
        file: "xtask/src/qemu/serial.rs",
        find: "        return Ok(Some(PanicTransition::Completed));",
        replace: "        return Ok(Some(PanicTransition::Unchanged));",
        breaks: "a complete panic record never releases the bounded observer",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        id: 399,
        file: "xtask/src/qemu/serial.rs",
        find: "    pub(super) fn publish(self, panicked: &AtomicBool) {\n        if self == Self::Completed {\n            panicked.store(true, Ordering::Release);\n        }\n    }",
        replace: "    pub(super) fn publish(self, panicked: &AtomicBool) {\n        panicked.store(true, Ordering::Release);\n    }",
        breaks: "a panic opening line releases QEMU before the transcript is complete",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        id: 400,
        file: "xtask/src/qemu/serial.rs",
        find: "fn is_complete_panic_record(line: &str) -> bool {\n    terminated_serial_text(line).and_then(|line| line.strip_prefix(\"[error] \"))\n        == Some(COMPLETE_MARKER)\n}",
        replace: "fn is_complete_panic_record(line: &str) -> bool {\n    line.contains(COMPLETE_MARKER)\n}",
        breaks: "a malformed or non-error panic terminator completes the record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        id: 401,
        file: "xtask/src/qemu/serial.rs",
        find: "    if line.starts_with(OPEN_PREFIX) {",
        replace: "    if line.contains(OPEN_PREFIX.trim_start_matches(\"[error] \")) {",
        breaks: "an unframed line can forge the kernel panic opening record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        id: 402,
        file: "xtask/src/qemu/serial.rs",
        find: "fn is_exact_no_x2apic_diagnosis(line: &str) -> bool {\n    is_exact_log_message(line, LogSeverity::Warning, NO_X2APIC_DIAGNOSIS)\n}",
        replace: "fn is_exact_no_x2apic_diagnosis(line: &str) -> bool {\n    line.contains(\"no x2apic mode\")\n}",
        breaks: "an arbitrary prior line can forge the no-x2APIC diagnosis",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::the_degraded_no_x2apic_requirement_uses_the_exact_warning_record",
        },
    },
    Mutation {
        id: 403,
        file: "xtask/src/qemu.rs",
        find: "const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),\n    RequiredLogRecord::info(SHIPPED_QEMU_TCG_ISOLATION_MARKER),\n    NO_X2APIC_RECORD,",
        replace: "const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),\n    RequiredLogRecord::info(SHIPPED_QEMU_TCG_ISOLATION_MARKER),\n    RequiredLogRecord::warning(\"no x2apic mode\"),",
        breaks: "the settled degraded boot requires only a no-x2APIC phrase instead of the canonical warning record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::the_degraded_no_x2apic_requirement_uses_the_exact_warning_record",
        },
    },
    Mutation {
        id: 404,
        file: "xtask/src/qemu/serial.rs",
        find: "        (Some(_), false) => bail!(\"QEMU stopped with an incomplete serial panic record\"),",
        replace: "        (Some(transcript), false) => Ok(Some(transcript)),",
        breaks: "a truncated panic report is credited as complete",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_panic_outcome_preserves_earlier_failures_and_its_exact_transcript",
        },
    },
    Mutation {
        id: 409,
        file: "xtask/src/qemu/serial.rs",
        find: "        *panic = Some(text.to_string());",
        replace: "        *panic = Some(text.trim_end().to_string());",
        breaks: "panic attribution silently drops trailing body whitespace",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        id: 410,
        file: "xtask/src/qemu/serial.rs",
        find: "        let text = terminated_serial_text(line)\n            .context(\"the kernel printed an unterminated panic opening record\")?;",
        replace: "        let text = terminated_serial_text(line).unwrap_or(line);",
        breaks: "an unterminated panic opening is accepted into the v1 record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        id: 411,
        file: "xtask/src/qemu/serial.rs",
        find: "    if text.contains('\\r') || text.contains('\\n') {\n        return None;\n    }",
        replace: "    let _ = text.contains('\\r') || text.contains('\\n');",
        breaks: "a panic record retains an internal or doubled physical carriage return",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        id: 417,
        file: "xtask/src/qemu.rs",
        find: "const EXPECTED_SELFTEST_CHECKS: usize = ",
        replace: "const EXPECTED_SELFTEST_CHECKS: usize = 1 + ",
        breaks: "the host accepts a different machine-check population than the kernel",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::kernel_source::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 418,
        file: "xtask/src/qemu/serial.rs",
        find: "    if observation.failed.len() != verdict.failed {",
        replace: "    if false && observation.failed.len() != verdict.failed {",
        breaks: "a failed verdict may omit the names of its red checks",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_failed_verdict_must_name_every_failed_check",
        },
    },
    Mutation {
        id: 474,
        file: "xtask/src/qemu.rs",
        find: "pub(crate) const MACHINE_TYPE: &str = \"pc-q35-9.2\";",
        replace: "pub(crate) const MACHINE_TYPE: &str = \"q35\";",
        breaks: "QEMU's moving q35 alias changes the qualified hardware model",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::the_machine_type_is_a_versioned_q35_not_an_alias",
        },
    },
    Mutation {
        id: 512,
        file: "xtask/src/qemu/run_files.rs",
        find: "    let escaped = path.to_string_lossy().replace(',', \",,\");",
        replace: "    let escaped = path.to_string_lossy().into_owned();",
        breaks: "a comma in a disk path remains QEMU drive syntax",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::drive_paths_escape_commas_without_quoting_spaces",
        },
    },
    Mutation {
        id: 513,
        file: "xtask/src/qemu/run_files.rs",
        find: "    drive_file(&format!(\"{options},snapshot=on\"), path)",
        replace: "    drive_file(&format!(\"{options},snapshot=off\"), path)",
        breaks: "two QEMU runs may write the same shared build media",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::shared_media_writes_use_a_private_snapshot",
        },
    },
    Mutation {
        id: 516,
        file: "xtask/src/qemu/run_files.rs",
        find: "        let sequence = NEXT_RUN.fetch_add(1, Ordering::Relaxed);",
        replace: "        let sequence = NEXT_RUN.load(Ordering::Relaxed);",
        breaks: "concurrent QEMU runs reuse one firmware-store name",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::simultaneous_runs_get_distinct_writable_firmware",
        },
    },
    Mutation {
        id: 519,
        file: "xtask/src/qemu/storage.rs",
        find: "        match file.try_lock() {",
        replace: "        match std::result::Result::<(), TryLockError>::Ok(()) {",
        breaks: "concurrent runners can rewrite the NVMe corpus together",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::storage_construction_is_serialized_across_runners",
        },
    },
    Mutation {
        id: 533,
        file: "xtask/src/qemu/serial.rs",
        find: "        if core::iter::Iterator::next(&mut fields).is_some() {\n            bail!(\"trailing fields in foundation baseline\");\n        }",
        replace: "        if false && core::iter::Iterator::next(&mut fields).is_some() {\n            bail!(\"trailing fields in foundation baseline\");\n        }",
        breaks: "a performance record can append unversioned fields",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::baseline_record_is_versioned_ordered_and_unique",
        },
    },
    Mutation {
        id: 537,
        file: "xtask/src/qemu/debug.rs",
        find: "    if port == 0 {",
        replace: "    if false && port == 0 {",
        breaks: "the debugger accepts QEMU's unallocated port-zero placeholder",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::the_debug_endpoint_is_allocated_and_loopback_only",
        },
    },
    Mutation {
        id: 545,
        file: "xtask/src/qemu/serial.rs",
        find: "            failed: observation.failed,",
        replace: "            failed: Vec::new(),",
        breaks: "a terminal observation discards exact checks that failed before its panic",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_panic_outcome_preserves_earlier_failures_and_its_exact_transcript",
        },
    },
    Mutation {
        id: 617,
        file: "xtask/src/qemu.rs",
        find: "        \"-no-user-config\",\n        \"-accel\",",
        replace: "        \"-accel\",",
        breaks: "ambient QEMU configuration can replace the qualified machine profile",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::every_qemu_runner_pins_the_exact_tcg_profile",
        },
    },
    Mutation {
        id: 618,
        file: "xtask/src/qemu.rs",
        find: "        \"-accel\",\n        \"tcg\",\n        \"-machine\",",
        replace: "        \"-accel\",\n        \"whpx\",\n        \"-machine\",",
        breaks: "the qualified machine can run under an unqualified accelerator",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::every_qemu_runner_pins_the_exact_tcg_profile",
        },
    },
    Mutation {
        id: 619,
        file: "xtask/src/qemu.rs",
        find: "        \"-cpu\",\n        \"max,hypervisor=on,tcg-cpuid=on\",\n        \"-m\",",
        replace: "        \"-cpu\",\n        \"max\",\n        \"-m\",",
        breaks: "the shipped QEMU CPU omits the exact TCG identity witness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::every_qemu_runner_pins_the_exact_tcg_profile",
        },
    },
    Mutation {
        id: 620,
        file: "xtask/src/qemu/boot_check.rs",
        find: "const DEGRADED_CPU: [&str; 2] = [\n    \"-cpu\",\n    \"qemu64,-x2apic,-smep,-smap,-umip,hypervisor=on,tcg-cpuid=on\",\n];",
        replace: "const DEGRADED_CPU: [&str; 2] = [\n    \"-cpu\",\n    \"qemu64,-x2apic,-smep,-smap,-umip\",\n];",
        breaks: "the degraded QEMU CPU omits the exact TCG identity witness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::every_qemu_runner_pins_the_exact_tcg_profile",
        },
    },
    Mutation {
        id: 621,
        file: "xtask/src/qemu.rs",
        find: "    validate_run_arguments(extra)?;",
        replace: "    let _unchecked_extra = extra;",
        breaks: "interactive arguments can replace the fixed TCG profile",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::interactive_arguments_cannot_replace_the_tcg_profile",
        },
    },
    Mutation {
        id: 622,
        file: "xtask/src/qemu.rs",
        find: "        if !matches!(option, \"-d\" | \"-D\" | \"-display\" | \"--display\") {",
        replace: "        if !matches!(option, \"-d\" | \"-D\" | \"-display\" | \"--display\" | \"-cpu\") {",
        breaks: "interactive arguments can replace the qualified CPU",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::interactive_arguments_cannot_replace_the_tcg_profile",
        },
    },
    Mutation {
        id: 623,
        file: "xtask/src/qemu.rs",
        find: "        if !matches!(option, \"-d\" | \"-D\" | \"-display\" | \"--display\") {",
        replace: "        if !matches!(option, \"-d\" | \"-D\" | \"-display\" | \"--display\" | \"-global\") {",
        breaks: "interactive arguments can override a qualified QOM property",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::interactive_arguments_cannot_replace_the_tcg_profile",
        },
    },
    Mutation {
        id: 624,
        file: "xtask/src/qemu.rs",
        find: "        if !matches!(option, \"-d\" | \"-D\" | \"-display\" | \"--display\") {",
        replace: "        if !matches!(option, \"-d\" | \"-D\" | \"-display\" | \"--display\" | \"-readconfig\") {",
        breaks: "interactive arguments can import an unqualified QEMU configuration",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::interactive_arguments_cannot_replace_the_tcg_profile",
        },
    },
    Mutation {
        id: 625,
        file: "xtask/src/qemu.rs",
        find: "const SHIPPED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),\n    RequiredLogRecord::info(SHIPPED_QEMU_TCG_ISOLATION_MARKER),\n    RequiredLogRecord::info(NVME_FALLBACK_MARKER),\n    RequiredLogRecord::info(\n        \"keyboard: PCI interrupt isolation keeps every io-apic line masked; polling\",\n    ),\n    RequiredLogRecord::info(\"keyboard: polled every 10 ms\"),\n    RequiredLogRecord::info(SHIPPED_USER_EXECUTION_MARKER),\n];",
        replace: "const SHIPPED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),\n    RequiredLogRecord::info(NVME_FALLBACK_MARKER),\n    RequiredLogRecord::info(\n        \"keyboard: PCI interrupt isolation keeps every io-apic line masked; polling\",\n    ),\n    RequiredLogRecord::info(\"keyboard: polled every 10 ms\"),\n    RequiredLogRecord::info(SHIPPED_USER_EXECUTION_MARKER),\n];",
        breaks: "a shipped boot can pass without consuming the TCG isolation policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 626,
        file: "xtask/src/qemu/serial.rs",
        find: "fn is_exact_log_message(line: &str, severity: LogSeverity, expected: &str) -> bool {\n    log_message(line, severity) == Some(expected)\n}",
        replace: "fn is_exact_log_message(line: &str, severity: LogSeverity, expected: &str) -> bool {\n    log_message(line, severity).is_some_and(|message| message.contains(expected))\n}",
        breaks: "a forged substring can satisfy the consumed-policy marker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::required_boot_records_refuse_framing_and_message_near_misses",
        },
    },
    Mutation {
        id: 627,
        file: "xtask/src/qemu/serial.rs",
        find: "    if verdict.status == VerdictStatus::Pass && !observation.qemu_tcg_isolation {",
        replace: "    if false && verdict.status == VerdictStatus::Pass && !observation.qemu_tcg_isolation {",
        breaks: "a passing self-test can omit the consumed TCG isolation policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_passing_selftest_requires_the_exact_consumed_tcg_policy",
        },
    },
    Mutation {
        id: 628,
        file: "xtask/src/qemu/serial.rs",
        find: "        if observation.qemu_tcg_isolation {",
        replace: "        if false && observation.qemu_tcg_isolation {",
        breaks: "duplicate TCG isolation decisions can masquerade as one consumed policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_passing_selftest_requires_the_exact_consumed_tcg_policy",
        },
    },
    Mutation {
        id: 797,
        file: "xtask/src/qemu/run_files.rs",
        find: "    let directory = crate::image::qemu_run_directory(target)?;",
        replace: r#"    let directory = target.join(crate::paths::QEMU_RUN_DIRECTORY);
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("creating {}", directory.display()))?;"#,
        breaks: "private firmware can be created through a redirected QEMU run root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::private_firmware_refuses_a_redirected_run_root_before_writing",
        },
    },
    Mutation {
        id: 799,
        file: "xtask/src/qemu/storage.rs",
        find: r#"    crate::mutate::workspace::prepare_real_directory_tree(&parent, "QEMU storage directory")?;"#,
        replace: r#"    std::fs::create_dir_all(&parent)
        .with_context(|| format!("creating {}", parent.display()))?;"#,
        breaks: "QEMU storage can be created through a redirected generated-state parent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::storage_creation_refuses_a_redirected_parent_before_writing",
        },
    },
    Mutation {
        id: 800,
        file: "xtask/src/qemu/storage.rs",
        find: r#"        crate::mutate::workspace::publish_staged_file(
            parent,
            &staging,
            backing,
            "NVMe image",
            |published| {
                let digest = crate::sha256::digest_reader(
                    crate::mutate::workspace::open_real_file(published, "published NVMe image")?,
                )
                .with_context(|| format!("hashing published NVMe image {}", published.display()))?;
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
            name: "qemu::tests::runner::storage_publication_never_writes_through_an_external_link",
        },
    },
    Mutation {
        id: 801,
        file: "xtask/src/qemu/storage.rs",
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
            name: "qemu::tests::runner::storage_staging_supports_read_modify_write_builders",
        },
    },
    Mutation {
        id: 802,
        file: "xtask/src/qemu/storage.rs",
        find: r#"fn require_storage_lock_path(parent: &Path, path: &Path, must_exist: bool) -> Result<()> {
    ensure!(
        path == parent.join("nvme.lock"),
        "storage build lock is not the exact allowlisted path"
    );
    let present = crate::mutate::workspace::require_direct_file_if_present(
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
            name: "qemu::tests::runner::storage_publication_never_writes_through_an_external_link",
        },
    },
    Mutation {
        id: 803,
        file: "xtask/src/qemu/storage.rs",
        find: "    after_open(path);\n    require_storage_lock_path(parent, path, true)?;",
        replace: "    after_open(path);\n    require_storage_lock_path(parent, path, false)?;",
        breaks: "an opened QEMU storage lock remains named before serialization begins",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::an_open_storage_lock_must_remain_at_its_name",
        },
    },
    Mutation {
        id: 813,
        file: "xtask/src/qemu/selftest.rs",
        find: "            Ok(false) => return termination,",
        replace: "            Ok(false) => return Ok(()),",
        breaks: "a live QEMU child after refused termination is accepted as stopped",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::qemu_stop_refuses_timeout_and_a_live_child_after_kill_refusal",
        },
    },
    Mutation {
        id: 814,
        file: "xtask/src/qemu/selftest.rs",
        find: r#"        if exited(child).context("polling terminated QEMU")? {
            return Ok(());
        }"#,
        replace: r#"        if false && exited(child).context("polling terminated QEMU")? {
            return Ok(());
        }"#,
        breaks: "QEMU teardown never observes the terminated direct child",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::qemu_stop_polls_until_a_terminated_child_is_reaped",
        },
    },
    Mutation {
        id: 815,
        file: "xtask/src/qemu/selftest.rs",
        find: r#"            bail!(
                "QEMU did not exit within {} ms after termination",
                timeout.as_millis()
            );"#,
        replace: "            return Ok(());",
        breaks: "QEMU teardown reports success when the terminated child never exits",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::qemu_stop_refuses_timeout_and_a_live_child_after_kill_refusal",
        },
    },
    Mutation {
        id: 914,
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
        id: 1183,
        file: "xtask/src/qemu/serial.rs",
        find: "    let line = line.strip_suffix('\\n')?;",
        replace: "    let line = line.strip_suffix('\\n').unwrap_or(line);",
        breaks: "an unterminated serial fragment can satisfy exact boot evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::required_boot_records_refuse_framing_and_message_near_misses",
        },
    },
    Mutation {
        id: 1184,
        file: "xtask/src/qemu/serial.rs",
        find: "    if text.contains('\\r') || text.contains('\\n') {\n        return None;\n    }",
        replace: "",
        breaks: "an internal carriage return can enter a framed panic transcript",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_boot_panic_retains_its_transcript_and_prior_x2apic_refusal",
        },
    },
    Mutation {
        id: 1185,
        file: "xtask/src/qemu/serial.rs",
        find: "            Self::Warning => \"[warn ] \",",
        replace: "            Self::Warning => \"[info ] \",",
        breaks: "warning evidence is accepted with info-record framing",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::required_boot_records_refuse_framing_and_message_near_misses",
        },
    },
    Mutation {
        id: 1186,
        file: "xtask/src/qemu/serial.rs",
        find: "fn is_required_boot_marker(line: &str, record: RequiredLogRecord) -> bool {\n    if record == NO_X2APIC_RECORD {\n        return is_exact_no_x2apic_diagnosis(line);\n    }\n    is_exact_log_message(line, record.severity, record.message)\n}",
        replace: "fn is_required_boot_marker(line: &str, record: RequiredLogRecord) -> bool {\n    if record.message == DEGRADED_FILESYSTEM_MARKER {\n        return line.contains(\"tmpfs:\");\n    }\n    if record == NO_X2APIC_RECORD {\n        return is_exact_no_x2apic_diagnosis(line);\n    }\n    is_exact_log_message(line, record.severity, record.message)\n}",
        breaks: "a tmpfs failure warning can forge the degraded filesystem success evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::failure_records_cannot_satisfy_required_boot_evidence",
        },
    },
    Mutation {
        id: 1187,
        file: "xtask/src/qemu/serial.rs",
        find: "        .strip_prefix(SETTLED_MARKER)",
        replace: "        .split_once(SETTLED_MARKER)\n        .map(|(_, uptime)| uptime)",
        breaks: "text before the steady-state message envelope is accepted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::steady_state_requires_one_exact_dynamic_info_record",
        },
    },
    Mutation {
        id: 1188,
        file: "xtask/src/qemu/serial.rs",
        find: "        .and_then(|uptime| uptime.strip_suffix(\" ms\"))",
        replace: "        .and_then(|uptime| uptime.split_once(\" ms\").map(|(uptime, _)| uptime))",
        breaks: "text after the steady-state uptime suffix is accepted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::steady_state_requires_one_exact_dynamic_info_record",
        },
    },
    Mutation {
        id: 1189,
        file: "xtask/src/qemu.rs",
        find: "    if text.is_empty()\n        || (text.len() > 1 && text.starts_with('0'))\n        || !text.bytes().all(|byte| byte.is_ascii_digit())\n    {",
        replace: "    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {",
        breaks: "a noncanonical leading-zero uptime satisfies steady state",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::steady_state_requires_one_exact_dynamic_info_record",
        },
    },
    Mutation {
        id: 1190,
        file: "xtask/src/qemu.rs",
        find: "    if text.is_empty()\n        || (text.len() > 1 && text.starts_with('0'))\n        || !text.bytes().all(|byte| byte.is_ascii_digit())\n    {",
        replace: "    if text.is_empty() || (text.len() > 1 && text.starts_with('0')) {",
        breaks: "a signed uptime satisfies the canonical decimal grammar",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::steady_state_requires_one_exact_dynamic_info_record",
        },
    },
    Mutation {
        id: 1191,
        file: "xtask/src/qemu.rs",
        find: "    <u64 as core::str::FromStr>::from_str(text).ok()",
        replace: "    Some(<u64 as core::str::FromStr>::from_str(text).unwrap_or(u64::MAX))",
        breaks: "an overflowing uptime satisfies the steady-state record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::steady_state_requires_one_exact_dynamic_info_record",
        },
    },
    Mutation {
        id: 1192,
        file: "xtask/src/qemu/boot_check.rs",
        find: "            if is_settled_record(&text) {",
        replace: "            if text.contains(SETTLED_MARKER) {",
        breaks: "the serial watcher publishes steady state from a marker substring",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::kernel_source::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1193,
        file: "xtask/src/qemu/boot_check.rs",
        find: "                let present = is_required_boot_marker(&text, *record);",
        replace: "                let present = text.contains(record.message);",
        breaks: "the serial watcher bypasses exact required-record matching",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::kernel_source::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1194,
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(NVME_FALLBACK_MARKER),\n    RequiredLogRecord::info(\n        \"keyboard: PCI interrupt isolation keeps every io-apic line masked; polling\",\n    ),",
        replace: "    RequiredLogRecord::info(\n        \"keyboard: PCI interrupt isolation keeps every io-apic line masked; polling\",\n    ),",
        breaks: "the shipped boot can settle without proving first-candidate NVMe retirement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1195,
        file: "xtask/src/qemu.rs",
        find: "const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),\n    RequiredLogRecord::info(SHIPPED_QEMU_TCG_ISOLATION_MARKER),\n    NO_X2APIC_RECORD,",
        replace: "const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),\n    NO_X2APIC_RECORD,",
        breaks: "the degraded boot can settle without proving its QEMU TCG policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1196,
        file: "xtask/src/qemu.rs",
        find: "    \"DMA-safe boot handoff refused: DMA handoff fence: PCI configuration is unavailable\";",
        replace: "    \"DMA-safe boot handoff refused: PCI configuration is unavailable\";",
        breaks: "the terminal no-PCI boundary loses the exact handoff-fence diagnosis",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1197,
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(DEGRADED_FILESYSTEM_MARKER),",
        replace: "    RequiredLogRecord::info(\"tmpfs:\"),",
        breaks: "the degraded filesystem requirement is only a broad tmpfs prefix",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1198,
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(DEGRADED_STORAGE_MARKER),",
        replace: "    RequiredLogRecord::info(\"devfs: storage is\"),",
        breaks: "the degraded storage requirement omits its exact device description",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1199,
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(DEGRADED_CONSOLE_MARKER),",
        replace: "    RequiredLogRecord::info(\"devfs: console is\"),",
        breaks: "the degraded console requirement omits its exact device description",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1200,
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(DEGRADED_PROTECTIONS_MARKER),",
        replace: "    RequiredLogRecord::info(\"cpu: protections\"),",
        breaks: "the degraded protection requirement proves neither mandatory nor absent features",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1201,
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::warning(DEGRADED_XSTATE_MARKER),",
        replace: "    RequiredLogRecord::warning(\"cpu: extended state unavailable\"),",
        breaks: "the degraded extended-state requirement omits the user-boundary refusal",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1202,
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::warning(DEGRADED_USER_MARKER),",
        replace: "    RequiredLogRecord::warning(\"user: no program\"),",
        breaks: "the degraded user requirement omits its exact first launch veto",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1203,
        file: "xtask/src/qemu/storage.rs",
        find: "        write_expected(&mut file)?;",
        replace: "        let _ = write_expected(&mut file);",
        breaks: "a failed QEMU storage stage can publish partial bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::a_failed_storage_stage_preserves_the_complete_authoritative_image",
        },
    },
    Mutation {
        id: 1204,
        file: "xtask/src/qemu/storage.rs",
        find: "    }\n    for lba in 0..NVME_SIGNED_BLOCKS {\n        Seek::seek(file, SeekFrom::Start(lba * NVME_BLOCK_BYTES as u64))",
        replace: "    }\n    for lba in 0..NVME_SIGNED_BLOCKS - 1 {\n        Seek::seek(file, SeekFrom::Start(lba * NVME_BLOCK_BYTES as u64))",
        breaks: "the streamed NVMe namespace omits its final signed block",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::the_nvme_namespace_is_streamed_to_its_exact_extent",
        },
    },
    Mutation {
        id: 1238,
        file: "xtask/src/qemu/serial.rs",
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
            name: "qemu::tests::serial::a_failed_check_requires_one_exact_terminated_error_record",
        },
    },
    Mutation {
        id: 1239,
        file: "xtask/src/qemu/serial.rs",
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
            name: "qemu::tests::serial::verdict_records_require_exact_framing_and_status_severity",
        },
    },
    Mutation {
        id: 1240,
        file: "xtask/src/qemu/serial.rs",
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
            name: "qemu::tests::serial::verdict_records_require_exact_framing_and_status_severity",
        },
    },
    Mutation {
        id: 1241,
        file: "xtask/src/qemu/serial.rs",
        find: "    let mut fields = message.strip_prefix(SELFTEST_VERDICT_PREFIX)?.split(' ');",
        replace: "    let mut fields = message\n        .strip_prefix(SELFTEST_VERDICT_PREFIX)?\n        .split_whitespace();",
        breaks: "noncanonical whitespace can separate self-test verdict fields",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::verdict_fields_require_single_ascii_spaces",
        },
    },
    Mutation {
        id: 1242,
        file: "xtask/src/qemu/serial.rs",
        find: r#"    usize::try_from(canonical_decimal(value).context("invalid self-test verdict count")?)
        .context("self-test verdict count does not fit usize")"#,
        replace: r#"    value.parse().context("invalid self-test verdict count")"#,
        breaks: "self-test verdict counts accept noncanonical integer spellings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::verdict_counts_use_canonical_decimals",
        },
    },
    Mutation {
        id: 1243,
        file: "xtask/src/qemu/serial.rs",
        find: r#"    let message = log_message(line, LogSeverity::Info)?;
    let mut fields = message.strip_prefix(BASELINE_PREFIX)?.split(' ');"#,
        replace: r#"    let start = line.find(BASELINE_PREFIX)?;
    let mut fields = line[start + BASELINE_PREFIX.len()..].trim_end().split(' ');"#,
        breaks: "a severity-less or unterminated substring can forge a performance baseline",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::baseline_records_require_exact_info_framing",
        },
    },
    Mutation {
        id: 1244,
        file: "xtask/src/qemu/serial.rs",
        find: "    let mut fields = message.strip_prefix(BASELINE_PREFIX)?.split(' ');",
        replace: "    let mut fields = message\n        .strip_prefix(BASELINE_PREFIX)?\n        .split_whitespace();",
        breaks: "noncanonical whitespace can separate performance-baseline fields",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::baseline_fields_require_single_ascii_spaces",
        },
    },
    Mutation {
        id: 1245,
        file: "xtask/src/qemu/serial.rs",
        find: r#"    canonical_decimal(value).with_context(|| format!("invalid baseline field {name}"))"#,
        replace: r#"    value
        .parse()
        .with_context(|| format!("invalid baseline field {name}"))"#,
        breaks: "performance-baseline values accept noncanonical integer spellings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::baseline_values_use_canonical_decimals",
        },
    },
    Mutation {
        id: 1246,
        file: "xtask/src/qemu/serial.rs",
        find: ".position(|record| is_required_boot_marker(line, *record))",
        replace: ".position(|record| line.contains(record.message))",
        breaks: "a substring can trigger an interactive QMP injection round",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::interactive_input_requires_exact_terminated_info_records",
        },
    },
    Mutation {
        id: 1247,
        file: "xtask/src/qemu/serial.rs",
        find: r#"    let ordered = match order {
        InteractiveMarkerOrder::Strict => index == *next,
        InteractiveMarkerOrder::Mutation => index >= *next,
    };
    ensure!(
        ordered,
        "self-test interactive marker {} arrived while waiting for {}",
        index + 1,
        *next + 1
    );"#,
        replace: r#"    let _unchecked_order = order;"#,
        breaks: "interactive QMP injection accepts duplicate or out-of-order marker records",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::interactive_input_markers_are_unique_and_ordered",
        },
    },
    Mutation {
        id: 1248,
        file: "xtask/src/qemu/serial.rs",
        find: "        InteractiveMarkerOrder::Mutation => index >= *next,",
        replace: "        InteractiveMarkerOrder::Mutation => index == *next,",
        breaks: "mutation observation stops at an earlier stage's missing interaction marker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::interactive_input_markers_are_unique_and_ordered",
        },
    },
    Mutation {
        id: 1249,
        file: "xtask/src/qemu/serial.rs",
        find: "    } else if observed == round {\n        InjectionAction::Inject",
        replace: "    } else if observed >= round {\n        InjectionAction::Inject",
        breaks: "a skipped interaction stage injects its stale keystrokes into the next stage",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::interactive_input_markers_are_unique_and_ordered",
        },
    },
    Mutation {
        id: 1250,
        file: "xtask/src/qemu/selftest.rs",
        find: r#"                    match observe_interactive_marker(
                        &text,
                        &mut next_interactive_marker,
                        marker_order,
                    ) {
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
            name: "qemu::tests::kernel_source::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1251,
        file: "xtask/src/qemu/serial.rs",
        find: "    RequiredLogRecord::info(STORAGE_DELAY_MARKER),",
        replace: "    RequiredLogRecord::warning(STORAGE_DELAY_MARKER),",
        breaks: "the storage-delay injection round is enrolled at warning severity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1252,
        file: "xtask/src/qemu/serial.rs",
        find: "    RequiredLogRecord::info(READY_MARKER),",
        replace: "    RequiredLogRecord::warning(READY_MARKER),",
        breaks: "the injected-keystroke round is enrolled at warning severity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1253,
        file: "xtask/src/qemu/serial.rs",
        find: "    RequiredLogRecord::info(TRANSLATED_MARKER),",
        replace: "    RequiredLogRecord::warning(TRANSLATED_MARKER),",
        breaks: "the translated-keystroke round is enrolled at warning severity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1254,
        file: "xtask/src/qemu/serial.rs",
        find: r#"        if observation.failed.iter().any(|failed| failed == name) {
            bail!("the self-test kernel named one failed check more than once");
        }"#,
        replace: "",
        breaks: "duplicate failed-check records can forge the mutation failure count",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::failed_check_names_are_unique_and_precede_the_terminal_verdict",
        },
    },
    Mutation {
        id: 1255,
        file: "xtask/src/qemu/serial.rs",
        find: r#"    if observation.verdict.is_some() {
        bail!("the self-test kernel printed serial data after its final verdict");
    }"#,
        replace: "",
        breaks: "failed-check evidence can be attributed after the terminal self-test verdict",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::failed_check_names_are_unique_and_precede_the_terminal_verdict",
        },
    },
    Mutation {
        id: 1517,
        file: "xtask/src/qemu/serial.rs",
        find: "const QMP_ON_STDIO: [&str; 2] = [\"-qmp\", \"stdio\"];",
        replace: "const QMP_ON_STDIO: [&str; 2] = [\"-qmp\", \"tcp:127.0.0.1:4444,server=on,wait=off\"];",
        breaks: "QMP returns to a loopback port any local process can reach",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::qemu_control_stays_on_the_child_s_own_streams",
        },
    },
    Mutation {
        id: 1518,
        file: "xtask/src/qemu/serial.rs",
        find: "    let input = child.stdin.take().context(\"taking QEMU's stdin pipe\")?;\n    let output = child.stdout.take().context(\"taking QEMU's stdout pipe\")?;\n    Qmp::negotiate(input, output, deadline, cancelled)",
        replace: "    let _monitor = std::net::TcpListener::bind((\"127.0.0.1\", 0));\n    let input = child.stdin.take().context(\"taking QEMU's stdin pipe\")?;\n    let output = child.stdout.take().context(\"taking QEMU's stdout pipe\")?;\n    Qmp::negotiate(input, output, deadline, cancelled)",
        breaks: "a loopback control listener returns beside the private QMP session",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::qemu_control_stays_on_the_child_s_own_streams",
        },
    },
    Mutation {
        id: 1519,
        file: "xtask/src/qemu/serial.rs",
        find: "        if closed.load(Ordering::Acquire) {\n            return Ok(0);\n        }",
        replace: "        if !closed.load(Ordering::Acquire) {\n            return Ok(0);\n        }",
        breaks: "the serial reader treats a live transport close as clean QEMU exit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_serial_reader_refuses_an_early_transport_close",
        },
    },
    Mutation {
        id: 1520,
        file: "xtask/src/qemu/serial.rs",
        find: "            getrandom::fill(&mut random)\n                .map_err(|error| anyhow!(\"obtaining a private QEMU serial-pipe name: {error}\"))?;",
        replace: "            random.fill(0);",
        breaks: "two automated QEMU runs can reuse one guessable serial-pipe name",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_serial_uses_a_private_backpressured_transport",
        },
    },
    Mutation {
        id: 1596,
        file: "xtask/src/qemu/serial.rs",
        find: "            sample.leaf <= sample.used,",
        replace: "            sample.leaf <= sample.capacity,",
        breaks: "a stack record whose live-leaf use exceeds its painted high-water mark is accepted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_passing_selftest_requires_bounded_conservative_nvme_stack_evidence",
        },
    },
    Mutation {
        id: 1626,
        file: "xtask/src/qemu/serial.rs",
        find: "            line_bytes <= line_limit,",
        replace: "            line_bytes <= usize::MAX,",
        breaks: "one unterminated guest serial record can grow without its reviewed ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_serial_capture_bounds_each_record_and_the_whole_run",
        },
    },
    Mutation {
        id: 1627,
        file: "xtask/src/qemu/serial.rs",
        find: "            aggregate <= total_limit,",
        replace: "            aggregate <= usize::MAX,",
        breaks: "many short guest serial records can exceed the run-wide capture ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_serial_capture_bounds_each_record_and_the_whole_run",
        },
    },
    Mutation {
        id: 1628,
        file: "xtask/src/qemu/serial.rs",
        find: "        let backend = OsString::from(format!(\"pipe,id=serial0,path={}\", self.name));",
        replace: "        let backend = OsString::from(format!(\"file,id=serial0,path={}\", self.name));",
        breaks: "automated QEMU serial returns to an unbounded disk-backed sink",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_serial_uses_a_private_backpressured_transport",
        },
    },
    Mutation {
        id: 1629,
        file: "xtask/src/qemu/serial.rs",
        find: "        if (b' '..=b'~').contains(&byte) {",
        replace: "        if true {",
        breaks: "guest terminal-control bytes are printed directly on the host console",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_serial_display_escapes_guest_terminal_controls",
        },
    },
    Mutation {
        id: 1726,
        file: "xtask/src/qemu.rs",
        find: "const IOMMU_DEVICE: [&str; 2] = [\"-device\", \"intel-iommu,intremap=on,eim=on,caching-mode=on\"];",
        replace: "const IOMMU_DEVICE: [&str; 2] = [\"-device\", \"intel-iommu,intremap=off,caching-mode=on\"];",
        breaks: "the qualified QEMU profile stops exercising VT-d interrupt remapping",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::every_qemu_runner_pins_the_exact_tcg_profile",
        },
    },
    Mutation {
        id: 1727,
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(\"keyboard: polled every 10 ms\"),\n    RequiredLogRecord::info(SHIPPED_USER_EXECUTION_MARKER),\n];\n\n/// A shipped kernel must accept this input without unmasking legacy IRQ1.",
        replace: "    RequiredLogRecord::info(SHIPPED_USER_EXECUTION_MARKER),\n];\n\n/// A shipped kernel must accept this input without unmasking legacy IRQ1.",
        breaks: "the shipped boot can pass without observing its live keyboard poller",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1809,
        file: "xtask/src/qemu/run_files.rs",
        find: "    table[0..4].copy_from_slice(b\"WZDM\");",
        replace: "    table[0..4].copy_from_slice(b\"XZDM\");",
        breaks: "the qualified QEMU coverage attestation has another ACPI signature",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::the_fixed_qemu_dma_coverage_names_the_real_unit",
        },
    },
    Mutation {
        id: 1810,
        file: "xtask/src/qemu.rs",
        find: "        if !matches!(option, \"-d\" | \"-D\" | \"-display\" | \"--display\") {",
        replace: "        if !matches!(option, \"-d\" | \"-D\" | \"-display\" | \"--display\" | \"-acpitable\") {",
        breaks: "interactive arguments can append a competing DMA coverage attestation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::interactive_arguments_cannot_replace_the_tcg_profile",
        },
    },
    Mutation {
        id: 1812,
        file: "xtask/src/qemu/run_files.rs",
        find: "    let file = path\n        .file_name()\n        .expect(\"a reserved QEMU ACPI table path has a file name\")\n        .to_string_lossy()",
        replace: "    let file = path.to_string_lossy()",
        breaks: "the Windows QEMU ACPI parser receives an absolute drive path as a colon-separated file list",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::the_fixed_qemu_dma_coverage_names_the_real_unit",
        },
    },
    Mutation {
        id: 1813,
        file: "xtask/src/qemu/debug.rs",
        find: "        let mut child = Command::new(&installation.emulator)\n            .current_dir(vars.directory())\n            .args(machine_args(Remapping::Present))\n            .args(coverage.arguments())\n            .args([\"-S\", \"-display\", \"none\", \"-serial\", \"null\", \"-no-reboot\"])",
        replace: "        let mut child = Command::new(&installation.emulator)\n            .current_dir(vars.directory())\n            .args(machine_args(Remapping::Present))\n            .args([\"-S\", \"-display\", \"none\", \"-serial\", \"null\", \"-no-reboot\"])",
        breaks: "the debugger runner bypasses the qualified DMA coverage attestation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::every_qemu_runner_pins_the_exact_tcg_profile",
        },
    },
    Mutation {
        id: 1830,
        file: "xtask/src/qemu.rs",
        find: "        .current_dir(vars.directory())\n        .args(machine_args(Remapping::Present))\n        .args(coverage.arguments())\n        .args([\"-serial\", \"stdio\"])",
        replace: "        .current_dir(vars.directory())\n        .args(machine_args(Remapping::Present))\n        .args([\"-serial\", \"stdio\"])",
        breaks: "the interactive QEMU runner boots without the DMA handoff authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::every_qemu_runner_pins_the_exact_tcg_profile",
        },
    },
    Mutation {
        id: 1837,
        file: "xtask/src/qemu.rs",
        find: "        if !matches!(option, \"-d\" | \"-D\" | \"-display\" | \"--display\") {",
        replace: "        if !matches!(option, \"-d\" | \"-D\" | \"-display\" | \"--display\" | \"-device\") {",
        breaks: "a caller can add a DMA-capable QEMU device under the fixed handoff attestation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::interactive_arguments_cannot_replace_the_tcg_profile",
        },
    },
    Mutation {
        id: 1851,
        file: "xtask/src/qemu/serial.rs",
        find: "    if verdict.status == VerdictStatus::Pass && !observation.strict_dma_handoff {",
        replace: "    if false && verdict.status == VerdictStatus::Pass && !observation.strict_dma_handoff {",
        breaks: "a passing QEMU self-test need not prove that its entry fence used the strict requester policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::a_passing_selftest_requires_the_exact_consumed_tcg_policy",
        },
    },
    Mutation {
        id: 1862,
        file: "xtask/src/qemu.rs",
        find: "const SHIPPED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),",
        replace: "const SHIPPED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[",
        breaks: "the shipped QEMU selector can pass without its strict DMA-fence witness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1863,
        file: "xtask/src/qemu.rs",
        find: "const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),",
        replace: "const DEGRADED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[",
        breaks: "the no-x2APIC QEMU selector can pass without its strict DMA-fence witness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 1864,
        file: "xtask/src/qemu.rs",
        find: "const UNISOLATED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[\n    RequiredLogRecord::info(STRICT_DMA_HANDOFF_MARKER),",
        replace: "const UNISOLATED_REQUIRED_MARKERS: &[RequiredLogRecord] = &[",
        breaks: "the no-IOMMU QEMU selector can pass without its strict DMA-fence witness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 2303,
        file: "xtask/src/qemu.rs",
        find: "    RequiredLogRecord::info(SHIPPED_USER_EXECUTION_MARKER),\n",
        replace: "",
        breaks: "a shipped QEMU boot qualifies without observing a user call",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::serial::automated_boots_require_the_exact_producer_records",
        },
    },
    Mutation {
        id: 2373,
        file: "xtask/src/qemu/selftest.rs",
        find: "        if !self.stop_attempted {",
        replace: "        if false && !self.stop_attempted {",
        breaks: "an unfinished QEMU owner leaves its child running on destruction",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_lifetime_cancels_and_stops_on_return_error_and_unwind",
        },
    },
    Mutation {
        id: 2374,
        file: "xtask/src/qemu/selftest.rs",
        find: "        self.cancel();\n        self.stop_attempted = true;",
        replace: "        self.stop_attempted = true;",
        breaks: "QEMU termination begins without first cancelling injection",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_lifetime_cancels_and_stops_on_return_error_and_unwind",
        },
    },
    Mutation {
        id: 2375,
        file: "xtask/src/qemu/selftest.rs",
        find: "        self.stop_attempted = true;",
        replace: "        self.stop_attempted = false;",
        breaks: "explicit child cleanup is retried during owner destruction",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_lifetime_reports_stop_refusal_and_preserves_both_errors",
        },
    },
    Mutation {
        id: 2376,
        file: "xtask/src/qemu/selftest.rs",
        find: "        (Err(error), Err(cleanup)) => {\n            Err(anyhow!(\"{error:#}; QEMU cleanup also failed: {cleanup:#}\"))\n        }",
        replace: "        (Err(error), Err(_cleanup)) => Err(error),",
        breaks: "a cleanup refusal disappears behind the observation error",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_lifetime_reports_stop_refusal_and_preserves_both_errors",
        },
    },
    Mutation {
        id: 2377,
        file: "xtask/src/qemu/selftest.rs",
        find: "    ensure!(\n        worker_finished_by(&worker, deadline),\n        \"QEMU {name} did not stop before its join deadline\"\n    );",
        replace: "    let _ = worker_finished_by(&worker, deadline);",
        breaks: "an unfinished worker reaches an unbounded join",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_worker_joins_refuse_unfinished_and_panicked_workers",
        },
    },
    Mutation {
        id: 2378,
        file: "xtask/src/qemu/selftest.rs",
        find: "        if now >= deadline {\n            return false;",
        replace: "        if false && now >= deadline {\n            return false;",
        breaks: "worker collection ignores its finite deadline",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_worker_joins_refuse_unfinished_and_panicked_workers",
        },
    },
    Mutation {
        id: 2379,
        file: "xtask/src/qemu/selftest.rs",
        find: "    let mut machine = QemuLifetime::new(child, stop_qemu);",
        replace: "    let mut machine = QemuLifetime::new(child, |_| Ok(()));",
        breaks: "the real self-test child loses its actual termination owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_owner_precedes_setup_and_stop_precedes_worker_joins",
        },
    },
    Mutation {
        id: 2380,
        file: "xtask/src/qemu/selftest.rs",
        find: "    let stopped = machine.stop();\n    serial.close();",
        replace: "    let stopped = Ok(());\n    serial.close();",
        breaks: "self-test worker collection precedes stopping its pipe peer",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_owner_precedes_setup_and_stop_precedes_worker_joins",
        },
    },
    Mutation {
        id: 2381,
        file: "xtask/src/qemu/selftest.rs",
        find: "                let worker = std::thread::spawn(move || report_hang(qmp.as_mut()));",
        replace: "                report_hang(qmp.as_mut());\n                let worker = std::thread::spawn(|| ());",
        breaks: "a hang report can block the main observation thread",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_owner_precedes_setup_and_stop_precedes_worker_joins",
        },
    },
    Mutation {
        id: 2382,
        file: "xtask/src/qemu/selftest.rs",
        find: "        && injector.as_ref().is_some_and(JoinHandle::is_finished)",
        replace: "        && injector.is_some()",
        breaks: "live injection is joined before timeout cleanup",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_owner_precedes_setup_and_stop_precedes_worker_joins",
        },
    },
    Mutation {
        id: 2383,
        file: "xtask/src/qemu/selftest.rs",
        find: "    combine_qemu_cleanup(outcome, cleanup)\n}\n\npub(super) enum SelftestTermination",
        replace: "    let _ = cleanup;\n    outcome\n}\n\npub(super) enum SelftestTermination",
        breaks: "the self-test return discards cleanup failure",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_owner_precedes_setup_and_stop_precedes_worker_joins",
        },
    },
    Mutation {
        id: 2384,
        file: "xtask/src/qemu/selftest.rs",
        find: "        (self.stop)(&mut self.child)",
        replace: "        Ok(())",
        breaks: "owner cleanup fails to stop a blocked control writer peer",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::stopping_the_owner_releases_a_qmp_writer_before_join",
        },
    },
    Mutation {
        id: 2385,
        file: "xtask/src/qemu/selftest.rs",
        find: "        SelftestTermination::TimedOut => bail!(\n            \"self-test did not finish within {} seconds\",\n            timeout.as_secs()\n        ),",
        replace: "        SelftestTermination::TimedOut => Ok(None),",
        breaks: "self-test timeout loses its exact error classification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_timeout_classification_survives_serial_and_stop_failures",
        },
    },
    Mutation {
        id: 2386,
        file: "xtask/src/qemu/selftest.rs",
        find: "        (Err(error), Err(cleanup)) => {\n            Err(anyhow!(\"{error:#}; QEMU cleanup also failed: {cleanup:#}\"))\n        }",
        replace: "        (Err(_error), Err(cleanup)) => Err(cleanup),",
        breaks: "cleanup failure erases the original observation error",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_timeout_classification_survives_serial_and_stop_failures",
        },
    },
    Mutation {
        id: 2387,
        file: "xtask/src/qemu/selftest.rs",
        find: "    let stopped = terminate_qemu_with(child, exited, terminate, timeout);",
        replace: "    let stopped = if initial.is_err() { Ok(()) } else { terminate_qemu_with(child, exited, terminate, timeout) };",
        breaks: "an initial status error skips QEMU termination",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::qemu_stop_attempts_termination_after_a_poll_failure_and_retains_every_error",
        },
    },
    Mutation {
        id: 2388,
        file: "xtask/src/qemu/selftest.rs",
        find: "    combine_qemu_cleanup(initial.map(drop), stopped)",
        replace: "    stopped",
        breaks: "successful reap erases the earlier status failure",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::qemu_stop_attempts_termination_after_a_poll_failure_and_retains_every_error",
        },
    },
    Mutation {
        id: 2389,
        file: "xtask/src/qemu/selftest.rs",
        find: "            Err(error) => return combine_qemu_cleanup(termination, Err(error)),",
        replace: "            Err(error) => return Err(error),",
        breaks: "a failed status recheck erases refused termination",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::qemu_stop_attempts_termination_after_a_poll_failure_and_retains_every_error",
        },
    },
    Mutation {
        id: 2390,
        file: "xtask/src/qemu/boot_check.rs",
        find: "    join_qemu_worker(forwarder, deadline, \"boot serial reader\")?",
        replace: "    let _ = deadline;\n    forwarder.join().map_err(|_| anyhow!(\"boot serial reader panicked\"))?",
        breaks: "boot completion blocks joining an unfinished serial reader",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::boot_serial_join_refuses_an_unfinished_reader",
        },
    },
    Mutation {
        id: 2391,
        file: "xtask/src/qemu/selftest.rs",
        find: "    let outcome =\n        combine_qemu_observation(outcome, serial_result).and_then(|(code, observation)| {",
        replace: "    let outcome =\n        combine_qemu_observation(Ok(None), serial_result).and_then(|(code, observation)| {",
        breaks: "serial cleanup bypasses self-test exit classification",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_owner_precedes_setup_and_stop_precedes_worker_joins",
        },
    },
    Mutation {
        id: 2392,
        file: "xtask/src/qemu/boot_check.rs",
        find: "    combine_qemu_cleanup(outcome, stop_outcome)?;",
        replace: "    let _ = stop_outcome;\n    outcome?;",
        breaks: "a shipped boot discards termination refusal",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_owner_precedes_setup_and_stop_precedes_worker_joins",
        },
    },
    Mutation {
        id: 2393,
        file: "xtask/src/qemu/boot_check.rs",
        find: "        combine_qemu_observation(outcome, serial_outcome),\n        stop_outcome,",
        replace: "        combine_qemu_observation(outcome, serial_outcome),\n        Ok(()),",
        breaks: "a degraded boot discards termination refusal",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::selftest_owner_precedes_setup_and_stop_precedes_worker_joins",
        },
    },
    Mutation {
        id: 2438,
        file: "xtask/src/qemu.rs",
        find: "        vars_firmware: firmware(directory, VARS_FIRMWARE)?,\n        emulator,",
        replace: "        vars_firmware: firmware(directory, VARS_FIRMWARE)?,\n        emulator: directory.join(exe_name(\"qemu-system-x86_64\")),",
        breaks: "an explicit QEMU executable is replaced by the default filename",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::installation_preserves_the_explicit_binary_and_its_firmware",
        },
    },
    Mutation {
        id: 2465,
        file: "xtask/src/qemu.rs",
        find: "    ensure!(path.is_file(), refusal);",
        replace: "    ensure!(true, refusal);",
        breaks: "a QEMU or QEMU_DIR setting that names no file is reported as missing firmware",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::a_missing_qemu_setting_is_not_blamed_on_firmware",
        },
    },
    Mutation {
        id: 2466,
        file: "xtask/src/qemu.rs",
        find: "qemu_dir.parent().filter(|_| qemu_dir.ends_with(\"bin\"))",
        replace: "qemu_dir.parent().filter(|_| false)",
        breaks: "firmware under an FHS prefix's share directory is not found",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::runner::installation_finds_firmware_under_an_fhs_prefix",
        },
    },
];
