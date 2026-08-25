//! Interrupt descriptor table gates.
//!
//! A 64-bit gate is sixteen bytes that name a handler, the code selector to
//! enter it with, and optionally one of the seven interrupt stacks in the
//! task-state segment. The handler address is split across three fields for
//! reasons that are entirely historical.

use crate::Selector;

/// Gates in an interrupt descriptor table. The architecture fixes this.
pub const TABLE_LEN: usize = 256;

/// Size of a full table in bytes.
pub const TABLE_BYTES: usize = TABLE_LEN * size_of::<Gate>();

/// Value the table's limit field takes: one less than its size.
pub const TABLE_LIMIT: u16 = (TABLE_BYTES - 1) as u16;

/// Present, ring 0, 64-bit interrupt gate.
///
/// An interrupt gate clears the interrupt flag on entry; the trap gate that
/// does not is deliberately unavailable, because a handler that wants to run
/// with interrupts on should say so itself.
const INTERRUPT_GATE: u8 = 0x8E;

/// Highest interrupt stack table index a gate can select.
const MAX_INTERRUPT_STACK: u8 = 7;

/// One interrupt descriptor table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct Gate {
    offset_low: u16,
    selector: u16,
    interrupt_stack: u8,
    attributes: u8,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

impl Gate {
    /// A gate with the present bit clear: taking this vector raises a general
    /// protection fault rather than jumping somewhere arbitrary.
    pub const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        interrupt_stack: 0,
        attributes: 0,
        offset_middle: 0,
        offset_high: 0,
        reserved: 0,
    };

    /// An interrupt gate entering `handler` through `selector`.
    ///
    /// `stack` selects an entry in the task-state segment's interrupt stack
    /// table, counted from 1; zero means the handler runs on whichever stack
    /// was in use when the interrupt arrived.
    ///
    /// # Panics
    ///
    /// Panics if `stack` exceeds 7.
    pub const fn interrupt(handler: u64, selector: Selector, stack: u8) -> Self {
        assert!(
            stack <= MAX_INTERRUPT_STACK,
            "the interrupt stack table has seven entries"
        );

        Self {
            offset_low: handler as u16,
            selector: selector.bits(),
            interrupt_stack: stack,
            attributes: INTERRUPT_GATE,
            offset_middle: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }

    /// An interrupt gate ring 3 may invoke with `int`.
    ///
    /// The same gate in every respect but its descriptor privilege level.
    /// A gate at DPL 0 raises a general-protection fault when user code names
    /// it, which is what every vector in this table should do — a device's
    /// line is not a service. Exactly one gate is opened this way, and it is
    /// the kernel's entry (D-96).
    ///
    /// Still an *interrupt* gate rather than a trap gate: the interrupt flag
    /// is cleared on entry either way here, because a handler that could be
    /// preempted before it has established a kernel stack discipline is the
    /// hazard this kernel spent Phase 2 removing.
    ///
    /// # Panics
    ///
    /// As [`interrupt`](Self::interrupt).
    pub const fn callable_from_user(handler: u64, selector: Selector, stack: u8) -> Self {
        let mut gate = Self::interrupt(handler, selector, stack);
        gate.attributes |= 0b11 << 5;
        gate
    }

    /// The privilege level that may reach this gate with `int`.
    pub const fn privilege(self) -> u8 {
        (self.attributes >> 5) & 0b11
    }

    /// The handler address, reassembled from its three fields.
    pub const fn handler_address(self) -> u64 {
        self.offset_low as u64 | (self.offset_middle as u64) << 16 | (self.offset_high as u64) << 32
    }

    /// The code selector the handler is entered through.
    pub const fn selector(self) -> Selector {
        Selector::from_bits(self.selector)
    }

    /// Which interrupt stack the handler runs on, or zero for the interrupted
    /// one.
    pub const fn interrupt_stack(self) -> u8 {
        self.interrupt_stack
    }

    /// Whether the CPU will accept this vector.
    pub const fn is_present(self) -> bool {
        self.attributes & 0x80 != 0
    }
}

#[cfg(test)]
mod tests {
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

        assert_eq!(gate.handler_address(), handler);
        assert_eq!(gate.offset_low, 0x3456);
        assert_eq!(gate.offset_middle, 0x8012);
        assert_eq!(gate.offset_high, 0xFFFF_FFFF);
    }

    #[test]
    fn a_gate_records_its_selector_and_stack() {
        let gate = Gate::interrupt(0x1000, KERNEL_CODE, 1);

        assert_eq!(gate.selector(), KERNEL_CODE);
        assert_eq!(gate.interrupt_stack(), 1);
        assert!(gate.is_present());
    }

    #[test]
    fn an_interrupt_gate_is_ring_zero_and_clears_the_interrupt_flag() {
        let gate = Gate::interrupt(0x1000, KERNEL_CODE, 0);

        assert_eq!(gate.attributes & 0b0110_0000, 0, "descriptor privilege");
        assert_eq!(gate.attributes & 0xF, 0xE, "gate type");
    }

    #[test]
    fn a_missing_gate_is_not_present() {
        assert!(!Gate::MISSING.is_present());
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
        // Present, DPL 3, still a 64-bit *interrupt* gate — the low nibble is
        // `0xE` and not `0xF`, which is what keeps it from being a trap gate
        // and keeps `IF` cleared on entry.
        assert_eq!(callable.attributes, 0b1110_1110);
        assert_eq!(callable.attributes & 0xF, 0xE, "it became a trap gate");
        assert_eq!(callable.privilege(), 3);
        assert_eq!(ordinary.privilege(), 0, "an ordinary gate admits ring 3");
        assert_eq!(callable.handler_address(), 0xffff_8000_0000_1234);
        assert!(callable.is_present() && ordinary.is_present());
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
        assert!(!Gate::MISSING.is_present());
    }
}
