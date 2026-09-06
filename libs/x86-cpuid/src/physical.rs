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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// The number of implemented physical-address bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether `start..end` is a nonempty extent below the exclusive limit.
    pub const fn contains(self, start: u64, end: u64) -> bool {
        start < end && end <= 1_u64 << self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decoded_width_reports_its_bits() {
        for bits in [32_u8, 36, 40, 46, 52] {
            let width =
                PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF, u32::from(bits)).unwrap();
            assert_eq!(width.bits(), bits);
        }
    }

    #[test]
    fn every_reported_byte_is_either_supported_or_refused() {
        for bits in u8::MIN..=u8::MAX {
            let decoded = PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF, u32::from(bits));
            assert_eq!(decoded.is_some(), (32..=52).contains(&bits), "{bits}");
        }
    }

    #[test]
    fn an_absent_leaf_never_borrows_an_unrelated_answer() {
        assert_eq!(
            PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF - 1, 52),
            None
        );
    }

    #[test]
    fn each_supported_limit_is_exclusive() {
        for bits in [36, 40, 46, 52] {
            let width = PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF, bits).unwrap();
            let limit = 1_u64 << bits;
            assert!(width.contains(limit - 1, limit));
            assert!(!width.contains(limit - 1, limit + 1));
            assert!(!width.contains(limit, limit));
        }
    }
}
