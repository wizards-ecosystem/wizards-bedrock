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
//! use x86_cpuid::xstate::{Alignment, Component, Layout, HEADER_BYTES, AVX, SSE, X87};
//!
//! // A machine with AVX, whose component 2 is 256 bytes and unaligned.
//! let layout = Layout::compacted(X87 | SSE | AVX, |index| match index {
//!     2 => Component::new(256, Alignment::Natural),
//!     _ => Component::new(0, Alignment::Natural),
//! });
//!
//! // x87 and SSE live in the legacy area, so AVX is the first thing after it.
//! assert_eq!(layout.size(), HEADER_BYTES + 256);
//! assert!(layout.is_consistent());
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

const AVX_SUPPORTED: u32 = 1 << 28;

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

/// Whether the processor supplies D-82's complete fixed state policy.
pub const fn has_permitted_components(available: u64) -> bool {
    available & PERMITTED == PERMITTED
}

/// Whether the live configured mask is exactly the fixed state policy.
///
/// Availability may include components the kernel declines to enable. A live
/// configuration may not: save-area sizing covers exactly [`PERMITTED`].
pub const fn matches_permitted_configuration(configured: u64) -> bool {
    configured == PERMITTED
}

/// Whether AVX state in `mask` has AVX instruction support.
///
/// `CPUID.0DH` advertising AVX state does not make AVX instructions legal;
/// leaf 1 must independently advertise them before `XCR0.AVX` is enabled.
pub const fn avx_state_has_instruction_support(mask: u64, basic_ecx: u32) -> bool {
    mask & AVX == 0 || basic_ecx & AVX_SUPPORTED != 0
}

/// What the processor says about one state component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    size: u32,
    alignment: Alignment,
}

/// Placement required by a state component's CPUID sub-leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    /// Place directly after the preceding component.
    Natural,
    /// Round the start up to a sixty-four-byte boundary.
    Bytes64,
}

impl Component {
    #[cfg(test)]
    const ABSENT: Self = Self {
        size: 0,
        alignment: Alignment::Natural,
    };

    /// A component of `size` bytes with its reported placement requirement.
    ///
    /// [`Alignment::Bytes64`] corresponds to `CPUID.0DH.ECX` bit 1.
    pub const fn new(size: u32, alignment: Alignment) -> Self {
        Self { size, alignment }
    }

    /// How many bytes it takes.
    pub const fn size(self) -> u32 {
        self.size
    }

    /// Whether it must start on a sixty-four-byte boundary.
    pub const fn is_aligned(self) -> bool {
        matches!(self.alignment, Alignment::Bytes64)
    }

    /// Whether the processor has it at all.
    pub const fn is_present(self) -> bool {
        self.size != 0
    }
}

