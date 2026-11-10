//! How a heap request maps onto a size class, and how a frame is carved into
//! slots of it.
//!
//! The arithmetic half of the kernel heap; `wost::mm::heap` holds the free
//! lists and the frames (D-47).
//!
//! Classes are powers of two from 16 to 2048 bytes. A frame is 4096-aligned
//! and a class's slots sit at multiples of the class size, so every slot is
//! aligned to its own class — which is why
//! [`SizeClass::for_size_align`] meets an alignment by choosing a larger
//! class rather than by padding.
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

use core::fmt;

#[cfg(any(test, feature = "selftest"))]
mod ownership;

#[cfg(any(test, feature = "selftest"))]
pub use ownership::AllocatorOwnership;

/// Bytes in a page frame — the unit a slab is carved from.
pub const FRAME_SIZE: usize = 4096;

/// Bytes in the smallest class. Below this a slot could not hold the free-list
/// link that lives in it while it is free.
pub const MIN_CLASS: usize = 16;

/// Bytes in the largest class. Above this a request takes whole frames, since
/// a slab of two-per-frame slots wastes more than it saves.
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
    /// The class at `index`, or `None` if there is no such class.
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

    /// The smallest class holding `size` bytes at `align`, or `None` if no
    /// class is large enough.
    ///
    /// Alignment is satisfied by *choosing a class*, never by padding: because
    /// a frame is [`FRAME_SIZE`]-aligned and slots sit at multiples of the
    /// class size, a slot is always aligned to its own class. So a request
    /// needing 64-byte alignment takes the 64-byte class at least, whatever
    /// its size.
    ///
    /// # Errors
    ///
    /// [`PlacementError::InvalidAlignment`] unless `align` is a non-zero power
    /// of two, as a [`core::alloc::Layout`] requires.
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
        /// Frames the request needs.
        count: usize,
        /// Alignment the run needs, in frames.
        align: usize,
    },
}

/// Decide where a request of `size` bytes at `align` is served from.
///
/// A zero-sized request is given the smallest slot that meets its alignment
/// rather than refused: `Layout` permits a size of zero, `GlobalAlloc`
/// requires a unique non-null address back, and one slot is the cheapest way
/// to provide one. An alignment too large for any class takes a frame, which
/// is the same answer one byte at that alignment would get.
///
/// # Errors
///
/// [`PlacementError::InvalidAlignment`] unless `align` is a non-zero power of
/// two, as a [`core::alloc::Layout`] requires.
pub const fn place(size: usize, align: usize) -> Result<Placement, PlacementError> {
    match SizeClass::for_size_align(size, align) {
        Err(error) => Err(error),
        Ok(Some(class)) => Ok(Placement::Slot(class)),
        // At least one frame: a zero-sized, over-aligned request lands here
        // and `GlobalAlloc` owes it a unique non-null address.
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
