use boot_record::{Memory, Record, Verdict};

use super::serial::{SELFTEST_PASSED, SELFTEST_REQUIRED, Selftest};

#[test]
fn a_valid_serial_pass_must_match_debug_exit() {
    let verdict = Record::Verdict(Verdict {
        passed: 76,
        failed: 0,
        expected: 76,
    });
    let colours = Record::Memory(Memory { colours: 32 });
    let passing = || {
        let mut selftest = Selftest::default();
        for record in SELFTEST_REQUIRED.iter().chain([&colours, &verdict]) {
            selftest.observe(&format!("[info ] {record}")).unwrap();
        }
        selftest
    };
    assert!(passing().conclude(Some(SELFTEST_PASSED)).is_ok());
    let error = passing().conclude(Some(35)).unwrap_err();
    assert!(error.to_string().contains("debug-exit"));
}
