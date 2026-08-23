//! Local vector table entries.
//!
//! One 32-bit register for each interrupt source the APIC owns itself — its
//! timer, its two local pins, its error line — all sharing a layout. An entry
//! says which vector the source delivers on, or that it delivers nothing.

use crate::FIRST_VECTOR;

/// Bits 8–10: delivery mode. Zero is a plain vector delivered to this
/// processor, which is the only mode this kernel programs. The others exist so
/// firmware can wire a pin to the non-maskable interrupt or to the legacy
/// controller, and undoing that is exactly what masking is for.
const DELIVERY_FIXED: u32 = 0;

/// Bit 16: the source is disconnected while this is set.
const MASK: u32 = 1 << 16;

/// Bits 17–18: the timer's mode, meaningful only in the timer's entry. The
/// third value, TSC-deadline, is not implemented here: it arms the timer from
/// a comparison against the timestamp counter rather than from a countdown,
/// which is a different interface, not another mode of this one.
const TIMER_MODE: u32 = 0b11 << 17;

/// The mode that counts down once and stops. Zero, which is also what the
/// register resets to.
const TIMER_ONE_SHOT: u32 = 0b00 << 17;

/// The mode that reloads the initial count at zero and keeps counting.
const TIMER_PERIODIC: u32 = 0b01 << 17;

/// Bits 0–7: the vector.
const VECTOR: u32 = 0xFF;

/// One local vector table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry(u32);

impl Entry {
    /// An entry that delivers nothing.
    ///
    /// This is also the value the architecture resets every entry to, so
    /// writing it puts a pin back the way the processor started rather than
    /// merely quiet.
    pub const MASKED: Self = Self(MASK);

    /// Deliver `vector` to this processor whenever the source fires.
    ///
    /// # Panics
    ///
    /// Panics if `vector` is below [`FIRST_VECTOR`]. The APIC would refuse it
    /// too, by recording an illegal-vector error and delivering nothing, which
    /// is a great deal harder to notice than a panic during boot.
    pub const fn fixed(vector: u8) -> Self {
        assert!(
            vector >= FIRST_VECTOR,
            "the local APIC will not deliver a vector below 16"
        );
        Self(vector as u32 | DELIVERY_FIXED)
    }

    /// Deliver `vector` once when the timer's countdown reaches zero, and
    /// then stop.
    ///
    /// The mode a tickless kernel programs: each expiry is an event the
    /// scheduler asked for, and the timer does nothing again until it is
    /// armed for the next one.
    ///
    /// # Panics
    ///
    /// Panics if `vector` is below [`FIRST_VECTOR`], for the same reason
    /// [`fixed`](Self::fixed) does — whose guard this rides.
    pub const fn one_shot(vector: u8) -> Self {
        Self(Self::fixed(vector).bits() | TIMER_ONE_SHOT)
    }

    /// Deliver `vector` every time the timer's countdown reaches zero.
    ///
    /// The reload is the hardware's: at zero the timer restarts from its
    /// initial count with no software involved, which is what makes the tick
    /// periodic rather than re-armed.
    ///
    /// # Panics
    ///
    /// Panics if `vector` is below [`FIRST_VECTOR`], for the same reason
    /// [`fixed`](Self::fixed) does — whose guard this rides.
    pub const fn periodic(vector: u8) -> Self {
        Self(Self::fixed(vector).bits() | TIMER_PERIODIC)
    }

    /// Interpret what the register returned.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The value to write.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether the source is disconnected.
    pub const fn is_masked(self) -> bool {
        self.0 & MASK != 0
    }

    /// The vector this entry delivers on.
    pub const fn vector(self) -> u8 {
        (self.0 & VECTOR) as u8
    }

    /// Whether the timer reloads and continues at zero rather than stopping.
    pub const fn is_periodic(self) -> bool {
        self.0 & TIMER_MODE == TIMER_PERIODIC
    }

