use super::*;
use crate::test_support::{create_directory_symlink, create_file_symlink, repository_root};

const IMAGE_SECTORS: u64 = 64 * 1024 * 1024 / 512;

#[test]
fn vmware_generation_never_writes_through_preexisting_output_links() {
    let root = crate::test_support::reserve_temp_directory("wizard-vmware-output-link");
    let directory = root.join("selector");
    let input = root.join("input");
    let sentinel = root.join("outside-sentinel");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(&input, b"new disk bytes").unwrap();
    std::fs::write(&sentinel, b"outside authority").unwrap();

    let disk = directory.join(FLAT_NAME);
    std::fs::hard_link(&sentinel, &disk).unwrap();
    let copy = copy_new(&input, &disk);
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"outside authority",
        "a generated disk truncated an external hardlink"
    );
    assert!(copy.is_err(), "a preexisting generated disk was accepted");
    std::fs::remove_file(&disk).unwrap();

    let descriptor = directory.join(VMX_NAME);
    create_file_symlink(&sentinel, &descriptor);
    let write = write_new(&descriptor, b"new VMX bytes");
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"outside authority",
        "a generated descriptor followed an external symlink"
    );
    assert!(
        write.is_err(),
        "a preexisting generated descriptor was accepted"
    );
    std::fs::remove_file(&descriptor).unwrap();

    let copied = copy_new(&input, &disk).unwrap();
    assert_eq!(copied, b"new disk bytes".len() as u64);
    assert_eq!(std::fs::read(&disk).unwrap(), b"new disk bytes");
    write_new(&descriptor, b"new VMX bytes").unwrap();
    assert_eq!(std::fs::read(&descriptor).unwrap(), b"new VMX bytes");

    std::fs::remove_dir_all(root).unwrap();
}

// One translation record per storage candidate: the retired one's first, then the kept one's.
const RETIRED_TRANSLATION: &str = "[info ] iommu: translation on for 00:04.0 through the unit at 0xfed90000: 4-level walks, flushed tables, caching mode, cap 0x80d2008c222f06c6, ecap 0xf02\n";
const KEPT_TRANSLATION: &str = "[info ] iommu: translation on for 00:05.0 through the unit at 0xfed90000: 4-level walks, flushed tables, caching mode, cap 0x80d2008c222f06c6, ecap 0xf02\n";
const FIXTURE_PAIRED_ISOLATION: &str =
    "cpu: user isolation paired roots: selected; shared mappings refused";
const FIXTURE_SELFTEST_SHARED_ISOLATION: &str =
    "cpu: user isolation paired roots: forced for self-test; shared proof retained: rdcl_no";
const FIXTURE_SHIPPED_SHARED_ISOLATION: &str =
    "cpu: user isolation shared kernel mappings: rdcl_no";
const FIXTURE_QEMU_ISOLATION: &str = "cpu: user isolation shared kernel mappings: qemu_tcg";

fn storage_records() -> String {
    format!(
        "{RETIRED_TRANSLATION}{KEPT_TRANSLATION}[info ] nvme: up \u{2014} 8 entries a queue, 4096 blocks of 512 bytes, doorbells 4 apart\n"
    )
}

fn selftest_transcript_with_isolation(isolation: &str) -> String {
    let expected = expected_selftest_checks(repository_root()).expect("current check count");
    let mut transcript = String::new();
    // The 2026-10-20 VMware self-test boot's order: storage, then user isolation, then the APIC.
    for marker in [
        "iommu: entry DMA fence retained VMware 440BX 00:00.0 as the qualified non-requester",
        VMWARE_ISA_DMA_MARKER,
        "cpu: protections wp smep smap umip nx",
        "cpu: extended state 832 bytes, xcr0 0x7, compacted",
    ] {
        transcript.push_str("[info ] ");
        transcript.push_str(marker);
        transcript.push('\n');
    }
    transcript.push_str(&storage_records());
    transcript.push_str("[info ] ");
    transcript.push_str(isolation);
    transcript.push('\n');
    transcript.push_str("[info ] apic: x2apic id 0 version 0x14 (bootstrap)\n");
    for marker in [
        "selftest: a thread crosses into ring three and calls back",
        "selftest: awaiting storage-delay keystroke",
        SELFTEST_DELAYED_READ_MARKER,
        "selftest: the kernel reads the file the host wrote",
        "selftest: both calibrated rates are believable",
        "selftest: awaiting injected keystroke",
        "selftest: an injected keystroke arrives, and its release follows",
        "selftest: awaiting translated keystroke",
        "selftest: a translated keystroke decodes to the same key",
    ] {
        transcript.push_str("[info ] ");
        transcript.push_str(marker);
        transcript.push('\n');
    }
    transcript.push_str(&format!(
        "[info ] selftest: verdict v1 PASS passed={expected} failed=0 expected={expected}\n"
    ));
    transcript
}

fn selftest_transcript() -> String {
    selftest_transcript_with_isolation(FIXTURE_PAIRED_ISOLATION)
}

