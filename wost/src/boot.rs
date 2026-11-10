//! Bootloader handoff: [`entry!`](crate::entry) and a [`BootInfo`] whose types name no bootloader.

mod limine_glue;
#[cfg(feature = "selftest")]
mod selftest;

use core::fmt;

pub use limine_glue::enter;
pub use memory_map::{DirectMapCache, MemoryKind, MemoryMap, MemoryMapError, MemoryRegion};
use pixel::FramebufferLayout;
pub use pixel::{FramebufferLayoutError, PixelFormat, PixelFormatError, Rgb};

use crate::mmio::{MmioError, MmioRegion};

/// Bytes in one framebuffer word, the only access width the region supports.
const WORD: usize = size_of::<u32>();

fn framebuffer_access<T>(result: Result<T, MmioError>) -> T {
    match result {
        Ok(value) => value,
        Err(reason) => panic!("validated framebuffer access left its mapping: {reason}"),
    }
}

/// Geometry and pixel layout of a linear framebuffer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FramebufferInfo {
    /// Visible width in pixels.
    pub width: usize,
    /// Visible height in pixels.
    pub height: usize,
    /// Bytes between the starts of consecutive rows, not always `width * bytes_per_pixel`.
    pub pitch: usize,
    pub bytes_per_pixel: usize,
    pub format: PixelFormat,
}

/// A linear framebuffer the bootloader mapped; with a back buffer, reads come from RAM.
///
/// Framebuffer memory is uncached, so the shadow keeps scrolling off device reads.
/// A refused MMIO access is geometry corruption, stopped before shadow and device diverge.
#[derive(Debug)]
pub struct Framebuffer {
    region: MmioRegion,
    info: FramebufferInfo,
    layout: FramebufferLayout,
    /// One `u32` per pixel at the device's stride, empty until a back buffer is attached.
    back: alloc::vec::Vec<u32>,
}

impl Framebuffer {
    /// Wrap a mapping the caller has vouched for, with no shadow yet.
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

    /// Give the framebuffer a shadow in RAM once the heap exists; idempotent.
    ///
    /// `false` means the allocation failed and the framebuffer keeps working unshadowed.
    /// The shadow starts as a copy of the screen, so lines already printed are kept.
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
            let value = framebuffer_access(MmioRegion::read_u32(&self.region, word * WORD));
            self.back.push(value);
        }
        true
    }

    /// Words in the shadow: the device's stride times its visible height.
    fn words(&self) -> usize {
        self.layout.span() / WORD
    }

    /// Record a word in the shadow, if there is one.
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

    /// The word at `offset`, from the shadow if there is one and from the device otherwise.
    fn word_at(&self, offset: usize) -> u32 {
        if self.back.is_empty() {
            framebuffer_access(MmioRegion::read_u32(&self.region, offset))
        } else {
            *self.back.get(offset / WORD).unwrap_or_else(|| {
                panic!("framebuffer shadow is shorter than its validated mapping")
            })
        }
    }

    fn write_word(&mut self, offset: usize, value: u32) {
        framebuffer_access(MmioRegion::write_u32(&mut self.region, offset, value));
        self.shadow(offset, value);
    }

    /// Set the pixel at (`x`, `y`), clipped to the visible area since edge glyphs run off it.
    ///
    /// Whatever survives the clip is still bounds-checked by the underlying region.
    pub fn write_pixel(&mut self, x: usize, y: usize, colour: Rgb) {
        let Some(offset) = self.layout.offset(x, y) else {
            return;
        };
        let word = self.info.format.pack(colour);
        self.write_word(offset, word);
    }

    /// Paint every visible pixel one colour.
    pub fn fill(&mut self, colour: Rgb) {
        self.fill_rows(0, self.info.height, colour);
    }

    /// Paint `rows` of pixels starting at row `y` one colour.
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

    /// Copy `rows` of pixels from `from_y` up to `to_y` as raw words, lossless for narrow channels.
    ///
    /// The caller clears the rows exposed at the bottom.
    ///
    /// # Panics
    ///
    /// If `to_y > from_y`: an overlapping copy downward would read rows it already overwrote.
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
                if span.copy_within(&mut self.back).is_none() {
                    panic!("framebuffer shadow is shorter than its validated mapping");
                }
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
                // From the shadow where there is one, since scrolling must not read device memory.
                let word = self.word_at(from);
                self.write_word(to, word);
            }
        }
    }
}

