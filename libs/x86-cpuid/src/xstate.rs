//! Where each piece of extended processor state sits in a compacted save area.
//!
//! Standard offsets are unordered (APX, component 19, sits at 960).
//!
//! Nothing may sort by offset or assume index order implies address order.
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

/// Legacy x87/SSE save area plus the XSAVE header, 512 + 64 bytes, holding components 0 and 1.
pub const HEADER_BYTES: usize = 576;

/// State component 0: x87.
pub const X87: u64 = 1 << 0;

/// State component 1: SSE.
pub const SSE: u64 = 1 << 1;

/// State component 2: AVX's upper halves.
pub const AVX: u64 = 1 << 2;

const AVX_SUPPORTED: u32 = 1 << 28;

/// What `XCR0` may hold; every enabled bit is per-thread memory for the life of the kernel.
pub const PERMITTED: u64 = X87 | SSE | AVX;

/// The state components this crate lays out: twenty, because APX is component 19.
pub const COMPONENTS: usize = 20;

/// Whether the processor supplies the complete fixed state policy.
pub const fn has_permitted_components(available: u64) -> bool {
    available & PERMITTED == PERMITTED
}

/// Whether the live configured mask is exactly [`PERMITTED`], which save-area sizing covers.
pub const fn matches_permitted_configuration(configured: u64) -> bool {
    configured == PERMITTED
}

/// Whether AVX state in `mask` has the leaf 1 AVX instruction support `CPUID.0DH` does not imply.
///
/// Checked before `XCR0.AVX` is enabled.
pub const fn avx_state_has_instruction_support(mask: u64, basic_ecx: u32) -> bool {
    mask & AVX == 0 || basic_ecx & AVX_SUPPORTED != 0
}

/// What the processor says about one state component.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Component {
    size: u32,
    alignment: Alignment,
}

/// Placement required by a state component's CPUID sub-leaf.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

    /// A component of `size` bytes; [`Alignment::Bytes64`] is `CPUID.0DH.ECX` bit 1.
    pub const fn new(size: u32, alignment: Alignment) -> Self {
        Self { size, alignment }
    }

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

/// End of one component at its standard `CPUID.0DH` offset, if present and past the header.
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
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    offsets: [Option<u32>; COMPONENTS],
    size: usize,
}

impl Layout {
    /// Lay out the components `mask` enables, asking `info` only about enabled indices from two.
    ///
    /// An `info` wired to `CPUID.0DH` thus never queries a sub-leaf the CPU did not advertise.
    ///
    /// Bits from [`COMPONENTS`] up are ignored, not rejected.
    ///
    /// AVX without SSE, or an enabled component with an absent sub-leaf, is an inconsistent layout.
    pub fn compacted(mask: u64, info: impl Fn(u32) -> Component) -> Self {
        let mut offsets = [None; COMPONENTS];
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

    /// The save-area size, zero when inconsistent; check [`Self::is_consistent`] first.
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

    /// Whether every enabled component has its own offset inside the area.
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
