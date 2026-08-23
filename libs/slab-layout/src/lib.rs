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
//! let class = SizeClass::for_size_align(20, 8).expect("small enough");
//! assert_eq!(class.bytes(), 32);
//! assert_eq!(class.slots_per_frame(), 128);
//!
//! // A 64 KiB request is too big for any class and takes whole frames.
//! assert_eq!(place(64 * 1024, 8), Placement::Frames { count: 16, align: 1 });
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

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

/// One size class of the slab allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
    pub const fn for_size_align(size: usize, align: usize) -> Option<Self> {
        let needed = if size > align { size } else { align };
        if needed > MAX_CLASS {
            return None;
        }

        let mut index = 0;
        while index < CLASSES {
            if MIN_CLASS << index >= needed {
                return Some(Self { index });
            }
            index += 1;
        }
        None
    }
}

/// Where a request of a given size and alignment is served from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
/// A zero-sized request is given the smallest slot rather than refused: `Layout`
/// permits a size of zero, `GlobalAlloc` requires a unique non-null address
/// back, and one slot is the cheapest way to provide one.
pub const fn place(size: usize, align: usize) -> Placement {
    match SizeClass::for_size_align(size, align) {
        Some(class) => Placement::Slot(class),
        None => Placement::Frames {
            count: size.div_ceil(FRAME_SIZE),
            align: align.div_ceil(FRAME_SIZE),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_are_powers_of_two_from_sixteen_to_two_thousand_and_forty_eight() {
        let sizes: Vec<_> = (0..CLASSES)
            .map(|index| SizeClass::from_index(index).expect("in range").bytes())
            .collect();
        assert_eq!(sizes, [16, 32, 64, 128, 256, 512, 1024, 2048]);
        assert_eq!(sizes[0], MIN_CLASS);
        assert_eq!(sizes[CLASSES - 1], MAX_CLASS);
        assert_eq!(SizeClass::from_index(CLASSES), None);
    }

    #[test]
    fn a_frame_divides_exactly_into_slots_of_every_class() {
        for index in 0..CLASSES {
            let class = SizeClass::from_index(index).expect("in range");
            assert_eq!(
                class.slots_per_frame() * class.bytes(),
                FRAME_SIZE,
                "class {} leaves a remainder in a frame",
                class.bytes()
            );
        }
    }

    #[test]
    fn a_request_takes_the_smallest_class_that_holds_it() {
        assert_eq!(
            SizeClass::for_size_align(1, 1).map(SizeClass::bytes),
            Some(16)
        );
        assert_eq!(
            SizeClass::for_size_align(16, 1).map(SizeClass::bytes),
            Some(16)
        );
        assert_eq!(
            SizeClass::for_size_align(17, 1).map(SizeClass::bytes),
            Some(32)
        );
        assert_eq!(
            SizeClass::for_size_align(2048, 1).map(SizeClass::bytes),
            Some(2048)
        );
        assert_eq!(SizeClass::for_size_align(2049, 1), None);
    }

    #[test]
    fn alignment_is_met_by_taking_a_larger_class_not_by_padding() {
        // Eight bytes needing 64-byte alignment: the 16-byte class would place
        // it at a multiple of 16, which is not necessarily a multiple of 64.
        let class = SizeClass::for_size_align(8, 64).expect("small enough");
        assert_eq!(class.bytes(), 64);
        assert_eq!(
            SizeClass::for_size_align(8, 2048).map(SizeClass::bytes),
            Some(2048)
        );
        assert_eq!(
            SizeClass::for_size_align(8, 4096),
            None,
            "an alignment past the largest class needs whole frames"
        );
    }

    #[test]
    fn every_slot_of_every_class_is_aligned_to_its_class() {
        // The invariant the whole design rests on. A frame is FRAME_SIZE
        // aligned, so an offset that is a multiple of the class size lands on
        // an address that is too.
        for index in 0..CLASSES {
            let class = SizeClass::from_index(index).expect("in range");
            assert!(FRAME_SIZE.is_multiple_of(class.bytes()));
            for slot in 0..class.slots_per_frame() {
                assert!(class.offset_of(slot).is_multiple_of(class.bytes()));
            }
        }
    }

    #[test]
    fn slot_offsets_fill_the_frame_without_overlapping() {
        let class = SizeClass::for_size_align(256, 1).expect("a class");
        let last = class.slots_per_frame() - 1;
        assert_eq!(class.offset_of(0), 0);
        assert_eq!(class.offset_of(last), FRAME_SIZE - class.bytes());
        assert_eq!(
            class.offset_of(last) + class.bytes(),
            FRAME_SIZE,
            "the last slot ends exactly at the frame's end"
        );
    }

    #[test]
    #[should_panic(expected = "slot outside the slab")]
    fn a_slot_past_the_end_of_a_slab_panics() {
        let class = SizeClass::for_size_align(16, 1).expect("a class");
        let _ = class.offset_of(class.slots_per_frame());
    }

    #[test]
    fn a_small_request_is_placed_in_a_slot() {
        assert_eq!(
            place(100, 8),
            Placement::Slot(SizeClass::for_size_align(100, 8).expect("a class"))
        );
    }

    #[test]
    fn a_large_request_is_placed_in_whole_frames() {
        assert_eq!(place(4096, 1), Placement::Frames { count: 1, align: 1 });
        assert_eq!(place(4097, 1), Placement::Frames { count: 2, align: 1 });
        assert_eq!(
            place(2049, 1),
            Placement::Frames { count: 1, align: 1 },
            "one byte past the largest class already needs a frame"
        );
        assert_eq!(
            place(8192, 8192),
            Placement::Frames { count: 2, align: 2 },
            "alignment is counted in frames, like the run itself"
        );
    }

    #[test]
    fn a_zero_sized_request_still_gets_a_slot() {
        // `GlobalAlloc` must return a unique non-null pointer, so zero bytes
        // are served rather than refused.
        assert_eq!(
            place(0, 1),
            Placement::Slot(SizeClass::from_index(0).expect("the smallest class"))
        );
    }
}