/// End of one component in the standard XSAVE format.
///
/// Standard offsets come from `CPUID.0DH` and need not follow component index
/// order. An absent component, one overlapping the legacy/header area, or an
/// extent outside CPUID's 32-bit size fields is invalid.
pub const fn standard_component_extent(component: Component, offset: u32) -> Option<usize> {
    if !component.is_present() || offset < HEADER_BYTES as u32 {
        return None;
    }
    match offset.checked_add(component.size()) {
        Some(end) => Some(end as usize),
        None => None,
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
    /// A mask that enables AVX without SSE, or an enabled component whose
    /// sub-leaf is absent, produces an inconsistent layout.
    ///
    /// Bits above [`COMPONENTS`] are ignored rather than rejected: a processor
    /// may report state this kernel has no intention of enabling, and the mask
    /// it is *given* is the one D-82 fixed.
    pub fn compacted(mask: u64, info: impl Fn(u32) -> Component) -> Self {
        let mut offsets = [None; COMPONENTS];
        // The legacy area is always present in a compacted save, whether or
        // not x87 and SSE are enabled — the header follows it either way.
        let mut cursor = HEADER_BYTES;

        let mut complete = mask & AVX == 0 || mask & SSE != 0;
        for (index, offset) in offsets.iter_mut().enumerate().skip(2) {
            if mask & (1 << index) == 0 {
                continue;
            }
            let component = info(index as u32);
            if !component.is_present() {
                complete = false;
                break;
            }
            if component.is_aligned() {
                cursor = cursor.next_multiple_of(64);
            }
            // Truncation could give two components one save offset (D-105).
            let Ok(narrowed) = u32::try_from(cursor) else {
                complete = false;
                break;
            };
            *offset = Some(narrowed);
            cursor += component.size() as usize;
        }

        Self {
            offsets,
            size: if complete { cursor } else { 0 },
        }
    }

    /// How many bytes a save area needs.
    ///
    /// [`HEADER_BYTES`] or more for a layout that came out consistent, because
    /// the legacy area and the header are there whatever is enabled — and
    /// **zero for one that did not**, which is how an overflowing layout
    /// reports itself rather than by aliasing two components. This said "never
    /// less than [`HEADER_BYTES`]" two lines above the very function written to
    /// catch that case (D-124, D-132), so a caller reading it would size an
    /// allocator from a number this cannot promise. Ask
    /// [`is_consistent`](Self::is_consistent) first; it is the invariant, and
    /// this is only a number.
    pub const fn size(&self) -> usize {
        self.size
    }

    #[cfg(test)]
    const fn offset_of(&self, index: u32) -> Option<usize> {
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
    ///
    /// An overflowing layout reports zero so it cannot silently alias two
    /// components (D-105).
    pub fn is_consistent(&self) -> bool {
        let no_duplicates = self.offsets.iter().enumerate().all(|(index, offset)| {
            offset.is_none_or(|offset| {
                !self.offsets[..index]
                    .iter()
                    .flatten()
                    .any(|earlier| *earlier == offset)
            })
        });
        no_duplicates
            && self
                .offsets
                .iter()
                .flatten()
                .all(|offset| (*offset as usize) < self.size)
            && self.size >= HEADER_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine whose only extended component is AVX.
    fn avx_only(index: u32) -> Component {
        match index {
            2 => Component::new(256, Alignment::Natural),
            _ => Component::ABSENT,
        }
    }

    #[test]
    fn component_accessors_preserve_size_presence_and_alignment_independently() {
        let aligned = Component::new(17, Alignment::Bytes64);
        assert_eq!(aligned.size(), 17);
        assert!(aligned.is_aligned());
        assert!(aligned.is_present());

        let empty_but_aligned = Component::new(0, Alignment::Bytes64);
        assert_eq!(empty_but_aligned.size(), 0);
        assert!(empty_but_aligned.is_aligned());
        assert!(!empty_but_aligned.is_present());
        assert!(!Component::ABSENT.is_aligned());
        assert!(!Component::ABSENT.is_present());
    }

    #[test]
    fn a_standard_component_extent_uses_the_reported_offset() {
        let avx = Component::new(256, Alignment::Natural);
        assert_eq!(standard_component_extent(avx, 576), Some(832));
        assert_eq!(standard_component_extent(Component::ABSENT, 576), None);
        assert_eq!(standard_component_extent(avx, 575), None);
        assert_eq!(
            standard_component_extent(Component::new(2, Alignment::Natural), u32::MAX),
            None
        );
    }

    /// A snapshot with AVX-512 and APX, whose *non-compacted* offsets are not
    /// ordered by index.
    ///
    /// Sizes are the architectural ones: `ZMM_Hi256` 512, `Hi16_ZMM` 1024,
    /// `PT` 128, `PKRU` 8, and APX's `EGPR` 80. The point of the snapshot is
    /// the ordering, not the exact bytes.
    fn avx512_and_apx(index: u32) -> Component {
        match index {
            2 => Component::new(256, Alignment::Natural),
            5 => Component::new(64, Alignment::Natural),
            6 => Component::new(512, Alignment::Natural),
            7 => Component::new(1024, Alignment::Natural),
            8 => Component::new(128, Alignment::Natural),
            9 => Component::new(8, Alignment::Natural),
            19 => Component::new(80, Alignment::Bytes64),
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
    /// In the non-compacted APX layout, component 19 sits below components 6,
    /// 7 and 8. Compacted offsets use mask-index order and ignore those
    /// processor offsets (`010` §8).
    #[test]
    fn a_high_index_at_a_low_offset_changes_nothing() {
        let mask = 0b111 | (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 19);
        let layout = Layout::compacted(mask, avx512_and_apx);

        // Hand-calculated: deriving the expectation from `Layout` let every
        // wrong monotonic implementation pass (D-105).
        assert_eq!(layout.offset_of(2), Some(576), "avx");
        assert_eq!(layout.offset_of(5), Some(832), "opmask");
        assert_eq!(layout.offset_of(6), Some(896), "zmm hi256");
        assert_eq!(layout.offset_of(7), Some(1408), "hi16 zmm");
        assert_eq!(layout.offset_of(8), Some(2432), "pt");
        assert_eq!(layout.offset_of(9), Some(2560), "pkru");
        // The components inside the legacy area are never placed after it.
        assert_eq!(layout.offset_of(0), None, "x87");
        assert_eq!(layout.offset_of(1), None, "sse");

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
            2 => Component::new(8, Alignment::Natural),
            3 => Component::new(16, Alignment::Bytes64),
            _ => Component::ABSENT,
        });
        assert_eq!(layout.offset_of(2), Some(HEADER_BYTES));
        assert_eq!(layout.offset_of(3), Some(640));
        assert_eq!(layout.size(), 640 + 16);
    }

    #[test]
    fn every_enabled_extended_component_must_be_present() {
        for index in 2..COMPONENTS {
            let layout = Layout::compacted(X87 | SSE | (1 << index), |_| Component::ABSENT);
            assert!(
                !layout.is_consistent(),
                "absent component {index} was accepted"
            );
        }
    }

    #[test]
    fn avx_without_sse_is_not_consistent() {
        let layout = Layout::compacted(X87 | AVX, avx_only);
        assert!(!layout.is_consistent(), "AVX without SSE was accepted");
    }

    #[test]
    fn avx_state_requires_avx_instruction_support() {
        assert!(avx_state_has_instruction_support(X87 | SSE, 0));
        assert!(!avx_state_has_instruction_support(PERMITTED, 0));
        assert!(!avx_state_has_instruction_support(PERMITTED, 1 << 27));
        assert!(!avx_state_has_instruction_support(PERMITTED, 1 << 29));
        assert!(avx_state_has_instruction_support(PERMITTED, AVX_SUPPORTED));
    }

    #[test]
    fn every_permitted_component_must_be_available() {
        assert!(has_permitted_components(PERMITTED));
        assert!(has_permitted_components(PERMITTED | (1 << 19)));
        for component in [X87, SSE, AVX] {
            assert!(!has_permitted_components(PERMITTED & !component));
        }
    }

    #[test]
    fn the_live_configuration_is_exactly_the_fixed_policy() {
        assert!(matches_permitted_configuration(PERMITTED));
        for component in [X87, SSE, AVX] {
            assert!(!matches_permitted_configuration(PERMITTED & !component));
        }
        for extra in [3, 7, 19, 40, 63] {
            assert!(
                !matches_permitted_configuration(PERMITTED | (1 << extra)),
                "extra component {extra} was admitted"
            );
        }
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

    #[test]
    fn a_mask_bit_beyond_the_table_is_ignored() {
        let layout = Layout::compacted(PERMITTED | (1 << 40) | (1 << 63), avx_only);
        assert_eq!(layout.size(), HEADER_BYTES + 256);
        assert_eq!(layout.offset_of(40), None);
        assert_eq!(layout.offset_of(63), None);
        assert!(layout.is_consistent());
    }

    #[test]
    fn a_layout_that_overflows_a_u32_offset_is_not_consistent() {
        let huge = |_| Component::new(0x8000_0000, Alignment::Natural);
        let layout = Layout::compacted(0b1_1100, huge);

        assert!(
            !layout.is_consistent(),
            "offsets {:?} at size {}",
            layout.offset_of(2),
            layout.size()
        );

        // And no two enabled components share a starting offset.
        let placed: Vec<Option<usize>> = (2..5).map(|index| layout.offset_of(index)).collect();
        for (index, offset) in placed.iter().enumerate() {
            for earlier in &placed[..index] {
                assert!(
                    offset.is_none() || offset != earlier,
                    "two components at {offset:?}"
                );
            }
        }
    }

    /// A duplicate offset is what `is_consistent` is for, so it must see one.
    #[test]
    fn two_components_at_one_offset_are_not_consistent() {
        // Constructed directly, because the constructor now refuses to build
        // one — the invariant still has to be checkable from the outside.
        let mut offsets = [None; COMPONENTS];
        offsets[2] = Some(576);
        offsets[3] = Some(576);
        let layout = Layout {
            offsets,
            size: 1024,
        };
        assert!(!layout.is_consistent(), "an alias passed");
    }

    #[test]
    fn a_component_must_start_before_the_end_of_the_area() {
        let mut offsets = [None; COMPONENTS];
        offsets[2] = Some(HEADER_BYTES as u32);
        let layout = Layout {
            offsets,
            size: HEADER_BYTES,
        };

        assert!(!layout.is_consistent(), "an empty tail component passed");
    }
}
