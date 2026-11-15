//! A physical address below the limit a page-table entry can name.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

use x86_paging::{Entry, MAX_PHYSICAL};

/// A physical address below [`MAX_PHYSICAL`].
///
/// Formats exactly as the `u64` it holds.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalAddress(u64);

impl PhysicalAddress {
    pub const ZERO: Self = Self(0);

    /// `address`, unless a page-table entry could not name it.
    pub const fn new(address: u64) -> Option<Self> {
        if address < MAX_PHYSICAL {
            Some(Self(address))
        } else {
            None
        }
    }

    /// The page an entry points to, which its address bits always keep below the limit.
    pub const fn from_entry(entry: Entry) -> Self {
        Self(entry.address())
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for PhysicalAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for PhysicalAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::LowerHex for PhysicalAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}
