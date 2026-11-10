//! Runtime physical-address width decoding.

/// The extended leaf which reports `MAXPHYADDR` in `EAX[7:0]`.
pub const ADDRESS_SIZE_LEAF: u32 = 0x8000_0008;

/// A physical-address width the four-level page-table encoding can represent.
///
/// # Examples
///
/// ```
/// use x86_cpuid::physical::{ADDRESS_SIZE_LEAF, PhysicalAddressWidth};
///
/// let width = PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF, 46).unwrap();
/// assert!(width.contains(0, 1_u64 << 46));
/// assert!(!width.contains(0, (1_u64 << 46) + 1));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PhysicalAddressWidth(u8);

impl PhysicalAddressWidth {
    /// Decode a supported `MAXPHYADDR` answer.
    pub const fn from_leaf(maximum_extended_leaf: u32, eax: u32) -> Option<Self> {
        let bits = (eax & u8::MAX as u32) as u8;
        if maximum_extended_leaf < ADDRESS_SIZE_LEAF || bits < 32 || bits > 52 {
            return None;
        }
        Some(Self(bits))
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether `start..end` is a nonempty extent below the exclusive limit.
    pub const fn contains(self, start: u64, end: u64) -> bool {
        start < end && end <= 1_u64 << self.0
    }
}

#[cfg(test)]
mod tests;
