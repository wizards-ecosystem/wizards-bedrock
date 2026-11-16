#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

// The crate's only unverified code: Verus cannot check formatting.
#[cfg_attr(verus_only, verifier::external)]
mod fmt;
pub mod proof;
mod slots;
#[cfg(test)]
mod tests;

#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus_spec, verus_verify};

pub use slots::{Issued, Released, Removed, Slots};

#[verus_verify]
const SLOT_BITS: u32 = 10;

#[verus_verify]
pub const PAGES: u32 = 1 << 22;

#[verus_verify]
pub const SLOTS_PER_PAGE: u16 = 1 << 10;

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Index {
    page: u32,
    slot: u16,
}

#[verus_verify]
impl Index {
    #[verus_spec(r =>
        ensures
            match r {
                Some(index) => index.spec_page() == page && index.spec_slot() == slot,
                None => page >= PAGES || slot >= SLOTS_PER_PAGE,
            },
    )]
    pub fn new(page: u32, slot: u16) -> Option<Self> {
        proof! {
            proof::lemma_page_constants();
        }
        if page >= PAGES || slot >= SLOTS_PER_PAGE {
            return None;
        }
        Some(Self { page, slot })
    }

    #[verus_spec(r =>
        ensures
            proof::index_bits(r.spec_page(), r.spec_slot()) == bits,
    )]
    pub fn from_bits(bits: u32) -> Self {
        proof! {
            proof::lemma_page_constants();
            proof::lemma_bits_name_an_index(bits);
        }
        Self {
            page: bits >> SLOT_BITS,
            slot: (bits & (u32::from(SLOTS_PER_PAGE) - 1)) as u16,
        }
    }

    #[verus_spec(r =>
        ensures
            r == proof::index_bits(self.spec_page(), self.spec_slot()),
            r >> 10 == self.spec_page(),
            r & 1023 == self.spec_slot() as u32,
    )]
    pub fn bits(self) -> u32 {
        proof! {
            use_type_invariant(self);
            proof::lemma_index_names_its_bits(self.page, self.slot);
        }
        (self.page << SLOT_BITS) | u32::from(self.slot)
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_page(),
            r < PAGES,
    )]
    pub fn page(self) -> u32 {
        proof! {
            use_type_invariant(self);
            proof::lemma_page_constants();
        }
        self.page
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_slot(),
            r < SLOTS_PER_PAGE,
    )]
    pub fn slot(self) -> u16 {
        proof! {
            use_type_invariant(self);
            proof::lemma_page_constants();
        }
        self.slot
    }
}

// One object page is one arena frame, so pages needed is frames needed.
#[verus_spec(r =>
    ensures
        match r {
            Some(frames) => 0 < per_page <= SLOTS_PER_PAGE
                && proof::frames_cover_the_batch(objects, free_slots, per_page, frames),
            None => per_page == 0 || per_page > SLOTS_PER_PAGE,
        },
)]
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
    proof! {
        proof::lemma_whole_pages(short, per_page, whole);
    }
    if short.is_multiple_of(per_page) {
        Some(whole)
    } else {
        Some(whole + 1)
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    Full,
    Stale,
    Condemned,
    Saturated,
    Held,
}
