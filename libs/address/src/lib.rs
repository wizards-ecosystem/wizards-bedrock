#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

use x86_paging::{Entry, MAX_PHYSICAL};

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalAddress(u64);

impl PhysicalAddress {
    pub const fn new(address: u64) -> Option<Self> {
        if address < MAX_PHYSICAL {
            Some(Self(address))
        } else {
            None
        }
    }

    pub const fn from_entry(entry: Entry) -> Self {
        Self(entry.address())
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for PhysicalAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl fmt::Display for PhysicalAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl fmt::LowerHex for PhysicalAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, formatter)
    }
}
