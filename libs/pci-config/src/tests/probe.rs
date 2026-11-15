use super::*;

#[test]
fn a_wide_bar_in_the_final_slot_is_refused_before_mutation() {
    let mut fake = FakeConfig::device();
    fake.bars[0] = 0x0000_C001;
    fake.bars[1] = 0x0000_0004;
    let original = fake.clone();

    let refused = probe_bars_unleased(BRIDGE_BARS, |event| fake.access(event))
        .map(drop)
        .map_err(|failure| failure.reason);
    assert_eq!(refused, Err(ProbeError::Malformed));
    assert_eq!(fake.command, original.command);
    assert_eq!(fake.status, original.status);
    assert_eq!(fake.bars, original.bars);
    assert!(fake.trace.iter().all(|event| !matches!(
        event,
        ProbeAccess::WriteCommand(_) | ProbeAccess::WriteBar(_, _)
    )));
}