#[test]
fn the_delayed_read_marker_matches_the_unique_kernel_format_literal() {
    let kernel = crate::contracts::module_source(repository_root(), "kernel/src/selftest.rs");
    let prefix = "\"selftest: delayed read evidence ";
    let occurrences = kernel.match_indices(prefix).collect::<Vec<_>>();
    assert_eq!(
        occurrences.len(),
        1,
        "the kernel must own one delayed-read evidence format"
    );
    let format = &kernel[occurrences[0].0 + 1..];
    let format = &format[..format
        .find('"')
        .expect("the delayed-read evidence format must terminate")];
    let success = format
        .replace("{deferred}", "true")
        .replace("{reached_device}", "true")
        .replace("{retained}", "true")
        .replace("{input_progress}", "true")
        .replace("{completed}", "true")
        .replace("{exact}", "true");
    assert!(
        !success.contains('{') && !success.contains('}'),
        "every delayed-read evidence placeholder must be rendered"
    );
    assert_eq!(success, SELFTEST_DELAYED_READ_MARKER);
}

fn shipped_transcript_with_isolation(isolation: &str) -> String {
    // The 2026-10-20 VMware shipped boot's order: the namespace survey precedes user isolation.
    let mut transcript = format!(
        "[info ] iommu: entry DMA fence retained VMware 440BX 00:00.0 as the qualified non-requester\n[info ] {VMWARE_ISA_DMA_MARKER}\n"
    );
    transcript.push_str(&storage_records());
    for marker in [
        "fat32: mounted \u{2014} 129046 clusters of 512 bytes",
        "tmpfs: 1 file(s) and 1 directory(ies) at the root",
        "devfs: storage is a block device",
        "devfs: console is a character device",
        isolation,
        "user: program running in slot 0",
        "kernel: reading the keyboard, uptime 1 ms",
        "user: ring 3 call observed",
    ] {
        transcript.push_str("[info ] ");
        transcript.push_str(marker);
        transcript.push('\n');
    }
    transcript
}

fn shipped_transcript() -> String {
    shipped_transcript_with_isolation(FIXTURE_PAIRED_ISOLATION)
}

#[test]
fn the_vmware_dma_handoff_marker_is_required_by_both_selectors() {
    for markers in [SELFTEST_MARKERS, SHIPPED_MARKERS] {
        assert_eq!(
            markers
                .iter()
                .filter(|marker| matches!(
                    marker,
                    TranscriptMarker::Exact(message)
                        if *message == VMWARE_DMA_HANDOFF_MARKER
                ))
                .count(),
            1,
            "each VMware selector must require exactly one consumed DMA policy witness"
        );
    }

    let kernel = std::fs::read_to_string(repository_root().join("kernel/src/main.rs"))
        .expect("reading kernel entry source");
    assert!(
        kernel.contains(VMWARE_DMA_HANDOFF_MARKER),
        "the verifier marker must remain the kernel's exact emitted record"
    );

    let root = repository_root();
    let record = format!("[info ] {VMWARE_DMA_HANDOFF_MARKER}\n");
    for (selector, transcript) in [
        (Kernel::SelfTest, selftest_transcript()),
        (Kernel::Shipped, shipped_transcript()),
    ] {
        verify_transcript(root, selector, transcript.as_bytes())
            .expect("canonical VMware DMA witness");
        let omitted = transcript.replacen(&record, "", 1);
        assert!(
            verify_transcript(root, selector, omitted.as_bytes()).is_err(),
            "selector accepted a transcript without its VMware DMA witness"
        );
    }
}

#[test]
fn the_vmware_isa_dma_marker_is_required_by_both_selectors() {
    for markers in [SELFTEST_MARKERS, SHIPPED_MARKERS] {
        assert_eq!(
            markers
                .iter()
                .filter(|marker| matches!(
                    marker,
                    TranscriptMarker::Exact(message) if *message == VMWARE_ISA_DMA_MARKER
                ))
                .count(),
            1,
            "each VMware selector must require exactly one quiesced-requester witness"
        );
    }

    let kernel = std::fs::read_to_string(repository_root().join("kernel/src/main.rs"))
        .expect("reading kernel entry source");
    assert!(
        kernel.contains(VMWARE_ISA_DMA_MARKER),
        "the verifier marker must remain the kernel's exact emitted record"
    );

    let root = repository_root();
    let record = format!("[info ] {VMWARE_ISA_DMA_MARKER}\n");
    for (selector, transcript) in [
        (Kernel::SelfTest, selftest_transcript()),
        (Kernel::Shipped, shipped_transcript()),
    ] {
        verify_transcript(root, selector, transcript.as_bytes())
            .expect("canonical VMware quiesce witness");
        let omitted = transcript.replacen(&record, "", 1);
        assert!(
            verify_transcript(root, selector, omitted.as_bytes()).is_err(),
            "selector accepted a transcript without its VMware quiesce witness"
        );
    }
}