/// Why the kernel has no usable framebuffer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum FramebufferError {
    /// The bootloader ignored the framebuffer request.
    NoResponse,
    /// The bootloader answered, but listed no framebuffers.
    NoFramebuffer,
    /// The framebuffer is not a direct RGB mapping.
    UnsupportedMemoryModel,
    /// Only 32 bits per pixel is implemented.
    UnsupportedBpp(u16),
    /// The reported geometry cannot bound the implemented pixel accesses.
    InvalidGeometry(FramebufferLayoutError),
    /// The reported colour channels overlap or leave the pixel word.
    InvalidPixelFormat(PixelFormatError),
}

impl fmt::Display for FramebufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoResponse => f.write_str("bootloader ignored the framebuffer request"),
            Self::NoFramebuffer => f.write_str("bootloader listed no framebuffers"),
            Self::UnsupportedMemoryModel => f.write_str("framebuffer is not direct RGB"),
            Self::UnsupportedBpp(bpp) => write!(f, "unsupported depth: {bpp} bits per pixel"),
            Self::InvalidGeometry(reason) => write!(f, "invalid framebuffer geometry: {reason}"),
            Self::InvalidPixelFormat(reason) => write!(f, "invalid framebuffer format: {reason}"),
        }
    }
}

impl core::error::Error for FramebufferError {}

/// The bootloader's higher-half direct map: each listed address reads at `physical + offset`.
///
/// It is readable before the kernel has page tables of its own, so the first table is walkable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DirectMap {
    offset: u64,
}

impl DirectMap {
    pub(crate) const fn new(offset: u64) -> Self {
        Self { offset }
    }

    pub const fn offset(self) -> u64 {
        self.offset
    }

    /// Where `physical` is readable, or `None` if the sum does not fit.
    pub const fn virtual_address(self, physical: u64) -> Option<u64> {
        self.offset.checked_add(physical)
    }
}

/// Where the bootloader placed the kernel image, which turns a linked address into its frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImageAddress {
    physical: u64,
    virtual_: u64,
}

impl ImageAddress {
    pub(crate) const fn new(physical: u64, virtual_: u64) -> Self {
        Self { physical, virtual_ }
    }

    /// The frame behind a virtual address inside the image, or `None` below its base.
    pub const fn to_physical(self, address: u64) -> Option<u64> {
        match address.checked_sub(self.virtual_) {
            Some(offset) => self.physical.checked_add(offset),
            None => None,
        }
    }
}

/// One protection class of the loaded kernel image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Section {
    /// First virtual address, page-aligned by the linker script.
    pub start: u64,
    /// One past the last virtual address, not necessarily aligned.
    pub end: u64,
}

impl Section {
    /// Bytes from `start` to the end of `end`'s page, since a mapping covers whole pages.
    pub const fn mapped_len(self) -> u64 {
        let len = self.end.saturating_sub(self.start);
        len.next_multiple_of(4096)
    }
}

/// The kernel image, divided by what each part may do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sections {
    /// Executable code, mapped read-only and executable.
    pub text: Section,
    /// Constants, mapped read-only and never executable.
    pub rodata: Section,
    /// `.requests`, `.data` and `.bss` in one linker segment, mapped writable, never executable.
    pub data: Section,
    /// The page-exact entry ranges, outside the ordinary image classes.
    pub entry_enclave: EntryEnclaveSections,
}

/// Linker-owned ranges from which the x86 entry enclave is validated.
///
/// The guards are in no mapping list; their named addresses let the page-table owner prove absence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EntryEnclaveSections {
    /// Interrupt stubs and common entry/exit assembly.
    pub text: Section,
    /// Immutable public entry geometry.
    pub rodata: Section,
    /// The IDT, GDT and TSS bytes hardware may read during entry.
    pub tables: Section,
    /// CPU-local root routing and public entry state.
    pub route: Section,
    /// The complete `.kpti.data` output extent, including its named guards.
    pub data: Section,
    /// The CPU-local privilege-transition stack.
    pub transition_stack: Section,
    /// The transition stack's lower guard, absent from constructed roots.
    pub transition_guard: Section,
    /// The entry-reachable interrupt-stack-table stack.
    pub ist_stack: Section,
    /// The interrupt stack's lower guard, absent from constructed roots.
    pub ist_guard: Section,
}

