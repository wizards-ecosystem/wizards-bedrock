//! Selection of an interrupt accepted by the local APIC but not yet delivered.

use crate::vector::InterruptVector;

/// The highest-priority vector in the eight IRR words, audited before the first `sti`.
///
/// Every bank is audited, so no request latched before remapping takeover crosses that `sti`.
pub fn highest(registers: &[u32; 8]) -> Option<u8> {
    for (bank, bits) in registers.iter().enumerate().rev() {
        if *bits != 0 {
            let bit = 31 - bits.leading_zeros() as usize;
            return u8::try_from(bank * 32 + bit).ok();
        }
    }
    None
}

/// Whether the named vector is pending, even beneath a higher-priority request.
pub fn contains(registers: &[u32; 8], vector: InterruptVector) -> bool {
    let raw = usize::from(vector.get());
    registers[raw / 32] & (1 << (raw % 32)) != 0
}

#[cfg(test)]
mod tests;
