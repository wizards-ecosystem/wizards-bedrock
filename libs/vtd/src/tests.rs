use super::*;

#[test]
fn takeover_requires_every_firmware_facility_to_be_inactive() {
    assert!(takeover_ready(0));
    assert!(takeover_ready(global_status::RTPS | global_status::IRTPS));
    assert!(!takeover_ready(global_status::TES));
    assert!(!takeover_ready(global_status::QIES));
    assert!(!takeover_ready(global_status::IRES));
    assert!(!takeover_ready(global_status::CFIS));
    assert!(!takeover_ready(u32::MAX));
}