/// The transcript with two markers' records exchanged.
fn swapped(transcript: &str, first: TranscriptMarker, second: TranscriptMarker) -> String {
    let mut lines: Vec<&str> = transcript.lines().collect();
    let at = |marker: TranscriptMarker| {
        let record = format!("[info ] {}", marker.example());
        lines
            .iter()
            .position(|line| *line == record)
            .expect("marker record")
    };
    let (first, second) = (at(first), at(second));
    lines.swap(first, second);
    format!("{}\n", lines.join("\n"))
}

#[test]
fn selftest_verdict_counts_use_one_canonical_decimal_grammar() {
    let root = repository_root();
    let valid = selftest_transcript();
    let expected = expected_selftest_checks(root).unwrap();
    verify_transcript(root, Kernel::SelfTest, valid.as_bytes()).unwrap();

    for forged in [
        valid.replace(
            &format!("passed={expected}"),
            &format!("passed=0{expected}"),
        ),
        valid.replace("failed=0", "failed=+0"),
        valid.replace(
            &format!("expected={expected}"),
            &format!("expected=0{expected}"),
        ),
        valid.replace(&format!("passed={expected}"), "passed=18446744073709551616"),
        valid.replace(&format!("passed={expected}"), "passed=\u{0661}"),
    ] {
        assert!(verify_transcript(root, Kernel::SelfTest, forged.as_bytes()).is_err());
    }
}

#[test]
fn vmware_selftest_verdict_requires_single_ascii_spaces() {
    let root = repository_root();
    let valid = selftest_transcript();
    verify_transcript(root, Kernel::SelfTest, valid.as_bytes()).unwrap();
    let verdict = format!("{}\n", valid.lines().last().expect("verdict record"));

    for forged in [
        valid.replace("verdict v1 PASS", "verdict v1  PASS"),
        valid.replace("verdict v1 PASS", "verdict v1\tPASS"),
        valid.replace("verdict v1 PASS", "verdict v1\u{a0}PASS"),
        valid.replace(" failed=0", "  failed=0"),
        valid.replacen(&verdict, &verdict.replace('\n', " \n"), 1),
        valid.replacen(&verdict, &verdict.replace('\n', "\t\n"), 1),
        valid.replacen(&verdict, &verdict.replace('\n', "\u{a0}\n"), 1),
        valid.trim_end_matches('\n').to_string(),
    ] {
        assert!(
            verify_transcript(root, Kernel::SelfTest, forged.as_bytes()).is_err(),
            "accepted noncanonical verdict spacing: {forged:?}"
        );
    }
}

