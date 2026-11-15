use super::serial::{SELFTEST_PASSED, SELFTEST_REQUIRED, Selftest, Verdict};

const PASSING: &[&str] = &[
    "[info ] selftest: nvme-stack v2 used=22000 capacity=32768 headroom=10768 leaf=21000",
    "[info ] selftest: verdict v1 PASS passed=211 failed=0 expected=211",
];

#[test]
fn malformed_or_unknown_verdicts_are_rejected() {
    for line in [
        "[info ] selftest: verdict v2 PASS passed=76 failed=0 expected=76",
        "[info ] selftest: verdict v1 MAYBE passed=76 failed=0 expected=76",
        "[info ] selftest: verdict v1 PASS passed=many failed=0 expected=76",
        "[info ] selftest: verdict v1 PASS passed=76 failed=0 expected=76 trailing",
        "[error] selftest: verdict v1 PASS passed=76 failed=0 expected=76",
    ] {
        assert!(Verdict::parse(line).unwrap().is_err(), "{line}");
    }
}

#[test]
fn a_valid_serial_pass_must_match_debug_exit() {
    let passing = || {
        let mut selftest = Selftest::default();
        for line in SELFTEST_REQUIRED.iter().chain(PASSING) {
            selftest.observe(line).unwrap();
        }
        selftest
    };
    assert!(passing().conclude(Some(SELFTEST_PASSED)).is_ok());
    let error = passing().conclude(Some(35)).unwrap_err();
    assert!(error.to_string().contains("debug-exit"));
}
