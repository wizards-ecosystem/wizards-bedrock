//! Probe for `libs/handle-space` — the three properties its module doc names.
//!
//! The doc calls the handle "the most security-sensitive integer in this
//! kernel" and then states three properties, each with a reason. All three are
//! transcribed here rather than restated from the bodies (D-242).

use handle_space::{GENERATIONS, Mixer, SLOTS, Slot};
use proptest::prelude::*;

/// Any slot the constructor accepts.
fn slot() -> impl Strategy<Value = Slot> {
    (0u32..SLOTS, 0u32..GENERATIONS)
        .prop_filter_map("a representable slot", |(index, generation)| {
            Slot::new(index, generation)
        })
}

/// Mixers from both constructors, since the doc says mixers may repeat.
fn mixer() -> impl Strategy<Value = Mixer> {
    prop_oneof![
        any::<u32>().prop_map(Mixer::new),
        any::<u32>().prop_map(Mixer::for_table),
    ]
}

proptest! {
    /// A handle decodes back to the slot it was made from, in its own table.
    #[test]
    fn a_handle_decodes_to_the_slot_it_was_made_from(slot in slot(), mixer in mixer()) {
        let value = slot.encode(mixer);
        prop_assert_eq!(
            Slot::decode(value, mixer),
            Some(slot),
            "handle {:#x} did not decode to the slot that made it",
            value
        );
    }

    /// "Zero is never a handle, whatever the mixer."
    ///
    /// The doc names this as the closure of "the commonest way to fabricate one
    /// by accident", so it is a claim about every slot and every mixer at once.
    #[test]
    fn zero_is_never_a_handle_whatever_the_mixer(slot in slot(), mixer in mixer()) {
        prop_assert_ne!(slot.encode(mixer), 0, "a slot encoded to zero");
    }

    /// "Reusing a slot changes the value, so a handle kept across a close does
    /// not name whatever took the slot next."
    #[test]
    fn reusing_a_slot_changes_the_handle(slot in slot(), mixer in mixer()) {
        let reused = slot.reused();
        prop_assert_ne!(
            slot.encode(mixer),
            reused.encode(mixer),
            "a reused slot encoded to the same handle as before it was closed"
        );
        prop_assert_eq!(reused.index(), slot.index(), "reuse moved the slot");
    }

    /// A value whose reserved bits are not intact names nothing.
    ///
    /// The doc is precise here and an earlier version of it was wrong (D-95):
    /// small integers with the reserved bits *set* do decode, to slots that are
    /// almost never in the arena. What decode refuses is a value whose reserved
    /// bits were disturbed.
    #[test]
    fn a_value_with_disturbed_reserved_bits_names_nothing(
        slot in slot(),
        mixer in mixer(),
        flip in 0u32..2,
    ) {
        let value = slot.encode(mixer);
        let disturbed = value ^ (1 << flip);
        prop_assert_eq!(
            Slot::decode(disturbed, mixer),
            None,
            "flipping reserved bit {} of {:#x} still named a slot",
            flip,
            value
        );
    }
}
