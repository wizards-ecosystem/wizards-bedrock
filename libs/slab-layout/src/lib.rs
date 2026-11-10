//! How a heap request maps onto a size class, and how a frame is carved into slots of it.
//!
//! ```
//! use slab_layout::{SizeClass, Placement, place};
//!
//! // A 20-byte request with 8-byte alignment fits the 32-byte class.
//! let class = SizeClass::for_size_align(20, 8)
//!     .expect("valid alignment")
//!     .expect("small enough");
//! assert_eq!(class.bytes(), 32);
//! assert_eq!(class.slots_per_frame(), 128);
//!
//! // A 64 KiB request is too big for any class and takes whole frames.
//! assert_eq!(
//!     place(64 * 1024, 8),
//!     Ok(Placement::Frames { count: 16, align: 1 })
//! );
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

#[cfg(any(test, feature = "selftest"))]
mod ownership;

#[cfg(any(test, feature = "selftest"))]
pub use ownership::AllocatorOwnership;

use core::fmt;

/// Bytes in a page frame, the unit a slab is carved from.
pub const FRAME_SIZE: usize = 4096;

/// Bytes in the smallest class, which must hold the free-list link a free slot stores.
pub const MIN_CLASS: usize = 16;

/// Bytes in the largest class; a slab of two slots per frame would waste more than it saves.
pub const MAX_CLASS: usize = 2048;

/// Number of size classes, from [`MIN_CLASS`] to [`MAX_CLASS`].
pub const CLASSES: usize = 8;

/// Why a request cannot be placed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum PlacementError {
    /// Rust alignments are non-zero powers of two.
    InvalidAlignment(usize),
}

impl fmt::Display for PlacementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidAlignment(align) => {
                write!(f, "alignment {align} is not a non-zero power of two")
            }
        }
    }
}

impl core::error::Error for PlacementError {}

/// One size class of the slab allocator.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct SizeClass {
    /// Index into the free lists: 0 is [`MIN_CLASS`].
    index: usize,
}

impl SizeClass {
    pub const fn from_index(index: usize) -> Option<Self> {
        if index < CLASSES {
            Some(Self { index })
        } else {
            None
        }
    }

    /// Which free list this class uses.
    pub const fn index(self) -> usize {
        self.index
    }

    /// Bytes in one slot of this class.
    pub const fn bytes(self) -> usize {
        MIN_CLASS << self.index
    }

    /// Slots one frame is carved into.
    pub const fn slots_per_frame(self) -> usize {
        FRAME_SIZE / self.bytes()
    }

    /// Byte offset of slot `slot` within a frame.
    ///
    /// # Panics
    ///
    /// Panics if `slot` is not below [`slots_per_frame`](Self::slots_per_frame).
    pub const fn offset_of(self, slot: usize) -> usize {
        assert!(slot < self.slots_per_frame(), "slot outside the slab");
        slot * self.bytes()
    }

    /// The smallest class holding `size` at `align`, since every slot is aligned to its own class.
    pub const fn for_size_align(size: usize, align: usize) -> Result<Option<Self>, PlacementError> {
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

/// Where a request of a given size and alignment is served from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    /// A slot of this class, taken from a slab.
    Slot(SizeClass),
    /// A run of whole frames, too big for any class.
    Frames {
        count: usize,
        /// Alignment the run needs, in frames.
        align: usize,
    },
}

/// Decide where `size` bytes at `align` are served from; zero bytes still get a unique address.
pub const fn place(size: usize, align: usize) -> Result<Placement, PlacementError> {
    match SizeClass::for_size_align(size, align) {
        Err(reason) => Err(reason),
        Ok(Some(class)) => Ok(Placement::Slot(class)),
        // A zero-sized over-aligned request takes a frame, as `GlobalAlloc` owes it an address.
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

#[cfg(test)]
mod tests;
