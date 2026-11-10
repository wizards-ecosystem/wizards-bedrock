use super::*;

fn vector(raw: u8) -> InterruptVector {
    InterruptVector::new(raw).expect("test vector is assignable")
}

/// SDM Vol. 3A §11.5.1: every local vector table entry resets to 0x00010000.
#[test]
fn a_masked_entry_is_the_value_the_processor_resets_to() {
    assert_eq!(Entry::MASKED.bits(), 0x0001_0000);
    assert!(Entry::MASKED.is_masked());
}

#[test]
fn a_fixed_entry_delivers_its_vector_and_is_not_masked() {
    let entry = Entry::fixed(vector(0xFE));

    assert_eq!(entry.vector(), 0xFE);
    assert!(!entry.is_masked());
    assert_eq!(entry.bits(), 0xFE);
}

/// Masking LINT0 must also clear the external-interrupt delivery mode firmware leaves there.
#[test]
fn masking_a_pin_firmware_wired_to_the_legacy_controller_clears_its_mode() {
    let firmware = Entry::from_bits(0x0000_0700);

    assert!(!firmware.is_masked());
    assert_eq!(Entry::MASKED.bits() & 0x0000_0700, 0);
}

#[test]
fn an_entry_read_back_round_trips() {
    let written = Entry::fixed(vector(0x40));

    assert_eq!(Entry::from_bits(written.bits()), written);
}

/// SDM Vol. 3A figure 11-8: periodic is bit 17 alone, beside the vector in bits 7:0.
#[test]
fn a_periodic_entry_is_its_vector_plus_the_mode_bit() {
    let entry = Entry::periodic(vector(0x41));

    assert_eq!(entry.bits(), 0x0002_0041);
    assert!(entry.is_periodic());
    assert!(!entry.is_masked());
    assert_eq!(entry.vector(), 0x41);
}

/// One-shot leaves the mode bits zero, which lets every other local vector share this type.
#[test]
fn a_one_shot_entry_is_its_vector_and_nothing_else() {
    let entry = Entry::one_shot(vector(0x41));

    assert_eq!(entry.bits(), 0x0000_0041);
    assert_eq!(entry, Entry::fixed(vector(0x41)));
    assert!(entry.is_one_shot());
    assert!(!entry.is_periodic());
    assert!(!entry.is_masked());
}

/// The two modes are exclusive, and a masked entry is one-shot like the reset value.
#[test]
fn no_entry_is_both_modes_at_once() {
    for entry in [
        Entry::one_shot(vector(0x41)),
        Entry::periodic(vector(0x41)),
        Entry::MASKED,
    ] {
        assert!(entry.is_one_shot() != entry.is_periodic(), "{entry:?}");
    }
}

#[test]
fn a_fixed_entry_is_one_shot() {
    assert!(!Entry::fixed(vector(0x40)).is_periodic());
    assert!(Entry::fixed(vector(0x40)).is_one_shot());
}

#[test]
fn configured_delivery_exhausts_both_vector_domains() {
    for vector in u8::MIN..=u8::MAX {
        for spurious in u8::MIN..=u8::MAX {
            let admitted = InterruptVector::new(vector).and_then(|vector| {
                InterruptVector::new(spurious)
                    .and_then(|spurious| DeliverableVector::configured(vector, spurious))
            });
            let expected = vector >= 0x30 && spurious >= 0x30 && vector != spurious;
            assert_eq!(
                admitted.map(|vector| vector.get().get()),
                expected.then_some(vector),
                "vector {vector}, spurious {spurious}"
            );
        }
    }
}

/// Re-masking the timer restores the reset mode, or the next unmask inherits a stale one.
#[test]
fn a_masked_entry_carries_no_timer_mode() {
    assert_eq!(Entry::MASKED.bits() & TIMER_MODE, 0);
    assert!(!Entry::MASKED.is_periodic());
}

#[test]
fn a_periodic_entry_round_trips() {
    let written = Entry::periodic(vector(0x41));

    assert_eq!(Entry::from_bits(written.bits()), written);
}

/// SDM Vol. 3A table 11-2: a narrower field reads a TSC-deadline entry as one-shot.
#[test]
fn the_timer_mode_field_is_bits_eighteen_and_seventeen() {
    assert_eq!(TIMER_MODE, 0x0006_0000);

    let deadline = Entry::from_bits(0x0004_0041);
    assert!(!deadline.is_one_shot(), "a deadline entry read as one-shot");
    assert!(!deadline.is_periodic());

    let one_shot = Entry::from_bits(0x0000_0041);
    assert!(one_shot.is_one_shot());

    let periodic = Entry::from_bits(0x0002_0041);
    assert!(periodic.is_periodic());
    assert!(!periodic.is_one_shot());
}
