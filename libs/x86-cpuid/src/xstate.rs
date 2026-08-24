//! Where each piece of extended processor state sits in a saved area.
//!
//! D-82 settled the policy — eager, compacted, boot-sized, `XCR0` limited to
//! x87, SSE and AVX — and put the arithmetic here on the grounds `010` §8
//! gives: *compute offsets from `CPUID.0DH` into a table in a `libs/` crate,
//! host-tested against synthetic CPUID snapshots including the APX case.*
//!
//! **The APX case is why this is not a formula.** In the *non-compacted*
//! format the processor reports an offset per component, and those offsets are
//! not ordered by index: APX is state component 19 and sits at offset 960, in
//! the hole deprecated MPX left behind — a higher index at a lower offset.
//! Linux assumed monotonicity, and 6.16 deleted the assertions that said so.
//! Nothing here may sort by offset or assume index order implies address
//! order.
//!
//! In the **compacted** format, which is the one this kernel uses, offsets are
//! not reported at all: they follow from the enabled mask, each component
//! placed after the last, with a sixty-four byte alignment where the processor
//! asks for one. That is what [`Layout`] computes.
//!
//! # Examples
//!
//! ```
//! use x86_cpuid::xstate::{Component, Layout, HEADER_BYTES, AVX, SSE, X87};
//!
//! // A machine with AVX, whose component 2 is 256 bytes and unaligned.
//! let layout = Layout::compacted(X87 | SSE | AVX, |index| match index {
//!     2 => Component::new(256, false),
//!     _ => Component::ABSENT,
//! });
//!
//! // x87 and SSE live in the legacy area, so AVX is the first thing after it.
//! assert_eq!(layout.offset_of(2), Some(HEADER_BYTES));
//! assert_eq!(layout.size(), HEADER_BYTES + 256);
//! ```

/// Legacy x87/SSE save area plus the XSAVE header: 512 + 64 bytes.
///
/// Components 0 and 1 are *in* that area rather than after it, which is why
/// neither ever gets an offset of its own.
pub const HEADER_BYTES: usize = 576;

/// State component 0: x87.
pub const X87: u64 = 1 << 0;

/// State component 1: SSE.
pub const SSE: u64 = 1 << 1;

/// State component 2: AVX's upper halves.
pub const AVX: u64 = 1 << 2;

/// What D-82 permits in `XCR0`, and nothing else.
///
/// Every enabled bit is per-thread memory for the life of the kernel, and
/// AMX's `XTILEDATA` alone is 8 KiB today and up to 64 KiB architecturally.
pub const PERMITTED: u64 = X87 | SSE | AVX;

/// The highest state component this crate will lay out.
///
/// Twenty because APX is component 19 and is the case the layout has to get
/// right; there is no cost to the two above it.
pub const COMPONENTS: usize = 20;

/// What the processor says about one state component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    size: u32,
    aligned: bool,
}

impl Component {
    /// A component the processor does not have.
    pub const ABSENT: Self = Self {
        size: 0,
        aligned: false,
    };

    /// A component of `size` bytes, sixty-four-byte aligned or not.
    ///
    /// `aligned` is `CPUID.0DH.ECX` bit 1 for this component.
    pub const fn new(size: u32, aligned: bool) -> Self {
        Self { size, aligned }
    }

    /// How many bytes it takes.
    pub const fn size(self) -> u32 {
        self.size
    }

    /// Whether it must start on a sixty-four-byte boundary.
    pub const fn is_aligned(self) -> bool {
        self.aligned
    }

    /// Whether the processor has it at all.
    pub const fn is_present(self) -> bool {
        self.size != 0
    }
}

/// Where every enabled component sits in a compacted save area.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    offsets: [Option<u32>; COMPONENTS],
    size: usize,
}

