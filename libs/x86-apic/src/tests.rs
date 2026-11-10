use super::*;

#[test]
fn a_register_remembers_its_offset() {
    assert_eq!(Register::new(0x0B0).offset(), 0x0B0);
    assert_eq!(Register::new(0).offset(), 0);
}

#[test]
#[should_panic(expected = "16-byte aligned")]
fn a_register_rejects_an_unaligned_offset() {
    let _ = Register::new(0x0B4);
}

#[test]
#[should_panic(expected = "outside the local APIC page")]
fn a_register_rejects_an_offset_past_the_page() {
    let _ = Register::new(REGISTER_BYTES);
}

/// The last aligned slot in the page still has to hold four whole bytes.
#[test]
fn the_last_register_in_the_page_is_accepted() {
    assert_eq!(
        Register::new(REGISTER_BYTES - REGISTER_STRIDE).offset(),
        0xFF0
    );
}

/// Spot values from SDM Vol. 3A table 11-1's "MSR address" column.
#[test]
fn a_register_maps_to_the_msr_the_manual_gives() {
    assert_eq!(Register::new(0x020).msr(), 0x802, "id");
    assert_eq!(Register::new(0x0B0).msr(), 0x80B, "end of interrupt");
    assert_eq!(Register::new(0x0F0).msr(), 0x80F, "spurious");
    assert_eq!(Register::new(0x300).msr(), 0x830, "interrupt command");
    assert_eq!(Register::new(0x370).msr(), 0x837, "lvt error");
}

#[test]
fn consecutive_slots_become_consecutive_registers() {
    let first = Register::new(0);
    let second = Register::new(REGISTER_STRIDE);

    assert_eq!(first.msr(), FIRST_MSR);
    assert_eq!(second.msr(), first.msr() + 1);
}

/// The whole page maps into 0x800 through 0x8FF, the block the architecture reserves.
#[test]
fn no_register_escapes_the_reserved_msr_block() {
    for offset in (0..REGISTER_BYTES).step_by(REGISTER_STRIDE) {
        let msr = Register::new(offset).msr();
        assert!((0x800..=0x8FF).contains(&msr), "offset {offset:#x}");
    }
}
