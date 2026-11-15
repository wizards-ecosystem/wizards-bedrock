use crate::Selector;

pub const TABLE_LEN: usize = 256;

const TABLE_BYTES: usize = TABLE_LEN * size_of::<Gate>();

pub const TABLE_LIMIT: u16 = (TABLE_BYTES - 1) as u16;

const INTERRUPT_GATE: u8 = 0x8E;

const MAX_INTERRUPT_STACK: u8 = 7;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
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
    pub const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        interrupt_stack: 0,
        attributes: 0,
        offset_middle: 0,
        offset_high: 0,
        reserved: 0,
    };

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

    // An interrupt gate, so IF is clear before kernel stack discipline exists.
    pub const fn callable_from_user(handler: u64, selector: Selector, stack: u8) -> Self {
        let mut gate = Self::interrupt(handler, selector, stack);
        gate.attributes |= 0b11 << 5;
        gate
    }

    pub const fn privilege(self) -> u8 {
        (self.attributes >> 5) & 0b11
    }
}