impl Layout {
    /// Lay out the components `mask` enables, asking `info` about each.
    ///
    /// `info` is called **only** for components the mask enables and only for
    /// indices two and above, so a caller wiring it to `CPUID.0DH` sub-leaves
    /// never queries one the processor did not advertise.
    ///
    /// Bits above [`COMPONENTS`] are ignored rather than rejected: a processor
    /// may report state this kernel has no intention of enabling, and the mask
    /// it is *given* is the one D-82 fixed.
    pub fn compacted(mask: u64, info: impl Fn(u32) -> Component) -> Self {
        let mut offsets = [None; COMPONENTS];
        // The legacy area is always present in a compacted save, whether or
        // not x87 and SSE are enabled — the header follows it either way.
        let mut cursor = HEADER_BYTES;

        for (index, offset) in offsets.iter_mut().enumerate().skip(2) {
            if mask & (1 << index) == 0 {
                continue;
            }
            let component = info(index as u32);
            if !component.is_present() {
                continue;
            }
            if component.is_aligned() {
                cursor = cursor.next_multiple_of(64);
            }
            *offset = Some(cursor as u32);
            cursor += component.size() as usize;
        }

        Self {
            offsets,
            size: cursor,
        }
    }

    /// How many bytes a save area needs.
    ///
    /// Never less than [`HEADER_BYTES`], because the legacy area and the
    /// header are there whatever is enabled.
    pub const fn size(&self) -> usize {
        self.size
    }

    /// Where component `index` starts, or `None` if it is not in this layout.
    ///
    /// `None` for components 0 and 1 as well: they are inside the legacy area
    /// rather than placed after it, so an offset for them would be a lie a
    /// caller could dereference.
    pub const fn offset_of(&self, index: u32) -> Option<usize> {
        if index as usize >= COMPONENTS {
            return None;
        }
        match self.offsets[index as usize] {
            Some(offset) => Some(offset as usize),
            None => None,
        }
    }

