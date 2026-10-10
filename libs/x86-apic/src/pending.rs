//! Selection of an interrupt accepted by the local APIC but not yet delivered.

use crate::vector::InterruptVector;

/// Return the highest-priority vector present in the eight IRR words.
///
/// The local APIC can retain a fixed interrupt in this state while the CPU's
/// interrupt flag is clear. Auditing every bank before the first `sti` keeps a
/// request accepted before interrupt-remapping takeover from crossing that
/// privilege boundary later.
pub fn highest(registers: &[u32; 8]) -> Option<u8> {
    for (bank, bits) in registers.iter().enumerate().rev() {
        if *bits != 0 {
            let bit = 31 - bits.leading_zeros() as usize;
            return u8::try_from(bank * 32 + bit).ok();
        }
    }
    None
}

/// Whether the named vector is present, even when a higher-priority request is
/// also pending.
pub fn contains(registers: &[u32; 8], vector: InterruptVector) -> bool {
    let raw = usize::from(vector.get());
    registers[raw / 32] & (1 << (raw % 32)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_irr_bit_names_its_exact_pending_vector() {
        for vector in u8::MIN..=u8::MAX {
            let mut registers = [0u32; 8];
            registers[usize::from(vector) / 32] = 1 << (vector % 32);
            assert_eq!(highest(&registers), Some(vector));
        }
        assert_eq!(highest(&[0; 8]), None);
    }

    #[test]
    fn the_highest_pending_priority_wins_across_all_banks() {
        let mut registers = [0u32; 8];
        for vector in [0x10u8, 0x40, 0x41, 0x80, 0xfe] {
            registers[usize::from(vector) / 32] |= 1 << (vector % 32);
        }
        assert_eq!(highest(&registers), Some(0xfe));
    }

    #[test]
    fn a_named_pending_vector_is_visible_beneath_higher_priority_work() {
        let named = InterruptVector::new(0x40).expect("named vector");
        let higher = InterruptVector::new(0x80).expect("higher vector");
        let absent = InterruptVector::new(0x41).expect("absent vector");
        let mut registers = [0u32; 8];
        for vector in [named, higher] {
            let raw = usize::from(vector.get());
            registers[raw / 32] |= 1 << (raw % 32);
        }

        assert_eq!(highest(&registers), Some(higher.get()));
        assert!(contains(&registers, named));
        assert!(contains(&registers, higher));
        assert!(!contains(&registers, absent));
    }
}
