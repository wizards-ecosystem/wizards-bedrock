//! Global-system-interrupt ranges owned by I/O APICs.
//!
//! ACPI supplies each range's base and the controller supplies its entry
//! count. This module joins those two untrusted numbers without hardware and
//! identifies a line's unique owner only after proving no ranges overlap.

use core::fmt;

/// One non-empty, representable I/O APIC GSI range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GsiRange {
    first: u32,
    last: u32,
}

impl GsiRange {
    /// Join an ACPI GSI base to a hardware-reported entry count.
    ///
    /// Returns `None` for an empty range or when its final GSI would exceed
    /// the 32-bit namespace ACPI defines.
    pub const fn new(first: u32, entries: u32) -> Option<Self> {
        let Some(distance) = entries.checked_sub(1) else {
            return None;
        };
        let Some(last) = first.checked_add(distance) else {
            return None;
        };
        Some(Self { first, last })
    }

    /// The first GSI this controller owns.
    pub const fn first(self) -> u32 {
        self.first
    }

    /// The last GSI this controller owns, inclusive.
    pub const fn last(self) -> u32 {
        self.last
    }

    /// Whether this controller owns `gsi`.
    pub const fn contains(self, gsi: u32) -> bool {
        self.first <= gsi && gsi <= self.last
    }

    /// Whether this range and `other` claim any common GSI.
    pub const fn overlaps(self, other: Self) -> bool {
        self.first <= other.last && other.first <= self.last
    }
}

/// Two controllers claim at least one common GSI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlappingRanges {
    /// Index of the earlier range in the supplied inventory.
    pub first: usize,
    /// Index of the later range in the supplied inventory.
    pub second: usize,
}

impl fmt::Display for OverlappingRanges {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "i/o apic ranges {} and {} overlap",
            self.first, self.second
        )
    }
}

impl core::error::Error for OverlappingRanges {}

/// Find the unique controller which owns `gsi`.
///
/// The complete set is checked before an owner is returned. An overlap is
/// malformed firmware even when it lies away from the particular GSI being
/// selected, because accepting only the convenient subset would make routing
/// depend on discovery order.
///
/// # Errors
///
/// [`OverlappingRanges`] naming the first ambiguous pair.
pub fn owner_of(ranges: &[GsiRange], gsi: u32) -> Result<Option<usize>, OverlappingRanges> {
    for first in 0..ranges.len() {
        for second in first + 1..ranges.len() {
            if ranges[first].overlaps(ranges[second]) {
                return Err(OverlappingRanges { first, second });
            }
        }
    }
    Ok(ranges.iter().position(|range| range.contains(gsi)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_is_non_empty_and_stays_inside_the_gsi_namespace() {
        assert_eq!(GsiRange::new(7, 0), None);
        assert_eq!(GsiRange::new(u32::MAX, 2), None);

        let one = GsiRange::new(u32::MAX, 1).expect("the final GSI alone");
        assert_eq!(one.first(), u32::MAX);
        assert_eq!(one.last(), u32::MAX);
        assert!(one.contains(u32::MAX));
    }

    #[test]
    fn a_gsi_selects_its_later_controller_across_a_gap() {
        let ranges = [
            GsiRange::new(0, 8).expect("0 through 7"),
            GsiRange::new(24, 16).expect("24 through 39"),
        ];

        assert_eq!(owner_of(&ranges, 1), Ok(Some(0)));
        assert_eq!(owner_of(&ranges, 23), Ok(None));
        assert_eq!(owner_of(&ranges, 24), Ok(Some(1)));
        assert_eq!(owner_of(&ranges, 39), Ok(Some(1)));
        assert_eq!(owner_of(&ranges, 40), Ok(None));
    }

    #[test]
    fn any_overlap_refuses_the_complete_inventory() {
        let ranges = [
            GsiRange::new(0, 24).expect("0 through 23"),
            GsiRange::new(40, 8).expect("40 through 47"),
            GsiRange::new(20, 8).expect("20 through 27"),
        ];

        assert_eq!(
            owner_of(&ranges, 44),
            Err(OverlappingRanges {
                first: 0,
                second: 2,
            })
        );
    }
}
