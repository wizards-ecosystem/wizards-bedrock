use super::*;

/// Set by the CPU on first use unless the descriptor already carries it.
const ACCESSED: u64 = 1 << 40;

/// The two-bit descriptor privilege level, zero for every ring 0 entry.
const PRIVILEGE: u64 = 0b11 << 45;

/// SDM Vol. 3A figure 8-4: bits 55:52 stay clear, since granularity would put `iomap_base` inside.
#[test]
fn a_task_state_descriptor_sets_no_size_or_granularity_bits() {
    const SIZE: u16 = 104;

    let descriptor = SystemDescriptor::task_state(0x1234_5678_9ABC_DEF0, SIZE);

    assert_eq!(descriptor.low & 0xFFFF, u64::from(SIZE - 1));

    for (bit, what) in [
        (52, "available-for-software"),
        (53, "reserved"),
        (54, "default/big"),
        (55, "granularity"),
    ] {
        assert_eq!(
            descriptor.low & (1 << bit),
            0,
            "bit {bit} ({what}) is set; a scaled limit puts iomap_base inside the segment"
        );
    }

    assert_ne!(descriptor.low & bit::PRESENT, 0, "present must be set");
    assert_eq!(
        descriptor.low & (0xF << 40),
        TASK_STATE_TYPE,
        "the type nibble is an available 64-bit TSS"
    );
}

#[test]
fn ring_zero_descriptors_match_the_known_encoding() {
    assert_eq!(Descriptor::KERNEL_CODE.bits(), 0x0020_9B00_0000_0000);
    assert_eq!(Descriptor::KERNEL_DATA.bits(), 0x0000_9300_0000_0000);
    assert_eq!(Descriptor::NULL.bits(), 0);
}

/// VMware faulted the first `iretq` to ring 3 writing an accessed bit in the read-only table.
#[test]
fn every_code_and_data_descriptor_is_already_accessed() {
    for descriptor in [
        Descriptor::KERNEL_CODE,
        Descriptor::KERNEL_DATA,
        Descriptor::USER_CODE,
        Descriptor::USER_DATA,
    ] {
        assert_ne!(descriptor.bits() & ACCESSED, 0, "{descriptor:?}");
    }
    assert_eq!(Descriptor::USER_DATA.bits(), 0x0000_F300_0000_0000);
    assert_eq!(Descriptor::USER_CODE.bits(), 0x0020_FB00_0000_0000);
}

#[test]
fn code_and_data_differ_only_in_execute_and_long_mode() {
    let difference = Descriptor::KERNEL_CODE.bits() ^ Descriptor::KERNEL_DATA.bits();
    assert_eq!(difference, bit::EXECUTABLE | bit::LONG_MODE);
}

#[test]
fn both_ring_zero_descriptors_are_present_at_privilege_zero() {
    for descriptor in [Descriptor::KERNEL_CODE, Descriptor::KERNEL_DATA] {
        assert_ne!(descriptor.bits() & bit::PRESENT, 0);
        assert_eq!(descriptor.bits() & PRIVILEGE, 0);
    }
}

#[test]
fn a_task_state_descriptor_scatters_its_base_across_four_fields() {
    let descriptor = SystemDescriptor::task_state(0xFFFF_8877_6655_4433, 104);

    assert_eq!(descriptor.low & 0xFFFF, 103);
    assert_eq!(descriptor.low >> 16 & 0xFFFF, 0x4433);
    assert_eq!(descriptor.low >> 32 & 0xFF, 0x55);
    assert_eq!(descriptor.low >> 56 & 0xFF, 0x66);
    assert_eq!(descriptor.high, 0xFFFF_8877);
}

#[test]
fn a_task_state_descriptor_is_present_available_and_not_a_segment() {
    let descriptor = SystemDescriptor::task_state(0x1000, 104);

    assert_ne!(descriptor.low & bit::PRESENT, 0);
    assert_eq!(descriptor.low >> 40 & 0xF, 0x9);
    assert_eq!(descriptor.low & bit::CODE_OR_DATA, 0);
    assert_eq!(descriptor.low & PRIVILEGE, 0);
}

#[test]
#[should_panic(expected = "cannot be empty")]
fn a_task_state_descriptor_rejects_a_zero_size() {
    let _ = SystemDescriptor::task_state(0x1000, 0);
}

/// Any other difference would be a different kind of segment, invisible in a boot that worked.
#[test]
fn the_user_descriptors_are_the_kernel_ones_at_ring_three() {
    assert_eq!(
        Descriptor::USER_CODE.bits(),
        Descriptor::KERNEL_CODE.bits() | (0b11 << 45)
    );
    assert_eq!(
        Descriptor::USER_DATA.bits(),
        Descriptor::KERNEL_DATA.bits() | (0b11 << 45)
    );
}

#[test]
fn only_the_user_descriptors_are_ring_three() {
    assert_eq!(Descriptor::KERNEL_CODE.bits() >> 45 & 0b11, 0);
    assert_eq!(Descriptor::KERNEL_DATA.bits() >> 45 & 0b11, 0);
    assert_eq!(Descriptor::USER_CODE.bits() >> 45 & 0b11, 3);
    assert_eq!(Descriptor::USER_DATA.bits() >> 45 & 0b11, 3);
    assert_eq!(Descriptor::NULL.bits() >> 45 & 0b11, 0);
}

/// An executable data descriptor is a code segment the CPU will jump into.
#[test]
fn only_the_code_descriptors_are_executable() {
    assert_ne!(Descriptor::KERNEL_CODE.bits() & bit::EXECUTABLE, 0);
    assert_ne!(Descriptor::USER_CODE.bits() & bit::EXECUTABLE, 0);
    assert_eq!(Descriptor::KERNEL_DATA.bits() & bit::EXECUTABLE, 0);
    assert_eq!(Descriptor::USER_DATA.bits() & bit::EXECUTABLE, 0);
}

/// A data descriptor with `L` set is reserved, and its result undefined.
#[test]
fn long_mode_is_a_code_only_bit_at_every_level() {
    const LONG_MODE: u64 = 1 << 53;
    assert_ne!(Descriptor::USER_CODE.bits() & LONG_MODE, 0);
    assert_eq!(Descriptor::USER_DATA.bits() & LONG_MODE, 0);
}
