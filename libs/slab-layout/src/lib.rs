#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

// The crate's only unverified code: Verus cannot check formatting.
#[cfg_attr(verus_only, verifier::external)]
mod fmt;
pub mod proof;

#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus, verus_spec, verus_verify};

#[verus_verify]
const FRAME_SIZE: usize = 4096;

// The smallest class must hold the free-list link a free slot stores.
#[verus_verify]
const MIN_CLASS: usize = 16;

// The largest class is 2048 bytes: a slab of one slot per frame would waste more than it saves.
#[verus_verify]
pub const CLASSES: usize = 8;

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlacementError {
    InvalidAlignment(usize),
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct SizeClass {
    index: usize,
}

#[verus_verify]
impl SizeClass {
    #[verus_spec(r =>
        ensures
            r == self.spec_index(),
            r < CLASSES,
    )]
    pub const fn index(self) -> usize {
        proof! {
            use_type_invariant(self);
        }
        self.index
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_bytes(),
            16 <= r <= 2048,
    )]
    pub const fn bytes(self) -> usize {
        proof! {
            use_type_invariant(self);
            proof::lemma_class_bytes(self.index);
        }
        MIN_CLASS << self.index
    }

    #[verus_spec(r =>
        ensures
            match r {
                Some(offset) => offset == slot * self.spec_bytes()
                    && offset + self.spec_bytes() <= 4096,
                None => slot * self.spec_bytes() + self.spec_bytes() > 4096,
            },
    )]
    pub const fn offset_of(self, slot: usize) -> Option<usize> {
        let bytes = self.bytes();
        proof! {
            proof::lemma_slot_in_frame(slot, bytes);
        }
        if slot < FRAME_SIZE / bytes {
            Some(slot * bytes)
        } else {
            None
        }
    }
}

verus! {

impl SizeClass {
    // Every slot is aligned to its own class size.
    const fn for_size_align(size: usize, align: usize) -> (r: Result<Option<Self>, PlacementError>)
        ensures
            match r {
                Err(reason) => reason == PlacementError::InvalidAlignment(align)
                    && !proof::is_power_of_two(align),
                Ok(Some(class)) => proof::is_power_of_two(align)
                    && proof::smallest_fit(class.spec_index(), size, align),
                Ok(None) => proof::is_power_of_two(align) && proof::no_fit(size, align),
            },
    {
        if !is_power_of_two(align) {
            return Err(PlacementError::InvalidAlignment(align));
        }
        let mut index = 0;
        while index < CLASSES
            invariant
                index <= CLASSES,
                proof::is_power_of_two(align),
                forall|smaller: usize| smaller < index ==> !proof::fits(smaller, size, align),
            decreases CLASSES - index,
        {
            let bytes = MIN_CLASS << index;
            proof {
                proof::lemma_fit_by_order(index, align);
            }
            if size <= bytes && align <= bytes {
                return Ok(Some(Self { index }));
            }
            index += 1;
        }
        Ok(None)
    }
}

const fn is_power_of_two(value: usize) -> (r: bool)
    ensures
        r == proof::is_power_of_two(value),
{
    let mut shift = 0;
    while shift < usize::BITS
        invariant
            shift <= usize::BITS,
            forall|smaller: u32| smaller < shift ==> value != 1usize << smaller,
        decreases usize::BITS - shift,
    {
        if value == 1 << shift {
            return true;
        }
        shift += 1;
    }
    false
}

} // verus!

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    Slot(SizeClass),
    Frames { count: usize, align: usize },
}

#[verus_spec(r =>
    ensures
        match r {
            Err(reason) => reason == PlacementError::InvalidAlignment(align)
                && !proof::is_power_of_two(align),
            Ok(Placement::Slot(class)) => proof::is_power_of_two(align)
                && proof::smallest_fit(class.spec_index(), size, align),
            Ok(Placement::Frames { count, align: frames }) => proof::is_power_of_two(align)
                && proof::no_fit(size, align)
                && count as int == if size == 0 { 1 } else { proof::frames_covering(size) }
                && frames as int == proof::frames_covering(align),
        },
)]
pub const fn place(size: usize, align: usize) -> Result<Placement, PlacementError> {
    match SizeClass::for_size_align(size, align) {
        Err(reason) => Err(reason),
        Ok(Some(class)) => Ok(Placement::Slot(class)),
        // A zero-sized over-aligned request takes a frame: `GlobalAlloc` owes it an address.
        Ok(None) if size == 0 => Ok(Placement::Frames {
            count: 1,
            align: frames_for(align),
        }),
        Ok(None) => Ok(Placement::Frames {
            count: frames_for(size),
            align: frames_for(align),
        }),
    }
}

#[verus_spec(r =>
    ensures
        r as int == proof::frames_covering(bytes),
)]
const fn frames_for(bytes: usize) -> usize {
    let whole = bytes / FRAME_SIZE;
    if bytes.is_multiple_of(FRAME_SIZE) {
        whole
    } else {
        whole + 1
    }
}