/// Where the linker put each protection class of this kernel.
///
/// The linker script must define every symbol read here, so a missing one fails to link.
pub fn sections() -> Sections {
    // Storage-less symbols: only addresses are taken, so `u8` is harmless and `&raw const` safe.
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
        static __kpti_route_start: u8;
        static __kpti_route_end: u8;
        static __kpti_data_end: u8;
        static __kpti_transition_guard_start: u8;
        static __kpti_transition_guard_end: u8;
        static __kpti_transition_stack_start: u8;
        static __kpti_transition_stack_end: u8;
        static __kpti_ist_guard_start: u8;
        static __kpti_ist_guard_end: u8;
        static __kpti_ist_stack_start: u8;
        static __kpti_ist_stack_end: u8;
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
            route: Section {
                start: linker_address(&raw const __kpti_route_start),
                end: linker_address(&raw const __kpti_route_end),
            },
            data: Section {
                start: linker_address(&raw const __kpti_route_start),
                end: linker_address(&raw const __kpti_data_end),
            },
            transition_stack: Section {
                start: linker_address(&raw const __kpti_transition_stack_start),
                end: linker_address(&raw const __kpti_transition_stack_end),
            },
            transition_guard: Section {
                start: linker_address(&raw const __kpti_transition_guard_start),
                end: linker_address(&raw const __kpti_transition_guard_end),
            },
            ist_stack: Section {
                start: linker_address(&raw const __kpti_ist_stack_start),
                end: linker_address(&raw const __kpti_ist_stack_end),
            },
            ist_guard: Section {
                start: linker_address(&raw const __kpti_ist_guard_start),
                end: linker_address(&raw const __kpti_ist_guard_end),
            },
        },
    }
}

/// Everything the bootloader handed the kernel.
#[derive(Debug)]
pub struct BootInfo {
    /// Whether the bootloader honoured the base revision the kernel asked for.
    pub base_revision_supported: bool,
    /// The framebuffer, or why there is no usable one.
    pub framebuffer: Result<Framebuffer, FramebufferError>,
    /// The physical memory map, or why there is none.
    pub memory_map: Result<MemoryMap, HandoffError>,
    /// The direct map of physical memory, or why there is none.
    pub direct_map: Result<DirectMap, HandoffError>,
    /// Where the bootloader placed the kernel image, or why it did not say.
    pub image: Result<ImageAddress, HandoffError>,
    /// The ACPI root pointer's address, virtual from base revision 4, or why there is none.
    pub rsdp: Result<u64, HandoffError>,
    /// The first module the bootloader loaded, this kernel's one-program initrd.
    pub module: Result<&'static [u8], HandoffError>,
}

/// Why part of the boot handoff is missing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum HandoffError {
    /// The bootloader ignored the request.
    NoResponse,
    /// The bootloader answered, but with nothing in it.
    Empty,
    /// The memory map could not be copied completely into early storage.
    MemoryMap(MemoryMapError),
}

impl fmt::Display for HandoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoResponse => f.write_str("bootloader ignored the request"),
            Self::Empty => f.write_str("bootloader answered with nothing"),
            Self::MemoryMap(reason) => write!(f, "invalid memory map: {reason}"),
        }
    }
}

impl core::error::Error for HandoffError {}

/// Unstable items [`entry!`](crate::entry) names, so no crate depends on `limine` directly.
#[doc(hidden)]
pub mod __macro {
    pub use limine::BaseRevision;
    pub use limine::request::{
        ExecutableAddressRequest, FramebufferRequest, HhdmRequest, MemoryMapRequest, ModuleRequest,
        RequestsEndMarker, RequestsStartMarker, RsdpRequest,
    };

    pub use super::limine_glue::{Requests, base_revision};
}

/// Define the request block and the `kmain` symbol the linker script's `ENTRY()` names.
///
/// `$path` must be `fn(BootInfo) -> !`.
/// It expands in the caller: a linker may drop an rlib's `#[used]` statics (rust-lang/rust#47384).
/// The caller must `deny`, never `forbid`, `unsafe_code`, so these `allow`s cover the expansion.
/// They stay `allow` because `expect` never fires inside a macro expansion and would look checked.
#[macro_export]
macro_rules! entry {
    ($path:path) => {
        const __WOST_ENTRY_SIGNATURE: fn($crate::boot::BootInfo) -> ! = $path;

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
