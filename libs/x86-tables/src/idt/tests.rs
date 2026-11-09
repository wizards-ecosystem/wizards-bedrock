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

/// A user-callable gate differs from an ordinary one by its privilege and
/// nothing else.
///
/// The bit that lets ring 3 in is the only one that may change. A gate
/// that also became a trap gate, or lost its stack, would be a different
/// kind of entry wearing the same name — and it would work, which is what
/// makes it worth checking (D-96).
///
/// The equalities against `ordinary` are identities: `callable_from_user`
/// delegates to `interrupt` and then sets two bits of `attributes`, so the
/// handler, selector and stack are literally the same fields, and
/// `((a | 0x60) >> 5) & 3 == 3` for every `a`. Transcribed bytes instead —
/// the whole attribute word, against the manual (D-105).
#[test]
fn a_user_callable_gate_differs_only_in_privilege() {
    let selector = Selector::new(1, 0);
    let ordinary = Gate::interrupt(0xffff_8000_0000_1234, selector, 0);
    let callable = Gate::callable_from_user(0xffff_8000_0000_1234, selector, 0);

    // Present, DPL 0, 64-bit interrupt gate: `1_00_0_1110`.
    assert_eq!(ordinary.attributes, 0b1000_1110);
    // Present, DPL 3, still an interrupt gate: the low nibble `0xE`, not
    // `0xF`, is what keeps `IF` cleared on entry.
    assert_eq!(callable.attributes, 0b1110_1110);
    assert_eq!(callable.attributes & 0xF, 0xE, "it became a trap gate");
    assert_eq!(callable.privilege(), 3);
    assert_eq!(ordinary.privilege(), 0, "an ordinary gate admits ring 3");
    assert_ne!(callable.attributes & 0x80, 0);
    assert_ne!(ordinary.attributes & 0x80, 0);
}

/// The bytes the CPU reads are where the manual says they are.
///
/// `tss::TaskState` got these pins from D-95 and `Gate` did not, though it
/// is equally a structure hardware parses byte for byte. Swapping
/// `interrupt_stack` and `attributes` keeps the size at sixteen and leaves
/// every accessor round-tripping through its own named field — the shipped
/// table then reads IST where DPL should be, with every test green
/// (D-105).
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

/// And every field of a built gate holds the value the manual asks for.
///
/// Read as fields rather than through the accessors, because an accessor
/// and the field it names always agree — that is what left the split of
/// the handler address across three fields unchecked. Together with the
/// offsets above this is the sixteen bytes, which `libs/` cannot
/// `transmute` to (D-105).
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

/// The missing gate admits nobody.
#[test]
fn the_missing_gate_is_not_user_callable() {
    assert_eq!(Gate::MISSING.privilege(), 0);
    assert_eq!(Gate::MISSING.attributes & 0x80, 0);
}