    /// Whether every enabled component landed somewhere inside the area.
    ///
    /// The invariant a caller should assert before handing the size to an
    /// allocator: an offset past the end would be a save that writes outside
    /// its buffer.
    pub fn is_consistent(&self) -> bool {
        self.offsets
            .iter()
            .flatten()
            .all(|offset| (*offset as usize) < self.size)
            && self.size >= HEADER_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine with AVX only, which is what D-82 permits.
    fn avx_only(index: u32) -> Component {
        match index {
            2 => Component::new(256, false),
            _ => Component::ABSENT,
        }
    }

    /// A snapshot with AVX-512 and APX, whose *non-compacted* offsets are not
    /// ordered by index.
    ///
    /// Sizes are the architectural ones: `ZMM_Hi256` 512, `Hi16_ZMM` 1024,
    /// `PT` 128, `PKRU` 8, and APX's `EGPR` 80. The point of the snapshot is
    /// the ordering, not the exact bytes.
    fn avx512_and_apx(index: u32) -> Component {
        match index {
            2 => Component::new(256, false),
            5 => Component::new(64, false),
            6 => Component::new(512, false),
            7 => Component::new(1024, false),
            8 => Component::new(128, false),
            9 => Component::new(8, false),
            19 => Component::new(80, true),
            _ => Component::ABSENT,
        }
    }

    /// An empty mask still costs the legacy area and the header.
    #[test]
    fn the_header_is_always_there() {
        let layout = Layout::compacted(0, |_| Component::ABSENT);
        assert_eq!(layout.size(), HEADER_BYTES);
        assert!(layout.is_consistent());

        // x87 and SSE are in the legacy area, so enabling them adds nothing
        // and neither gets an offset of its own.
        let legacy = Layout::compacted(X87 | SSE, |_| Component::ABSENT);
        assert_eq!(legacy.size(), HEADER_BYTES);
        assert_eq!(legacy.offset_of(0), None);
        assert_eq!(legacy.offset_of(1), None);
    }

    /// The layout D-82 actually permits.
    #[test]
    fn the_permitted_mask_places_avx_after_the_header() {
        let layout = Layout::compacted(PERMITTED, avx_only);
        assert_eq!(layout.offset_of(2), Some(HEADER_BYTES));
        assert_eq!(layout.size(), HEADER_BYTES + 256);
        assert!(layout.is_consistent());
        assert_eq!(PERMITTED, 0b111, "D-82 permits x87, SSE and AVX only");
    }

    /// **A component with a higher index at a lower offset does not disturb
    /// the compacted layout.**
    ///
    /// The APX case (`010` §8). In the non-compacted format component 19 sits
    /// at offset 960, below components 6, 7 and 8 — Linux assumed offsets rose
    /// with index and had to delete the assertions in 6.16. Compacted offsets
    /// are computed from the mask in *index* order and never read the
    /// processor's offsets at all, so the hazard cannot reach this code. This
    /// test is what says so.
    #[test]
    fn a_high_index_at_a_low_offset_changes_nothing() {
        let mask = 0b111 | (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 19);
        let layout = Layout::compacted(mask, avx512_and_apx);

        // Index order, and each after the last.
        let mut previous = HEADER_BYTES;
        for index in [2u32, 5, 6, 7, 8, 9, 19] {
            let offset = layout
                .offset_of(index)
                .unwrap_or_else(|| panic!("component {index} was enabled but not placed"));
            assert!(
                offset >= previous,
                "component {index} at {offset} overlaps what came before it"
            );
            previous = offset + avx512_and_apx(index).size() as usize;
        }

        // APX asks for alignment, so it starts on a sixty-four byte boundary
        // and the area is exactly long enough to hold it.
        let apx = layout.offset_of(19).expect("apx was enabled");
        assert_eq!(apx % 64, 0, "an aligned component started unaligned");
        assert_eq!(layout.size(), apx + 80);
        assert!(layout.is_consistent());
    }

    /// Alignment padding goes before the component, not after.
    #[test]
    fn alignment_pads_ahead_of_the_component() {
        // Component 2 is 8 bytes, so component 3 would start at 584 — but it
        // asks for alignment, so it starts at 640.
        let layout = Layout::compacted(0b1111, |index| match index {
            2 => Component::new(8, false),
            3 => Component::new(16, true),
            _ => Component::ABSENT,
        });
        assert_eq!(layout.offset_of(2), Some(HEADER_BYTES));
        assert_eq!(layout.offset_of(3), Some(640));
        assert_eq!(layout.size(), 640 + 16);
    }

    /// A component the mask enables but the processor does not have is skipped
    /// rather than given a zero-length slot.
    ///
    /// A zero-length slot would share an offset with whatever came next, and
    /// two components at one address is the shape that corrupts a restore.
    #[test]
    fn an_absent_component_takes_no_room() {
        let layout = Layout::compacted(0b1111, |index| match index {
            3 => Component::new(32, false),
            _ => Component::ABSENT,
        });
        assert_eq!(layout.offset_of(2), None, "an absent component was placed");
        assert_eq!(layout.offset_of(3), Some(HEADER_BYTES));
        assert_eq!(layout.size(), HEADER_BYTES + 32);
    }

    /// No two enabled components share an offset, and none starts past the
    /// end.
    ///
    /// Checked by collecting them rather than by re-deriving the walk, so a
    /// rewritten `compacted` disagrees instead of agreeing with itself.
    #[test]
    fn no_two_components_share_an_offset() {
        let mask = 0b111 | (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 19);
        let layout = Layout::compacted(mask, avx512_and_apx);

        let mut placed: Vec<usize> = (0..COMPONENTS as u32)
            .filter_map(|index| layout.offset_of(index))
            .collect();
        let count = placed.len();
        placed.sort_unstable();
        placed.dedup();
        assert_eq!(placed.len(), count, "two components share an offset");
        assert!(
            placed.iter().all(|offset| *offset < layout.size()),
            "a component starts past the end of the area"
        );
        assert!(layout.is_consistent());
    }

    /// Bits above the components this crate lays out are ignored, not
    /// misplaced.
    #[test]
    fn a_mask_bit_beyond_the_table_is_ignored() {
        let layout = Layout::compacted(PERMITTED | (1 << 40) | (1 << 63), avx_only);
        assert_eq!(layout.size(), HEADER_BYTES + 256);
        assert_eq!(layout.offset_of(40), None);
        assert_eq!(layout.offset_of(63), None);
        assert!(layout.is_consistent());
    }
}
