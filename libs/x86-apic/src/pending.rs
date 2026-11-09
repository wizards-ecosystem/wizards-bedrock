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
mod tests;
