mod limine_glue;

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

pub use limine_glue::enter;
pub use memory_map::{DirectMapCache, MemoryKind, MemoryMap, MemoryMapError, MemoryRegion};
use pixel::FramebufferLayout;
pub use pixel::{FramebufferLayoutError, PixelFormat, PixelFormatError, Rgb};

use crate::mmio::{MmioError, MmioRegion};

const WORD: usize = size_of::<u32>();

static FRAMEBUFFER_START: AtomicU64 = AtomicU64::new(0);
static FRAMEBUFFER_LEN: AtomicU64 = AtomicU64::new(0);

fn record_framebuffer(start: u64, len: u64) {
    FRAMEBUFFER_START.store(start, Ordering::Relaxed);
    FRAMEBUFFER_LEN.store(len, Ordering::Release);
}

pub(crate) fn framebuffer_extent() -> Option<(u64, u64)> {
    match FRAMEBUFFER_LEN.load(Ordering::Acquire) {
        0 => None,
        len => Some((FRAMEBUFFER_START.load(Ordering::Relaxed), len)),
    }
}

fn framebuffer_access<T>(result: Result<T, MmioError>) -> T {
    match result {
        Ok(value) => value,
        Err(reason) => panic!("validated framebuffer access left its mapping: {reason}"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FramebufferInfo {
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bytes_per_pixel: usize,
    pub format: PixelFormat,
}

// The shadow is the precondition for ever mapping the framebuffer write-combining.
#[derive(Debug)]
pub struct Framebuffer {
    region: MmioRegion,
    info: FramebufferInfo,
    layout: FramebufferLayout,
    back: alloc::vec::Vec<u32>,
}

impl Framebuffer {
    pub(crate) const fn new(
        region: MmioRegion,
        info: FramebufferInfo,
        layout: FramebufferLayout,
    ) -> Self {
        Self {
            region,
            info,
            layout,
            back: alloc::vec::Vec::new(),
        }
    }

    pub const fn info(&self) -> FramebufferInfo {
        self.info
    }

    #[must_use = "a refused framebuffer shadow was not attached"]
    pub fn attach_back_buffer(&mut self) -> bool {
        if !self.back.is_empty() {
            return true;
        }

        let words = self.words();
        if self.back.try_reserve_exact(words).is_err() {
            return false;
        }
        for word in 0..words {
            let value = framebuffer_access(self.region.read_u32(word * WORD));
            self.back.push(value);
        }
        true
    }

    fn words(&self) -> usize {
        self.layout.span() / WORD
    }

    fn shadow(&mut self, offset: usize, value: u32) {
        if self.back.is_empty() {
            return;
        }
        let word = self
            .back
            .get_mut(offset / WORD)
            .unwrap_or_else(|| panic!("framebuffer shadow is shorter than its validated mapping"));
        *word = value;
    }

    fn word_at(&self, offset: usize) -> u32 {
        if self.back.is_empty() {
            framebuffer_access(self.region.read_u32(offset))
        } else {
            *self.back.get(offset / WORD).unwrap_or_else(|| {
                panic!("framebuffer shadow is shorter than its validated mapping")
            })
        }
    }

    fn write_word(&mut self, offset: usize, value: u32) {
        framebuffer_access(self.region.write_u32(offset, value));
        self.shadow(offset, value);
    }

    pub fn write_pixel(&mut self, x: usize, y: usize, colour: Rgb) {
        let Some(offset) = self.layout.offset(x, y) else {
            return;
        };
        let word = self.info.format.pack(colour);
        self.write_word(offset, word);
    }

    pub fn fill(&mut self, colour: Rgb) {
        self.fill_rows(0, self.info.height, colour);
    }

    pub fn fill_rows(&mut self, y: usize, rows: usize, colour: Rgb) {
        let word = self.info.format.pack(colour);
        for row in y..y.saturating_add(rows).min(self.info.height) {
            let Some(start) = self.layout.offset(0, row) else {
                continue;
            };
            framebuffer_access(self.region.fill_u32(start, self.info.width, word));
            for column in 0..self.info.width {
                self.shadow(start + column * WORD, word);
            }
        }
    }

    pub fn copy_rows(&mut self, from_y: usize, to_y: usize, rows: usize) {
        let Some(plan) = self.layout.copy_rows_up(from_y, to_y, rows) else {
            panic!("copy_rows moves rows upward only");
        };
        let rows = plan.len();
        if !self.back.is_empty() && rows != 0 {
            for span in plan {
                let destination = span.destination_offset();
                let Some(values) = span.source(&self.back) else {
                    panic!("framebuffer shadow is shorter than its validated mapping");
                };
                framebuffer_access(self.region.write_u32_slice(destination, values));
                assert!(
                    span.copy_within(&mut self.back).is_some(),
                    "framebuffer shadow is shorter than its validated mapping"
                );
            }
            return;
        }

        let width = self.info.width;
        for row in 0..rows {
            let Some(from_y) = from_y.checked_add(row) else {
                break;
            };
            let Some(to_y) = to_y.checked_add(row) else {
                break;
            };
            for column in 0..width {
                let Some(from) = self.layout.offset(column, from_y) else {
                    continue;
                };
                let Some(to) = self.layout.offset(column, to_y) else {
                    continue;
                };
                let word = self.word_at(from);
                self.write_word(to, word);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum FramebufferError {
    NoResponse,
    NoFramebuffer,
    UnsupportedMemoryModel,
    UnsupportedBpp(u16),
    InvalidGeometry(FramebufferLayoutError),
    InvalidPixelFormat(PixelFormatError),
}

impl fmt::Display for FramebufferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoResponse => formatter.write_str("bootloader ignored the framebuffer request"),
            Self::NoFramebuffer => formatter.write_str("bootloader listed no framebuffers"),
            Self::UnsupportedMemoryModel => formatter.write_str("framebuffer is not direct RGB"),
            Self::UnsupportedBpp(bpp) => {
                write!(formatter, "unsupported depth: {bpp} bits per pixel")
            }
            Self::InvalidGeometry(reason) => {
                write!(formatter, "invalid framebuffer geometry: {reason}")
            }
            Self::InvalidPixelFormat(reason) => {
                write!(formatter, "invalid framebuffer format: {reason}")
            }
        }
    }
}

impl core::error::Error for FramebufferError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DirectMap {
    offset: u64,
}

impl DirectMap {
    pub(crate) const fn new(offset: u64) -> Self {
        Self { offset }
    }

    // Frame pointers are this plus an address below the ceiling: page-aligned, in the higher half.
    pub(crate) const fn from_handoff(offset: u64) -> Result<Self, HandoffError> {
        if offset < x86_paging::USER_END
            || !x86_paging::is_canonical(offset)
            || !offset.is_multiple_of(x86_paging::PAGE_SIZE)
            || offset.checked_add(crate::mm::CEILING).is_none()
        {
            return Err(HandoffError::DirectMapOffset(offset));
        }
        Ok(Self::new(offset))
    }

    pub const fn offset(self) -> u64 {
        self.offset
    }

    pub const fn virtual_address(self, physical: u64) -> Option<u64> {
        self.offset.checked_add(physical)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImageAddress {
    physical: u64,
    virtual_base: u64,
}

impl ImageAddress {
    pub(crate) const fn new(physical: u64, virtual_base: u64) -> Self {
        Self {
            physical,
            virtual_base,
        }
    }

    pub const fn to_physical(self, address: u64) -> Option<u64> {
        match address.checked_sub(self.virtual_base) {
            Some(offset) => self.physical.checked_add(offset),
            None => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Section {
    pub start: u64,
    pub end: u64,
}

impl Section {
    pub const fn mapped_len(self) -> u64 {
        let len = self.end.saturating_sub(self.start);
        len.next_multiple_of(x86_paging::PAGE_SIZE)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sections {
    pub text: Section,
    pub rodata: Section,
    pub data: Section,
    pub entry_enclave: EntryEnclaveSections,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EntryEnclaveSections {
    pub text: Section,
    pub rodata: Section,
    pub tables: Section,
    pub data: Section,
}

// Never make a symbol weak or `PROVIDE`d: a zero address would map the kernel text writable.
pub fn sections() -> Sections {
    // Storage-less symbols: only their addresses are taken, never their bytes.
    unsafe extern "C" {
        static __text_start: u8;
        static __text_end: u8;
        static __rodata_start: u8;
        static __rodata_end: u8;
        static __data_start: u8;
        static __data_end: u8;
        static __kpti_text_start: u8;
        static __kpti_text_end: u8;
        static __kpti_rodata_start: u8;
        static __kpti_rodata_end: u8;
        static __kpti_tables_start: u8;
        static __kpti_tables_end: u8;
        static __kpti_data_start: u8;
        static __kpti_data_end: u8;
    }

    // Distinct symbols may alias one boundary, so opacity stops their equality folding to false.
    fn linker_address(symbol: *const u8) -> u64 {
        core::hint::black_box(symbol).addr() as u64
    }

    Sections {
        text: Section {
            start: linker_address(&raw const __text_start),
            end: linker_address(&raw const __text_end),
        },
        rodata: Section {
            start: linker_address(&raw const __rodata_start),
            end: linker_address(&raw const __rodata_end),
        },
        data: Section {
            start: linker_address(&raw const __data_start),
            end: linker_address(&raw const __data_end),
        },
        entry_enclave: EntryEnclaveSections {
            text: Section {
                start: linker_address(&raw const __kpti_text_start),
                end: linker_address(&raw const __kpti_text_end),
            },
            rodata: Section {
                start: linker_address(&raw const __kpti_rodata_start),
                end: linker_address(&raw const __kpti_rodata_end),
            },
            tables: Section {
                start: linker_address(&raw const __kpti_tables_start),
                end: linker_address(&raw const __kpti_tables_end),
            },
            data: Section {
                start: linker_address(&raw const __kpti_data_start),
                end: linker_address(&raw const __kpti_data_end),
            },
        },
    }
}

// Minted once from the bootloader's answer; neither `Copy` nor `Clone`, so it is the only authority.
#[derive(Debug)]
pub struct BootMemory {
    map: MemoryMap,
    direct: DirectMap,
}

impl BootMemory {
    pub(crate) const fn new(map: MemoryMap, direct: DirectMap) -> Self {
        Self { map, direct }
    }

    pub const fn map(&self) -> &MemoryMap {
        &self.map
    }

    pub const fn direct(&self) -> DirectMap {
        self.direct
    }
}

#[derive(Debug)]
pub struct BootInfo {
    pub base_revision_supported: bool,
    pub framebuffer: Result<Framebuffer, FramebufferError>,
    pub memory: Result<BootMemory, HandoffError>,
    pub image: Result<ImageAddress, HandoffError>,
    // Virtual from base revision 4.
    pub rsdp: Result<u64, HandoffError>,
    pub module: Result<&'static [u8], HandoffError>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum HandoffError {
    NoResponse,
    Empty,
    MemoryMap(MemoryMapError),
    DirectMapOffset(u64),
}

impl fmt::Display for HandoffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoResponse => formatter.write_str("bootloader ignored the request"),
            Self::Empty => formatter.write_str("bootloader answered with nothing"),
            Self::MemoryMap(reason) => write!(formatter, "invalid memory map: {reason}"),
            Self::DirectMapOffset(offset) => write!(
                formatter,
                "direct-map offset {offset:#x} is not a page-aligned higher-half base"
            ),
        }
    }
}

impl core::error::Error for HandoffError {}

#[doc(hidden)]
pub mod __macro {
    pub use limine::BaseRevision;
    pub use limine::request::{
        ExecutableAddressRequest, FramebufferRequest, HhdmRequest, MemoryMapRequest, ModuleRequest,
        RequestsEndMarker, RequestsStartMarker, RsdpRequest,
    };

    pub use super::limine_glue::{Requests, base_revision};
}

// Expands in the caller: a linker may drop an rlib's `#[used]` statics (rust-lang/rust#47384).
#[macro_export]
macro_rules! entry {
    ($path:path) => {
        const __WOST_ENTRY_SIGNATURE: fn($crate::boot::BootInfo, $crate::sync::ThreadContext) -> ! =
            $path;

        // `allow`, since `expect` never fires in a macro; the caller must `deny`, not `forbid`.
        #[used]
        #[allow(unsafe_code, reason = "wost code expanding under the caller's deny")]
        #[unsafe(link_section = ".requests_start_marker")]
        static __WOST_REQUESTS_START: $crate::boot::__macro::RequestsStartMarker =
            $crate::boot::__macro::RequestsStartMarker::new();

        #[used]
        #[allow(unsafe_code, reason = "wost code expanding under the caller's deny")]
        #[unsafe(link_section = ".requests")]
        static __WOST_BASE_REVISION: $crate::boot::__macro::BaseRevision =
            $crate::boot::__macro::base_revision();

        #[used]
        #[allow(unsafe_code, reason = "wost code expanding under the caller's deny")]
        #[unsafe(link_section = ".requests")]
        static __WOST_FRAMEBUFFER: $crate::boot::__macro::FramebufferRequest =
            $crate::boot::__macro::FramebufferRequest::new();

        #[used]
        #[allow(unsafe_code, reason = "wost code expanding under the caller's deny")]
        #[unsafe(link_section = ".requests")]
        static __WOST_MEMORY_MAP: $crate::boot::__macro::MemoryMapRequest =
            $crate::boot::__macro::MemoryMapRequest::new();

        #[used]
        #[allow(unsafe_code, reason = "wost code expanding under the caller's deny")]
        #[unsafe(link_section = ".requests")]
        static __WOST_DIRECT_MAP: $crate::boot::__macro::HhdmRequest =
            $crate::boot::__macro::HhdmRequest::new();

        #[used]
        #[allow(unsafe_code, reason = "wost code expanding under the caller's deny")]
        #[unsafe(link_section = ".requests")]
        static __WOST_IMAGE: $crate::boot::__macro::ExecutableAddressRequest =
            $crate::boot::__macro::ExecutableAddressRequest::new();

        #[used]
        #[allow(unsafe_code, reason = "wost code expanding under the caller's deny")]
        #[unsafe(link_section = ".requests")]
        static __WOST_RSDP: $crate::boot::__macro::RsdpRequest =
            $crate::boot::__macro::RsdpRequest::new();

        #[used]
        #[allow(unsafe_code, reason = "wost code expanding under the caller's deny")]
        #[unsafe(link_section = ".requests")]
        static __WOST_MODULES: $crate::boot::__macro::ModuleRequest =
            $crate::boot::__macro::ModuleRequest::new();

        #[used]
        #[allow(unsafe_code, reason = "wost code expanding under the caller's deny")]
        #[unsafe(link_section = ".requests_end_marker")]
        static __WOST_REQUESTS_END: $crate::boot::__macro::RequestsEndMarker =
            $crate::boot::__macro::RequestsEndMarker::new();

        #[allow(unsafe_code, reason = "wost code expanding under the caller's deny")]
        #[unsafe(no_mangle)]
        extern "C" fn kmain() -> ! {
            $crate::boot::enter(
                $crate::boot::__macro::Requests {
                    base_revision: &__WOST_BASE_REVISION,
                    framebuffer: &__WOST_FRAMEBUFFER,
                    memory_map: &__WOST_MEMORY_MAP,
                    direct_map: &__WOST_DIRECT_MAP,
                    image: &__WOST_IMAGE,
                    rsdp: &__WOST_RSDP,
                    modules: &__WOST_MODULES,
                },
                __WOST_ENTRY_SIGNATURE,
            )
        }
    };
}
