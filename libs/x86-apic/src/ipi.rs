const DELIVERY_NMI: u64 = 0b100 << 8;

// Every delivery but an INIT de-assert carries the assert level.
const LEVEL_ASSERT: u64 = 1 << 14;

// The x2APIC command register takes the full 32-bit destination in its high half.
pub const fn non_maskable(destination: u32) -> u64 {
    (destination as u64) << 32 | LEVEL_ASSERT | DELIVERY_NMI
}
