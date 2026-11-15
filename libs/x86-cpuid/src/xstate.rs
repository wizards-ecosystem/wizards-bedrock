// Standard offsets are unordered (APX, component 19, sits at 960): never sort by offset.

pub const HEADER_BYTES: usize = 576;

const X87: u64 = 1 << 0;

const SSE: u64 = 1 << 1;

const AVX: u64 = 1 << 2;

const AVX_SUPPORTED: u32 = 1 << 28;

const PERMITTED: u64 = X87 | SSE | AVX;

const COMPONENTS: usize = 20;

// `CPUID.0DH` does not imply the leaf 1 AVX instruction support, so AVX state needs both.
pub const fn enabled_components(available: u64, basic_ecx: u32) -> Option<u64> {
    let mut mask = available & PERMITTED;
    if basic_ecx & AVX_SUPPORTED == 0 {
        mask &= !AVX;
    }
    if mask & (X87 | SSE) == X87 | SSE {
        Some(mask)
    } else {
        None
    }
}

pub const fn includes_avx(mask: u64) -> bool {
    mask & AVX != 0
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Component {
    size: u32,
    alignment: Alignment,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alignment {
    Natural,
    Bytes64,
}

impl Component {
    // `Alignment::Bytes64` is `CPUID.0DH.ECX` bit 1.
    pub const fn new(size: u32, alignment: Alignment) -> Self {
        Self { size, alignment }
    }

    pub const fn size(self) -> u32 {
        self.size
    }

    const fn is_aligned(self) -> bool {
        matches!(self.alignment, Alignment::Bytes64)
    }

    pub const fn is_present(self) -> bool {
        self.size != 0
    }
}

pub const fn standard_component_extent(component: Component, offset: u32) -> Option<usize> {
    if !component.is_present() || offset < HEADER_BYTES as u32 {
        return None;
    }
    match offset.checked_add(component.size()) {
        Some(end) => Some(end as usize),
        None => None,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    offsets: [Option<u32>; COMPONENTS],
    size: usize,
}

impl Layout {
    // `info` is asked only about enabled indices from two, so no unadvertised sub-leaf is read.
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

    // Zero when inconsistent; the caller checks `is_consistent` first.
    pub const fn size(&self) -> usize {
        self.size
    }

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
