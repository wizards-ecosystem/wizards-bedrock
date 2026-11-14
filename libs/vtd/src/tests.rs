use super::*;

#[test]
fn takeover_requires_every_firmware_facility_to_be_inactive() {
    assert!(takeover_ready(0));
    assert!(takeover_ready(gsts::RTPS | gsts::IRTPS));
    assert!(!takeover_ready(gsts::TES));
    assert!(!takeover_ready(gsts::QIES));
    assert!(!takeover_ready(gsts::IRES));
    assert!(!takeover_ready(gsts::CFIS));
    assert!(!takeover_ready(u32::MAX));
}
