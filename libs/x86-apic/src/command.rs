//! The interrupt command register.
//!
//! Writing it sends an interrupt to a processor. The only destination this
//! kernel names is itself — the mechanism it uses to prove its own interrupt
//! path end to end — and a command addressed by shorthand carries no
//! destination, so the upper half of the register stays zero.

use crate::local_vector::DeliverableVector;

/// Bits 8–10: delivery mode. Zero delivers the vector as written, which is
/// everything short of the startup and reset commands that bring up another
/// processor.
const DELIVERY_FIXED: u32 = 0;

/// Bit 14: asserted rather than de-asserted. The de-assert form exists only for
/// the level-triggered INIT of very old multiprocessor firmware, so every
/// command this kernel builds sets it.
const ASSERT: u32 = 1 << 14;

/// Bits 18–19: destination shorthand. One means the processor that sent it.
const SHORTHAND_SELF: u32 = 1 << 18;

/// The contents of [`crate::register::COMMAND`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command(u32);

impl Command {
    /// Build a self-directed command from a deliverable source.
    pub const fn self_interrupt(vector: DeliverableVector) -> Self {
        Self(vector.get().get() as u32 | DELIVERY_FIXED | ASSERT | SHORTHAND_SELF)
    }

    /// The value to write.
    ///
    /// The register is 64 bits wide and its upper half is the destination,
    /// which a shorthand makes irrelevant — so a caller widens this and writes
    /// zeros above it.
    pub const fn bits(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::InterruptVector;

    fn vector(raw: u8) -> InterruptVector {
        InterruptVector::new(raw).expect("test vector is assignable")
    }

    #[test]
    fn a_self_interrupt_names_its_vector_and_no_destination() {
        let command = Command::self_interrupt(
            DeliverableVector::configured(vector(0x40), vector(0xff)).unwrap(),
        );

        assert_eq!(command.bits() & 0xFF, 0x40);
        assert_eq!(command.bits(), 0x0004_4040);
        assert_eq!(command.bits() & 0x0000_0700, 0, "delivery mode is fixed");
        assert_ne!(command.bits() & ASSERT, 0, "level is asserted");
        assert_eq!(
            command.bits() & 0x000C_0000,
            SHORTHAND_SELF,
            "shorthand is self"
        );
    }

    #[test]
    fn configured_self_delivery_exhausts_the_vector_domain() {
        const SPURIOUS: u8 = 0xff;
        for vector in u8::MIN..=u8::MAX {
            let command = InterruptVector::new(vector).and_then(|vector| {
                DeliverableVector::configured(vector, self::vector(SPURIOUS))
                    .map(Command::self_interrupt)
            });
            let expected = vector >= 0x30 && vector != SPURIOUS;
            assert_eq!(command.is_some(), expected, "vector {vector}");
        }
        assert!(InterruptVector::new(17).is_none());
    }

    #[test]
    fn a_spurious_self_delivery_performs_no_fake_register_write() {
        let mut writes = 0;
        if DeliverableVector::configured(vector(0xff), vector(0xff))
            .map(Command::self_interrupt)
            .is_some()
        {
            writes += 1;
        }
        assert_eq!(writes, 0);
    }

    /// Bit 12 is the delivery-status bit of the memory-mapped encoding, which
    /// x2APIC removed — a write there is synchronous. Building a command that
    /// sets it would be writing a reserved bit, which faults.
    #[test]
    fn a_command_leaves_the_removed_delivery_status_bit_clear() {
        let command = Command::self_interrupt(
            DeliverableVector::configured(vector(0x40), vector(0xff)).unwrap(),
        );
        assert_eq!(command.bits() & 1 << 12, 0);
    }
}
