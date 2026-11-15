#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

const FRAME_SIZE: usize = 4096;

// The smallest class must hold the free-list link a free slot stores.
const MIN_CLASS: usize = 16;

// A slab of two slots per frame would waste more than it saves.
const MAX_CLASS: usize = 2048;

pub const CLASSES: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlacementError {
    InvalidAlignment(usize),
}

impl fmt::Display for PlacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidAlignment(align) => {
                write!(
                    formatter,
                    "alignment {align} is not a non-zero power of two"
                )
            }
        }
    }
}

impl core::error::Error for PlacementError {}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct SizeClass {
    index: usize,
}

impl SizeClass {
    const fn from_index(index: usize) -> Option<Self> {
        if index < CLASSES {
            Some(Self { index })
        } else {
            None
        }
    }

    pub const fn index(self) -> usize {
        self.index
    }

    pub const fn bytes(self) -> usize {
        MIN_CLASS << self.index
    }

    pub const fn slots_per_frame(self) -> usize {
        FRAME_SIZE / self.bytes()
    }

    pub const fn offset_of(self, slot: usize) -> usize {
        assert!(slot < self.slots_per_frame(), "slot outside the slab");
        slot * self.bytes()
    }

    // Every slot is aligned to its own class size.
    const fn for_size_align(size: usize, align: usize) -> Result<Option<Self>, PlacementError> {
        if align == 0 || !align.is_power_of_two() {
            return Err(PlacementError::InvalidAlignment(align));
        }
        let needed = if size > align { size } else { align };
        if needed > MAX_CLASS {
            return Ok(None);
        }
        let bytes = if needed <= MIN_CLASS {
            MIN_CLASS
        } else {
            needed.next_power_of_two()
        };
        let index = (bytes / MIN_CLASS).trailing_zeros() as usize;
        Ok(Self::from_index(index))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    Slot(SizeClass),
    Frames { count: usize, align: usize },
}

pub const fn place(size: usize, align: usize) -> Result<Placement, PlacementError> {
    match SizeClass::for_size_align(size, align) {
        Err(reason) => Err(reason),
        Ok(Some(class)) => Ok(Placement::Slot(class)),
        // A zero-sized over-aligned request takes a frame: `GlobalAlloc` owes it an address.
        Ok(None) if size == 0 => Ok(Placement::Frames {
            count: 1,
            align: align.div_ceil(FRAME_SIZE),
        }),
        Ok(None) => Ok(Placement::Frames {
            count: size.div_ceil(FRAME_SIZE),
            align: align.div_ceil(FRAME_SIZE),
        }),
    }
}
