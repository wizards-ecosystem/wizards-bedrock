//! Global-system-interrupt ranges owned by I/O APICs, joined from ACPI bases and hardware counts.

use core::fmt;

use register_bits::u16_halves;

/// The most redirection entries a plausible version register reports; more means another device.
const MAX_REDIRECTION_ENTRIES: u16 = 240;

/// Version identifiers 82093AA and every later I/O APIC report in bits 7:0.
const MIN_VERSION: u16 = 0x11;
const MAX_VERSION: u16 = 0x20;

/// The redirection-entry count, one past bits 23:16, if bits 7:0 name a real I/O APIC.
///
/// ```
/// use x86_apic::io::entries_from_version;
///
/// assert_eq!(entries_from_version(0x0017_0011), Some(24));
/// assert_eq!(entries_from_version(0x0000_0011), Some(1));
/// assert_eq!(entries_from_version(0x0017_0009), None);
/// assert_eq!(entries_from_version(0xFFFF_FFFF), None);
/// ```
pub const fn entries_from_version(version: u32) -> Option<u32> {
    let (low, high) = u16_halves(version);
    let identity = low & 0xFF;
    if identity < MIN_VERSION || identity > MAX_VERSION {
        return None;
    }
    let entries = (high & 0xFF) + 1;
    if entries > MAX_REDIRECTION_ENTRIES {
        return None;
    }
    Some(entries as u32)
}

/// One non-empty, representable I/O APIC GSI range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GsiRange {
    first: u32,
    last: u32,
}

impl GsiRange {
    /// Join an ACPI GSI base to a hardware entry count, unless empty or past the 32-bit namespace.
    pub const fn new(first: u32, entries: u32) -> Option<Self> {
        let Some(distance) = entries.checked_sub(1) else {
            return None;
        };
        let Some(last) = first.checked_add(distance) else {
            return None;
        };
        Some(Self { first, last })
    }

    pub const fn first(self) -> u32 {
        self.first
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// The unique controller owning `gsi`, after the whole set is proven free of overlaps.
///
/// An overlap anywhere refuses, so routing cannot depend on discovery order.
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
mod tests;