    /// Whether the timer counts down once and stops.
    pub const fn is_one_shot(self) -> bool {
        self.0 & TIMER_MODE == TIMER_ONE_SHOT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SDM Vol. 3A §11.5.1: every local vector table entry resets to 0x00010000.
    #[test]
    fn a_masked_entry_is_the_value_the_processor_resets_to() {
        assert_eq!(Entry::MASKED.bits(), 0x0001_0000);
        assert!(Entry::MASKED.is_masked());
    }

    #[test]
    fn a_fixed_entry_delivers_its_vector_and_is_not_masked() {
        let entry = Entry::fixed(0xFE);

        assert_eq!(entry.vector(), 0xFE);
        assert!(!entry.is_masked());
        assert_eq!(entry.bits(), 0xFE);
    }

    /// Firmware conventionally leaves LINT0 delivering the legacy controller's
    /// output as an external interrupt — delivery mode 7, unmasked. Masking it
    /// has to clear the mode as well as set the mask bit, or the pin stays
    /// wired to a controller the kernel has stopped listening to.
    #[test]
    fn masking_a_pin_firmware_wired_to_the_legacy_controller_clears_its_mode() {
        let firmware = Entry::from_bits(0x0000_0700);

        assert!(!firmware.is_masked());
        assert_eq!(Entry::MASKED.bits() & 0x0000_0700, 0);
    }

    #[test]
    fn an_entry_read_back_round_trips() {
        let written = Entry::fixed(0x40);

        assert_eq!(Entry::from_bits(written.bits()), written);
    }

    /// SDM Vol. 3A figure 11-8: the periodic mode is bit 17 alone, beside the
    /// vector in bits 0–7 and nothing else set.
    #[test]
    fn a_periodic_entry_is_its_vector_plus_the_mode_bit() {
        let entry = Entry::periodic(0x41);

        assert_eq!(entry.bits(), 0x0002_0041);
        assert!(entry.is_periodic());
        assert!(!entry.is_masked());
        assert_eq!(entry.vector(), 0x41);
    }

    /// The same figure: one-shot is the mode bits left at zero, so a one-shot
    /// entry is indistinguishable from a plain fixed one — which is the
    /// property that lets every other local vector share this type.
    #[test]
    fn a_one_shot_entry_is_its_vector_and_nothing_else() {
        let entry = Entry::one_shot(0x41);

        assert_eq!(entry.bits(), 0x0000_0041);
        assert_eq!(entry, Entry::fixed(0x41));
        assert!(entry.is_one_shot());
        assert!(!entry.is_periodic());
        assert!(!entry.is_masked());
    }

    /// The two modes this crate encodes are exclusive, and a masked entry is
    /// one-shot by the same rule the processor resets it to.
    #[test]
    fn no_entry_is_both_modes_at_once() {
        for entry in [Entry::one_shot(0x41), Entry::periodic(0x41), Entry::MASKED] {
            assert!(entry.is_one_shot() != entry.is_periodic(), "{entry:?}");
        }
    }

    #[test]
    fn a_fixed_entry_is_one_shot() {
        assert!(!Entry::fixed(0x40).is_periodic());
        assert!(Entry::fixed(0x40).is_one_shot());
    }

    /// Re-masking the timer must also put its mode back to the reset value, or
    /// the next unmask inherits a stale one.
    #[test]
    fn a_masked_entry_carries_no_timer_mode() {
        assert_eq!(Entry::MASKED.bits() & TIMER_MODE, 0);
        assert!(!Entry::MASKED.is_periodic());
    }

    #[test]
    fn a_periodic_entry_round_trips() {
        let written = Entry::periodic(0x41);

        assert_eq!(Entry::from_bits(written.bits()), written);
    }

    #[test]
    #[should_panic(expected = "below 16")]
    fn an_entry_refuses_a_vector_the_apic_would_reject() {
        let _ = Entry::fixed(FIRST_VECTOR - 1);
    }

    #[test]
    #[should_panic(expected = "below 16")]
    fn a_periodic_entry_refuses_a_vector_the_apic_would_reject() {
        let _ = Entry::periodic(FIRST_VECTOR - 1);
    }

    #[test]
    #[should_panic(expected = "below 16")]
    fn a_one_shot_entry_refuses_a_vector_the_apic_would_reject() {
        let _ = Entry::one_shot(FIRST_VECTOR - 1);
    }
}