#[test]
fn oversized_source_counts_are_refused_before_text_allocation() {
    let root = crate::test_support::reserve_temp_directory("wizard-vmware-source-bound");
    let path = root.join("selftest.rs");
    File::create(&path)
        .unwrap()
        .set_len(MAX_SOURCE_BYTES + 1)
        .unwrap();

    let error = source_usize(
        std::slice::from_ref(&path),
        "const EXPECTED_CHECKS: usize = ",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("exceeding the 4194304-byte allocation limit"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn vmware_isolation_markers_are_bound_to_kernel_producers() {
    let kernel = std::fs::read_to_string(repository_root().join("kernel/src/main.rs"))
        .expect("reading kernel entry source");
    let forced = VMWARE_SELFTEST_SHARED_ISOLATION_MARKER.replace("rdcl_no", "{reason}");
    let shared = VMWARE_SHIPPED_SHARED_ISOLATION_MARKER.replace("rdcl_no", "{reason}");

    for producer in [
        format!("\"{VMWARE_PAIRED_ISOLATION_MARKER}\""),
        format!("\"{forced}\""),
        format!("info!(\"{shared}\")"),
    ] {
        assert!(
            kernel.contains(&producer),
            "the verifier marker is not bound to kernel producer {producer}"
        );
    }
}

#[test]
fn vmware_isolation_records_are_selector_specific_exact_and_unique() {
    let root = repository_root();
    for (kernel, transcript) in [
        (
            Kernel::SelfTest,
            selftest_transcript_with_isolation(FIXTURE_PAIRED_ISOLATION),
        ),
        (
            Kernel::SelfTest,
            selftest_transcript_with_isolation(FIXTURE_SELFTEST_SHARED_ISOLATION),
        ),
        (
            Kernel::Shipped,
            shipped_transcript_with_isolation(FIXTURE_PAIRED_ISOLATION),
        ),
        (
            Kernel::Shipped,
            shipped_transcript_with_isolation(FIXTURE_SHIPPED_SHARED_ISOLATION),
        ),
    ] {
        verify_transcript(root, kernel, transcript.as_bytes())
            .expect("selector-appropriate isolation evidence");
    }

    let selftest = selftest_transcript();
    let shipped = shipped_transcript();
    let paired = format!("[info ] {FIXTURE_PAIRED_ISOLATION}\n");
    let selftest_shared = format!("[info ] {FIXTURE_SELFTEST_SHARED_ISOLATION}\n");
    let shipped_shared = format!("[info ] {FIXTURE_SHIPPED_SHARED_ISOLATION}\n");
    let qemu = format!("[info ] {FIXTURE_QEMU_ISOLATION}\n");
    let unknown = "[info ] cpu: user isolation invented native claim\n";
    let trailing = format!("[info ] {FIXTURE_PAIRED_ISOLATION} trailing\n");
    let warning = format!("[warn ] {FIXTURE_PAIRED_ISOLATION}\n");

    let rejected = [
        (
            Kernel::SelfTest,
            selftest_transcript_with_isolation(FIXTURE_SHIPPED_SHARED_ISOLATION),
        ),
        (
            Kernel::Shipped,
            shipped_transcript_with_isolation(FIXTURE_SELFTEST_SHARED_ISOLATION),
        ),
        (
            Kernel::SelfTest,
            selftest_transcript_with_isolation(FIXTURE_QEMU_ISOLATION),
        ),
        (
            Kernel::Shipped,
            shipped_transcript_with_isolation(FIXTURE_QEMU_ISOLATION),
        ),
        (Kernel::SelfTest, selftest.replacen(&paired, "", 1)),
        (Kernel::Shipped, shipped.replacen(&paired, "", 1)),
        (Kernel::SelfTest, selftest.replacen(&paired, unknown, 1)),
        (Kernel::SelfTest, selftest.replacen(&paired, &trailing, 1)),
        (Kernel::SelfTest, selftest.replacen(&paired, &warning, 1)),
        (
            Kernel::SelfTest,
            selftest.replacen(&paired, &format!("{paired}{paired}"), 1),
        ),
        (
            Kernel::SelfTest,
            selftest.replacen(&paired, &format!("{paired}{selftest_shared}"), 1),
        ),
        (
            Kernel::Shipped,
            shipped.replacen(&paired, &format!("{paired}{shipped_shared}"), 1),
        ),
        (
            Kernel::SelfTest,
            selftest.replacen(&paired, &format!("{paired}{qemu}"), 1),
        ),
    ];
    for (kernel, transcript) in rejected {
        assert!(
            verify_transcript(root, kernel, transcript.as_bytes()).is_err(),
            "accepted missing, ambiguous, cross-selector or inexact isolation evidence"
        );
    }
}

#[test]
fn uart_crlf_and_lf_are_both_valid_transcript_terminators() {
    let root = repository_root();
    for (kernel, transcript) in [
        (Kernel::SelfTest, selftest_transcript()),
        (Kernel::Shipped, shipped_transcript()),
    ] {
        verify_transcript(root, kernel, transcript.as_bytes()).unwrap();
        verify_transcript(root, kernel, transcript.replace('\n', "\r\n").as_bytes()).unwrap();
    }
}

#[test]
fn only_a_carriage_return_paired_with_line_feed_is_allowed() {
    let root = repository_root();
    let valid = selftest_transcript();
    for poisoned in [
        valid.replacen("[info ]", "[info\r ]", 1),
        format!("\r{valid}"),
        valid.replace('\n', "\r\r\n"),
        format!("{valid}\r"),
    ] {
        assert!(verify_transcript(root, Kernel::SelfTest, poisoned.as_bytes()).is_err());
    }
}

#[test]
fn a_vmware_selftest_transcript_is_complete_current_and_unambiguous() {
    let root = repository_root();
    let valid = selftest_transcript();
    assert!(verify_transcript(root, Kernel::SelfTest, valid.as_bytes()).is_ok());

    for marker in SELFTEST_MARKERS {
        let omitted = valid.replacen(marker.example(), "omitted", 1);
        assert!(
            verify_transcript(root, Kernel::SelfTest, omitted.as_bytes()).is_err(),
            "omitting {} was accepted",
            marker.example()
        );
    }
    let marker_record = format!("[info ] {}\n", SELFTEST_MARKERS[0].example());
    let duplicated_marker = valid.replacen(
        &marker_record,
        &format!("{marker_record}{marker_record}"),
        1,
    );
    assert!(verify_transcript(root, Kernel::SelfTest, duplicated_marker.as_bytes()).is_err());
    let duplicated = format!("{valid}{}", valid.lines().last().expect("verdict line"));
    assert!(verify_transcript(root, Kernel::SelfTest, duplicated.as_bytes()).is_err());
    let reordered = swapped(&valid, SELFTEST_MARKERS[4], SELFTEST_MARKERS[5]);
    assert!(verify_transcript(root, Kernel::SelfTest, reordered.as_bytes()).is_err());
    let split_compound = valid.replace(", compacted", "\n[info ] , compacted");
    assert!(verify_transcript(root, Kernel::SelfTest, split_compound.as_bytes()).is_err());
    let merged_steps = valid.replace(
        "\n[info ] selftest: a thread crosses",
        " [info ] selftest: a thread crosses",
    );
    assert!(verify_transcript(root, Kernel::SelfTest, merged_steps.as_bytes()).is_err());
    let expected = expected_selftest_checks(root).unwrap();
    let stale = valid.replace(
        &format!("passed={expected}"),
        &format!("passed={}", expected - 1),
    );
    assert!(verify_transcript(root, Kernel::SelfTest, stale.as_bytes()).is_err());
    let panicked = format!("{valid}[error] KERNEL PANIC\n");
    assert!(verify_transcript(root, Kernel::SelfTest, panicked.as_bytes()).is_err());
    let failed = format!("{valid}[error] selftest: injected defect FAILED\n");
    assert!(verify_transcript(root, Kernel::SelfTest, failed.as_bytes()).is_err());
    for poisoned in [
        valid.trim_end_matches('\n').to_string(),
        valid.replacen("[info ]", "[info\r ]", 1),
        valid.replacen("[info ]", "[info\u{1b} ]", 1),
        valid.replacen("[info ]", "[info\u{8} ]", 1),
    ] {
        assert!(
            verify_transcript(root, Kernel::SelfTest, poisoned.as_bytes()).is_err(),
            "accepted a partial or terminal-active transcript: {poisoned:?}"
        );
    }
    let mut invalid_utf8 = b"[info ] invalid byte: ".to_vec();
    invalid_utf8.push(0xff);
    invalid_utf8.push(b'\n');
    invalid_utf8.extend_from_slice(valid.as_bytes());
    assert!(verify_transcript(root, Kernel::SelfTest, &invalid_utf8).is_err());
}

#[test]
fn a_nul_transcript_is_rejected_before_canonical_text_validation() {
    let root = repository_root();
    let with_nul = format!("{}\0", selftest_transcript());
    let canonical_validation_started = std::cell::Cell::new(false);

    assert!(
        verify_transcript_with(
            Kernel::SelfTest,
            with_nul.as_bytes(),
            || {
                canonical_validation_started.set(true);
                Ok(())
            },
            || expected_selftest_checks(root),
        )
        .is_err()
    );
    assert!(!canonical_validation_started.get());
}

const CAPTURED_SELFTEST: &[u8] =
    include_bytes!("../../fixtures/vmware/2026-10-24-8f4586a-self-test.serial");
const CAPTURED_SHIPPED: &[u8] =
    include_bytes!("../../fixtures/vmware/2026-10-24-8f4586a-shipped.serial");
const CAPTURED_SELFTEST_CHECKS: usize = 294;

#[test]
fn real_vmware_captures_satisfy_only_their_own_selector() {
    let captured = || Ok(CAPTURED_SELFTEST_CHECKS);
    verify_transcript_with(Kernel::SelfTest, CAPTURED_SELFTEST, || Ok(()), captured).unwrap();
    verify_transcript_with(Kernel::Shipped, CAPTURED_SHIPPED, || Ok(()), captured).unwrap();
    assert!(
        verify_transcript_with(Kernel::SelfTest, CAPTURED_SHIPPED, || Ok(()), captured).is_err()
    );
    assert!(
        verify_transcript_with(Kernel::Shipped, CAPTURED_SELFTEST, || Ok(()), captured).is_err()
    );
    assert!(
        verify_transcript_with(
            Kernel::SelfTest,
            CAPTURED_SELFTEST,
            || Ok(()),
            || { Ok(CAPTURED_SELFTEST_CHECKS + 1) }
        )
        .is_err()
    );
}

#[test]
fn a_shipped_vmware_transcript_proves_liveness_without_selftest_substitution() {
    let root = repository_root();
    let valid = shipped_transcript();
    assert!(verify_transcript(root, Kernel::Shipped, valid.as_bytes()).is_ok());

    for marker in SHIPPED_MARKERS {
        let omitted = valid.replacen(marker.example(), "omitted", 1);
        assert!(
            verify_transcript(root, Kernel::Shipped, omitted.as_bytes()).is_err(),
            "omitting {} was accepted",
            marker.example()
        );
    }
    let reordered = swapped(&valid, SHIPPED_MARKERS[0], SHIPPED_MARKERS[1]);
    assert!(verify_transcript(root, Kernel::Shipped, reordered.as_bytes()).is_err());
    let selftest = format!("{valid}selftest: verdict v1 PASS passed=1 failed=0 expected=1\n");
    assert!(verify_transcript(root, Kernel::Shipped, selftest.as_bytes()).is_err());
    let refusal = format!("{valid}user: no program: refused\n");
    assert!(verify_transcript(root, Kernel::Shipped, refusal.as_bytes()).is_err());
}

#[test]
fn shipped_vmware_requires_execution_after_entry_not_launch_announcements() {
    let root = repository_root();
    let valid = shipped_transcript();
    let record = "[info ] user: ring 3 call observed\n";
    for replacement in [
        "",
        "[warn ] user: ring 3 call observed\n",
        "[info ] user: ring 3 call observed trailing\n",
        "[info ] prefix user: ring 3 call observed\n",
        "[info ] user 0: entering ring 3 at 0x400000000000 with 0 on 0x1000\n",
        "[info ] user: ring 3 call observed\n[info ] user: ring 3 call observed\n",
    ] {
        let forged = valid.replacen(record, replacement, 1);
        assert!(
            verify_transcript(root, Kernel::Shipped, forged.as_bytes()).is_err(),
            "accepted {replacement:?}"
        );
    }
    assert!(verify_transcript(root, Kernel::Shipped, valid.as_bytes()).is_ok());
    let rescheduled = format!("{record}{}", valid.replacen(record, "", 1));
    assert!(verify_transcript(root, Kernel::Shipped, rescheduled.as_bytes()).is_ok());
}

#[test]
fn vmware_evidence_requires_canonical_info_records() {
    let root = repository_root();
    for (kernel, valid, markers) in [
        (Kernel::SelfTest, selftest_transcript(), SELFTEST_MARKERS),
        (Kernel::Shipped, shipped_transcript(), SHIPPED_MARKERS),
    ] {
        for marker in markers {
            let canonical = format!("[info ] {}", marker.example());
            for forged in [
                valid.replacen(&canonical, &format!("[warn ] {}", marker.example()), 1),
                valid.replacen(
                    &canonical,
                    &format!("[info ] forged {}", marker.example()),
                    1,
                ),
                valid.replacen(&canonical, &format!("{canonical} trailing"), 1),
            ] {
                assert!(
                    verify_transcript(root, kernel, forged.as_bytes()).is_err(),
                    "accepted a noncanonical record for {}",
                    marker.example()
                );
            }
        }

        let first = format!("\n[info ] {}", markers[1].example());
        let merged = valid.replacen(&first, &format!(" [info ] {}", markers[1].example()), 1);
        assert!(
            verify_transcript(root, kernel, merged.as_bytes()).is_err(),
            "accepted two execution records on one line"
        );
    }

    let valid = selftest_transcript();
    for forged in [
        valid.replacen("[info ] selftest: verdict", "[warn ] selftest: verdict", 1),
        valid.replacen(
            "[info ] selftest: verdict",
            "[info ] forged selftest: verdict",
            1,
        ),
    ] {
        assert!(
            verify_transcript(root, Kernel::SelfTest, forged.as_bytes()).is_err(),
            "accepted a noncanonical self-test verdict"
        );
    }
}

#[test]
fn a_vmware_transcript_without_translation_is_refused() {
    let root = repository_root();
    let storage = format!("[info ] {}\n", TranscriptMarker::Nvme.example());
    for (kernel, valid) in [
        (Kernel::SelfTest, selftest_transcript()),
        (Kernel::Shipped, shipped_transcript()),
    ] {
        let untranslated = valid.replacen(&storage_records(), &storage, 1);
        let error = verify_transcript(root, kernel, untranslated.as_bytes()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no canonical `iommu: translation on` record"),
            "{error}"
        );
    }
}

#[test]
fn a_vmware_translation_after_storage_is_refused() {
    let root = repository_root();
    let storage = format!("[info ] {}\n", TranscriptMarker::Nvme.example());
    // The retired candidate's record still precedes storage, so only the last record can decide.
    let late = format!("{RETIRED_TRANSLATION}{storage}{KEPT_TRANSLATION}");
    for (kernel, valid) in [
        (Kernel::SelfTest, selftest_transcript()),
        (Kernel::Shipped, shipped_transcript()),
    ] {
        let reordered = valid.replacen(&storage_records(), &late, 1);
        let error = verify_transcript(root, kernel, reordered.as_bytes()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("translation on after `nvme: up`"),
            "{error}"
        );
    }
}

#[test]
fn two_vmware_translations_before_storage_qualify() {
    let root = repository_root();
    let storage = format!("[info ] {}\n", TranscriptMarker::Nvme.example());
    let retirement = "[info ] nvme: candidate 1 retired cleanly; trying next\n";
    let separated = format!("{RETIRED_TRANSLATION}{retirement}{KEPT_TRANSLATION}{storage}");
    for (kernel, valid) in [
        (Kernel::SelfTest, selftest_transcript()),
        (Kernel::Shipped, shipped_transcript()),
    ] {
        assert!(valid.contains(&storage_records()));
        verify_transcript(root, kernel, valid.as_bytes()).unwrap();
        let spaced = valid.replacen(&storage_records(), &separated, 1);
        verify_transcript(root, kernel, spaced.as_bytes()).unwrap();
    }
}

#[test]
fn a_warning_translation_record_does_not_count() {
    let root = repository_root();
    let storage = format!("[info ] {}\n", TranscriptMarker::Nvme.example());
    let retired = RETIRED_TRANSLATION.replacen("[info ]", "[warn ]", 1);
    let kept = KEPT_TRANSLATION.replacen("[info ]", "[warn ]", 1);
    let warned = format!("{retired}{kept}{storage}");
    for (kernel, valid) in [
        (Kernel::SelfTest, selftest_transcript()),
        (Kernel::Shipped, shipped_transcript()),
    ] {
        let demoted = valid.replacen(&storage_records(), &warned, 1);
        let error = verify_transcript(root, kernel, demoted.as_bytes()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no canonical `iommu: translation on` record"),
            "{error}"
        );

        let late_warning = valid.replacen(&storage, &format!("{storage}{kept}"), 1);
        verify_transcript(root, kernel, late_warning.as_bytes()).unwrap();
    }
}

#[test]
fn selftest_and_shipped_vms_do_not_share_machine_state() {
    let paths = Paths::new(PathBuf::from("/repo"), None);
    assert_eq!(
        vm_directory(&paths, Kernel::SelfTest),
        paths.vm_dir().join("self-test")
    );
    assert_eq!(
        vm_directory(&paths, Kernel::Shipped),
        paths.vm_dir().join("shipped")
    );
}

#[test]
fn a_generated_vm_creates_one_empty_serial_transcript() {
    let root = crate::test_support::reserve_temp_directory("wizard-vmware");
    let log = root.join(format!("serial-{}.log", "1".repeat(64)));

    write_new(&log, &[]).expect("fresh transcript");

    assert_eq!(std::fs::read(&log).unwrap(), b"");
    assert!(
        write_new(&log, &[]).is_err(),
        "a second generation reused the transcript path"
    );
    std::fs::remove_dir_all(root).expect("remove VMware fixture");
}

#[test]
fn generation_replaces_only_its_exact_selector_directory() {
    let root = crate::test_support::reserve_temp_directory("wizard-vmware-reset");
    let repository = root.join("repository");
    let paths = Paths::new(repository.clone(), None);
    let selftest = vm_directory(&paths, Kernel::SelfTest);
    let shipped = vm_directory(&paths, Kernel::Shipped);
    std::fs::create_dir_all(&selftest).expect("self-test selector");
    std::fs::create_dir_all(&shipped).expect("shipped selector");
    std::fs::write(selftest.join("nvram"), "stale firmware state").unwrap();
    std::fs::write(selftest.join("bedrock.vmss"), "stale suspend state").unwrap();
    std::fs::write(shipped.join("keep"), "other selector").unwrap();

    reset_vm_directory(&paths, Kernel::SelfTest).expect("reset self-test selector");

    assert!(selftest.is_dir());
    assert!(
        crate::paths::directory_entries(&selftest)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read_to_string(shipped.join("keep")).unwrap(),
        "other selector"
    );
    std::fs::remove_dir_all(root).expect("remove VMware reset fixture");
}

#[test]
fn a_redirected_repository_ancestor_is_refused_before_vmware_root_creation() {
    let root = crate::test_support::reserve_temp_directory("wizard-vmware-ancestor");
    let outside_parent = root.join("outside-parent");
    let outside_repository = outside_parent.join("repository");
    let redirected_parent = root.join("redirected-parent");
    std::fs::create_dir_all(&outside_repository).expect("outside repository");
    create_directory_symlink(&outside_parent, &redirected_parent);
    let paths = Paths::new(redirected_parent.join("repository"), None);

    assert!(reset_vm_directory(&paths, Kernel::SelfTest).is_err());
    assert!(
        !outside_repository.join("vm").exists(),
        "VMware root was created through a redirected repository ancestor"
    );
    std::fs::remove_dir_all(root).expect("remove redirected VMware fixture");
}

#[test]
fn destructive_vmware_paths_require_one_canonical_parent() {
    let root = crate::test_support::reserve_temp_directory("wizard-vmware-canonical");
    let parent = root.join("parent");
    let child = parent.join("child");
    let sibling = root.join("sibling");
    std::fs::create_dir_all(&child).unwrap();
    std::fs::create_dir(&sibling).unwrap();

    require_canonical_child(&parent, &child, "fixture child").unwrap();
    assert!(require_canonical_child(&parent, &sibling, "fixture sibling").is_err());

    let redirected_parent = root.join("redirected-parent");
    create_directory_symlink(&parent, &redirected_parent);
    assert!(
        require_canonical_child(
            &redirected_parent,
            &redirected_parent.join("child"),
            "redirected fixture child"
        )
        .is_err()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn generation_invalidates_success_and_receipt_before_any_fallible_preflight() {
    let dispatch = include_str!("../lib.rs");
    assert!(
        dispatch.contains("Task::QualifyVmware { kernel } => vmware::qualify(&paths, kernel),")
    );
    let vmware = dispatch.find("Task::Vmware { profile, kernel }").unwrap();
    let qualify = dispatch[vmware..].find("Task::QualifyVmware").unwrap() + vmware;
    let arm = &dispatch[vmware..qualify];
    let invalidate_pass = arm.find("qualification::invalidate_vmware").unwrap();
    let invalidate_receipt = arm.find("vmware::invalidate_generation").unwrap();
    let build = arm.find("image::build").unwrap();
    let generate = arm.find("vmware::generate").unwrap();
    assert!(invalidate_pass < invalidate_receipt && invalidate_receipt < build && build < generate);

    let source = include_str!("../vmware.rs");
    let generate = source.find("pub fn generate").unwrap();
    let qualify = source[generate..].find("pub(crate) fn qualify").unwrap() + generate;
    let body = &source[generate..qualify];
    let invalidate_pass = body
        .find("qualification::invalidate_vmware(paths, kernel)")
        .unwrap();
    let invalidate_receipt = body.find("invalidate_generation(paths, kernel)").unwrap();
    let preflight = body.find("qualification::vmrun").unwrap();
    let reset = body.find("write_new(&serial_log, &[])").unwrap();
    let receipt = body
        .find("qualification::commit_vmware_generation")
        .unwrap();
    assert!(
        invalidate_pass < invalidate_receipt
            && invalidate_receipt < preflight
            && preflight < reset
            && reset < receipt
    );
}

#[test]
fn geometry_rounds_cylinders_up() {
    // Ceiling division keeps the geometry large enough for the final partial cylinder.
    assert_eq!(geometry(IMAGE_SECTORS).cylinders, 9);
    assert_eq!(geometry(1).cylinders, 1);
    assert_eq!(geometry(HEADS * SECTORS_PER_TRACK).cylinders, 1);
    assert_eq!(geometry(HEADS * SECTORS_PER_TRACK + 1).cylinders, 2);
}

#[test]
fn geometry_always_covers_the_extent() {
    for sectors in [1, 63, 16_065, 16_066, 131_072, 2_097_152] {
        let g = geometry(sectors);
        let covered = g.cylinders * g.heads * g.sectors_per_track;
        assert!(covered >= sectors, "{sectors} sectors not covered by {g:?}");
        let previous = (g.cylinders - 1) * g.heads * g.sectors_per_track;
        assert!(previous < sectors, "{g:?} is larger than it needs to be");
    }
}

#[test]
fn render_substitutes_every_placeholder() {
    let vmdk = render(
        VMDK_TEMPLATE,
        &[
            ("SECTORS", "131072".to_string()),
            ("FLAT_NAME", FLAT_NAME.to_string()),
            ("CYLINDERS", "9".to_string()),
            ("HEADS", "255".to_string()),
            ("SECTORS_PER_TRACK", "63".to_string()),
        ],
    )
    .unwrap();

    assert!(!vmdk.contains("{{"));
    assert!(vmdk.contains(r#"RW 131072 FLAT "bedrock-flat.img" 0"#));
    assert!(vmdk.contains(r#"ddb.geometry.cylinders = "9""#));
}

#[test]
fn render_rejects_a_placeholder_it_was_not_given() {
    let error = render("hello {{NAME}}", &[]).unwrap_err().to_string();
    assert!(error.contains("NAME"), "{error}");
}

#[test]
fn render_does_not_rescan_substituted_text() {
    let rendered = render(
        "{{A}}{{B}}",
        &[("A", "{{B}}".to_string()), ("B", "value".to_string())],
    )
    .unwrap();
    assert_eq!(rendered, "{{B}}value");
}

#[test]
fn the_vmx_boots_efi_and_logs_serial_to_a_file() {
    let serial = format!("serial-{}.log", "1".repeat(64));
    let vmx = render(
        VMX_TEMPLATE,
        &[
            ("DISPLAY_NAME", DISPLAY_NAME.to_string()),
            ("VMDK_NAME", VMDK_NAME.to_string()),
            ("NVME_VMDK_NAME", NVME_VMDK_NAME.to_string()),
            (
                "DMA_HANDOFF_FILE",
                crate::platform_acpi::DMA_HANDOFF_FILE.to_string(),
            ),
            ("SERIAL_LOG", serial.clone()),
        ],
    )
    .unwrap();

    assert!(vmx.contains(r#"firmware = "efi""#));
    // Storage fails closed on a machine with no remapping unit.
    assert!(vmx.contains(r#"vvtd.enable = "TRUE""#));
    assert!(vmx.contains(r#"acpi.addtable.filename = "dma-handoff.bin""#));
    assert!(vmx.contains(&format!(r#"serial0.fileName = "{serial}""#)));
    assert!(vmx.contains(r#"sata0:0.fileName = "bedrock.vmdk""#));
    // A namespace-less candidate before the usable controller makes VMware retirement observable.
    assert!(vmx.contains(r#"nvme0.present = "TRUE""#));
    assert!(
        !vmx.contains(r#"nvme0:0.present"#),
        "the first controller must have no namespace to retire"
    );
    assert!(vmx.contains(r#"nvme1.present = "TRUE""#));
    assert!(vmx.contains(r#"nvme1:0.fileName = "wizard-nvm.vmdk""#));
    // A PCIe-only controller in a machine with no root ports is refused before the guest looks.
    assert!(vmx.contains(r#"pciBridge4.virtualDev = "pcieRootPort""#));
}
