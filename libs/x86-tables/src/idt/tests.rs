use super::*;

const KERNEL_CODE: Selector = Selector::new(1, 0);

#[test]
fn a_gate_is_sixteen_bytes_and_a_table_is_four_kilobytes() {
    assert_eq!(size_of::<Gate>(), 16);
    assert_eq!(TABLE_BYTES, 4096);
    assert_eq!(TABLE_LIMIT, 4095);
}

#[test]
fn a_handler_address_survives_being_split_in_three() {
    let handler = 0xFFFF_FFFF_8012_3456;
    let gate = Gate::interrupt(handler, KERNEL_CODE, 0);

    assert_eq!(gate.offset_low, 0x3456);
    assert_eq!(gate.offset_middle, 0x8012);
    assert_eq!(gate.offset_high, 0xFFFF_FFFF);
}

#[test]
fn a_gate_records_its_selector_and_stack() {
    for stack in [0, 1, MAX_INTERRUPT_STACK] {
        let gate = Gate::interrupt(0x1000, KERNEL_CODE, stack);
        assert_eq!(gate.selector, KERNEL_CODE.bits());
        assert_eq!(gate.interrupt_stack, stack);
        assert_ne!(gate.attributes & 0x80, 0);
    }
}

#[test]
fn an_interrupt_gate_is_ring_zero_and_clears_the_interrupt_flag() {
    let gate = Gate::interrupt(0x1000, KERNEL_CODE, 0);

    assert_eq!(gate.attributes & 0b0110_0000, 0, "descriptor privilege");
    assert_eq!(gate.attributes & 0xF, 0xE, "gate type");
}

#[test]
fn a_missing_gate_is_not_present() {
    assert_eq!(Gate::MISSING.attributes & 0x80, 0);
    assert_eq!(Gate::MISSING, Gate::default());
}

#[test]
#[should_panic(expected = "seven entries")]
fn a_gate_rejects_a_stack_outside_the_table() {
    let _ = Gate::interrupt(0x1000, KERNEL_CODE, 8);
}

/// Whole attribute bytes from the manual; a low nibble of `0xE` keeps `IF` cleared on entry.
#[test]
fn a_user_callable_gate_differs_only_in_privilege() {
    let selector = Selector::new(1, 0);
    let ordinary = Gate::interrupt(0xffff_8000_0000_1234, selector, 0);
    let callable = Gate::callable_from_user(0xffff_8000_0000_1234, selector, 0);

    assert_eq!(ordinary.attributes, 0b1000_1110);
    assert_eq!(callable.attributes, 0b1110_1110);
    assert_eq!(callable.attributes & 0xF, 0xE, "it became a trap gate");
    assert_eq!(callable.privilege(), 3);
    assert_eq!(ordinary.privilege(), 0, "an ordinary gate admits ring 3");
    assert_ne!(callable.attributes & 0x80, 0);
    assert_ne!(ordinary.attributes & 0x80, 0);
}

/// Swapping `interrupt_stack` and `attributes` keeps the size and every accessor round trip.
#[test]
fn a_gate_is_laid_out_the_way_the_manual_says() {
    use core::mem::{align_of, offset_of, size_of};

    assert_eq!(size_of::<Gate>(), 16);
    assert_eq!(align_of::<Gate>(), 4);
    assert_eq!(offset_of!(Gate, offset_low), 0);
    assert_eq!(offset_of!(Gate, selector), 2);
    assert_eq!(offset_of!(Gate, interrupt_stack), 4);
    assert_eq!(offset_of!(Gate, attributes), 5);
    assert_eq!(offset_of!(Gate, offset_middle), 6);
    assert_eq!(offset_of!(Gate, offset_high), 8);
    assert_eq!(offset_of!(Gate, reserved), 12);
}

/// Read as fields, since an accessor always agrees with the field it names.
#[test]
fn a_built_gate_holds_the_encoding_the_manual_asks_for() {
    let gate = Gate::interrupt(0xffff_8000_1234_5678, Selector::new(1, 0), 1);
    assert_eq!(gate.offset_low, 0x5678);
    assert_eq!(gate.selector, 0x0008);
    assert_eq!(gate.interrupt_stack, 1);
    assert_eq!(gate.attributes, 0x8E);
    assert_eq!(gate.offset_middle, 0x1234);
    assert_eq!(gate.offset_high, 0xffff_8000);
    assert_eq!(gate.reserved, 0);
}

#[test]
fn the_missing_gate_is_not_user_callable() {
    assert_eq!(Gate::MISSING.privilege(), 0);
    assert_eq!(Gate::MISSING.attributes & 0x80, 0);
}
