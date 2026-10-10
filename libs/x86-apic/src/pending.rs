//! Selection of an interrupt accepted by the local APIC but not yet delivered.

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
}
