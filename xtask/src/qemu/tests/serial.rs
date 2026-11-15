use super::*;

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
