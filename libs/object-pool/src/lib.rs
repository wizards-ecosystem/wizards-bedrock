#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod slots;
#[cfg(test)]
mod tests;

use core::fmt;

pub use slots::{Issued, Released, Removed, Slots};

const SLOT_BITS: u32 = 10;

pub const PAGES: u32 = 1 << (u32::BITS - SLOT_BITS);

pub const SLOTS_PER_PAGE: u16 = 1 << SLOT_BITS;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Index {
    page: u32,
    slot: u16,
}

impl Index {
    pub fn new(page: u32, slot: u16) -> Option<Self> {
        if page >= PAGES || slot >= SLOTS_PER_PAGE {
            return None;
        }
        Some(Self { page, slot })
    }

    pub fn from_bits(bits: u32) -> Self {
        Self {
            page: bits >> SLOT_BITS,
            slot: (bits & (u32::from(SLOTS_PER_PAGE) - 1)) as u16,
        }
    }

    pub fn bits(self) -> u32 {
        (self.page << SLOT_BITS) | u32::from(self.slot)
    }

    pub fn page(self) -> u32 {
        self.page
    }

    pub fn slot(self) -> u16 {
        self.slot
    }
}

// One object page is one arena frame, so pages needed is frames needed.
pub fn frames_needed(objects: u32, free_slots: u32, per_page: u16) -> Option<u32> {
    if per_page == 0 || per_page > SLOTS_PER_PAGE {
        return None;
    }
    if objects <= free_slots {
        return Some(0);
    }
    let short = objects - free_slots;
    let per_page = u32::from(per_page);
    let whole = short / per_page;
    if short.is_multiple_of(per_page) {
        Some(whole)
    } else {
        whole.checked_add(1)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    Full,
    Stale,
    Condemned,
    Saturated,
    Held,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full => "no slot on the page is free",
            Self::Stale => "the slot holds no live object of that generation",
            Self::Condemned => "the object's strong count already reached zero",
            Self::Saturated => "the object's strong count is at its maximum",
            Self::Held => "the object still has strong references",
        })
    }
}

impl core::error::Error for Refusal {}
