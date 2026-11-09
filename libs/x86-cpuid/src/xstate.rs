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
mod tests;
