//! The handle encoding's properties, over every slot and mixer.

use handle_space::{GENERATIONS, Mixer, SLOTS, Slot};
use proptest::prelude::*;

/// Any slot the constructor accepts.
fn slot() -> impl Strategy<Value = Slot> {
    (0u32..SLOTS, 0u32..GENERATIONS)
        .prop_filter_map("a representable slot", |(index, generation)| {
            Slot::new(index, generation)
        })
}

/// Mixers from both constructors, since mixers may repeat.
fn mixer() -> impl Strategy<Value = Mixer> {
    prop_oneof![
        any::<u32>().prop_map(Mixer::new),
        any::<u32>().prop_map(Mixer::for_table),
    ]
}

proptest! {
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

    /// Zero is the commonest handle to fabricate by accident.
    #[test]
    fn zero_is_never_a_handle_whatever_the_mixer(slot in slot(), mixer in mixer()) {
        prop_assert_ne!(slot.encode(mixer), 0, "a slot encoded to zero");
    }

    /// A handle kept across a close does not name whatever took the slot next.
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

    /// Small integers with the reserved bits set do decode; disturbed reserved bits never do.
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
