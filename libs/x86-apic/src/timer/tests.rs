use super::*;
use crate::vector::InterruptVector;

fn vector(raw: u8) -> InterruptVector {
    InterruptVector::new(raw).expect("test vector is assignable")
}

#[test]
fn one_shot_error_is_a_standard_error() {
    fn assert_error<T: core::error::Error>() {}

    assert_error::<OneShotError>();
    assert_eq!(
        OneShotError::Running.to_string(),
        "the local APIC timer is still running"
    );
}

const ALL: &[Divide] = &[
    Divide::By1,
    Divide::By2,
    Divide::By4,
    Divide::By8,
    Divide::By16,
    Divide::By32,
    Divide::By64,
    Divide::By128,
];

/// A ninth variant breaks the match, and the length catches one added there but missing from `ALL`.
#[test]
fn all_is_every_divide_the_enum_has() {
    for divide in ALL {
        let itself = match divide {
            Divide::By1 => Divide::By1,
            Divide::By2 => Divide::By2,
            Divide::By4 => Divide::By4,
            Divide::By8 => Divide::By8,
            Divide::By16 => Divide::By16,
            Divide::By32 => Divide::By32,
            Divide::By64 => Divide::By64,
            Divide::By128 => Divide::By128,
        };
        assert_eq!(*divide, itself);
    }
    assert_eq!(ALL.len(), 8, "ALL has fallen behind the enum");
}

/// SDM Vol. 3A §11.5.4's divide-value table, transcribed: bits 1:0 and 3, all set for one.
#[test]
fn every_divide_encoding_matches_the_manual() {
    for (divide, bits, factor) in [
        (Divide::By2, 0b0000, 2),
        (Divide::By4, 0b0001, 4),
        (Divide::By8, 0b0010, 8),
        (Divide::By16, 0b0011, 16),
        (Divide::By32, 0b1000, 32),
        (Divide::By64, 0b1001, 64),
        (Divide::By128, 0b1010, 128),
        (Divide::By1, 0b1011, 1),
    ] {
        assert_eq!(divide.bits(), bits, "{divide:?} bits");
        assert_eq!(divide.factor(), factor, "{divide:?} factor");
    }
}

/// Bit 2 is reserved, and the architecture faults on reserved bits in this register.
#[test]
fn the_reserved_bit_stays_clear() {
    for divide in ALL {
        assert_eq!(divide.bits() & (1 << 2), 0, "{divide:?}");
    }
}

#[test]
fn no_two_divides_share_an_encoding() {
    for (index, divide) in ALL.iter().enumerate() {
        for other in &ALL[index + 1..] {
            assert_ne!(divide.bits(), other.bits(), "{divide:?} and {other:?}");
        }
    }
}

#[test]
fn a_wait_inside_the_counter_is_the_count() {
    assert_eq!(initial_count(1), 1);
    assert_eq!(initial_count(3_000_000), 3_000_000);
    assert_eq!(initial_count(u64::from(u32::MAX)), u32::MAX);
}

/// A deadline already passed must still interrupt, since a zero count stops the timer for good.
#[test]
fn a_wait_of_no_time_still_arms_the_timer() {
    assert_eq!(initial_count(0), 1);
}

/// A sleep past the counter's range is armed as far as it goes, and the caller re-arms.
#[test]
fn a_wait_past_the_counter_is_the_counter() {
    assert_eq!(initial_count(u64::from(u32::MAX) + 1), u32::MAX);
    assert_eq!(initial_count(u64::MAX), u32::MAX);
}

#[test]
fn one_shot_publication_requires_a_stopped_distinct_source() {
    let vector = DeliverableVector::configured(vector(0x41), vector(0xff)).unwrap();
    assert_eq!(one_shot_entry(1, vector), Err(OneShotError::Running));
    assert!(DeliverableVector::configured(self::vector(0xff), self::vector(0xff)).is_none());
    assert_eq!(
        one_shot_entry(0, vector),
        Ok(Entry::one_shot(self::vector(0x41)))
    );
}

#[test]
fn refused_one_shot_plans_perform_no_fake_register_write() {
    for (count, vector) in [(1, 0x41), (u32::MAX, 0x41), (0, 0xff), (0, 17)] {
        let mut writes = 0;
        if InterruptVector::new(vector)
            .and_then(|vector| DeliverableVector::configured(vector, self::vector(0xff)))
            .is_some_and(|vector| one_shot_entry(count, vector).is_ok())
        {
            writes += 1;
        }
        assert_eq!(writes, 0, "count {count}, vector {vector}");
    }
}

#[test]
fn configured_timer_delivery_exhausts_the_vector_domain() {
    const SPURIOUS: u8 = 0xff;
    for vector in u8::MIN..=u8::MAX {
        let writes = InterruptVector::new(vector)
            .and_then(|vector| DeliverableVector::configured(vector, self::vector(SPURIOUS)))
            .is_some_and(|vector| one_shot_entry(0, vector).is_ok());
        let expected = vector >= 0x30 && vector != SPURIOUS;
        assert_eq!(writes, expected, "vector {vector}");
    }
}
