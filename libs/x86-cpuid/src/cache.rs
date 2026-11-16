pub const CACHE_LEAF: u32 = 4;

const MAX_COLOURS: u64 = 64;

const PAGE_BYTES: u64 = 4096;

const TYPE_MASK: u32 = 0x1f;

const LINE_MASK: u32 = 0xfff;

mod kind {
    pub(super) const DATA: u32 = 1;
    pub(super) const UNIFIED: u32 = 3;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cache {
    level: u32,
    holds_data: bool,
    way_bytes: u64,
}

impl Cache {
    // `None` is type zero, which ends the sub-leaf walk.
    pub fn from_leaf(eax: u32, ebx: u32, ecx: u32) -> Option<Self> {
        let kind = eax & TYPE_MASK;
        if kind == 0 {
            return None;
        }
        let line = u64::from(ebx & LINE_MASK) + 1;
        let sets = u64::from(ecx) + 1;
        Some(Self {
            level: (eax >> 5) & 0x7,
            holds_data: kind == kind::DATA || kind == kind::UNIFIED,
            way_bytes: line * sets,
        })
    }

    const fn page_colours(self) -> u64 {
        self.way_bytes / PAGE_BYTES
    }
}

// A power of two up to 64 and bounded by the unsliced L2, since a sliced LLC's sets over-count.
pub fn colour_count(caches: impl IntoIterator<Item = Cache>) -> u32 {
    let mut second: Option<u64> = None;
    let mut last: Option<(u32, u64)> = None;
    for cache in caches.into_iter().filter(|cache| cache.holds_data) {
        let colours = cache.page_colours();
        if cache.level == 2 {
            second = Some(second.map_or(colours, |known| known.min(colours)));
        }
        last = match last {
            Some((level, known)) if level == cache.level => Some((level, known.min(colours))),
            Some((level, _)) if level > cache.level => last,
            _ => Some((cache.level, colours)),
        };
    }
    let Some(second) = second else {
        return 1;
    };
    let bounded = last.map_or(second, |(_, colours)| second.min(colours));
    match bounded.min(MAX_COLOURS) {
        0 => 1,
        colours => 1 << colours.ilog2(),
    }
}
