use crate::vector::InterruptVector;

pub fn highest(registers: &[u32; 8]) -> Option<u8> {
    for (bank, bits) in registers.iter().enumerate().rev() {
        if *bits != 0 {
            let bit = 31 - bits.leading_zeros() as usize;
            return u8::try_from(bank * 32 + bit).ok();
        }
    }
    None
}

pub fn contains(registers: &[u32; 8], vector: InterruptVector) -> bool {
    let raw = usize::from(vector.get());
    registers[raw / 32] & (1 << (raw % 32)) != 0
}
