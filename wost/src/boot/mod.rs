//! Bootloader handoff.
//!
//! [`entry!`](crate::entry) defines the kernel's entry point and the bootloader
//! request block. Everything the bootloader tells us arrives as [`BootInfo`],
//! whose types name no bootloader at all — swapping Limine out is a change to
//! `limine_glue.rs` and nothing else.

mod limine_glue;

use core::fmt;

pub use limine_glue::enter;
pub use memory_map::{DirectMapCache, MemoryKind, MemoryMap, MemoryMapError, MemoryRegion};
use pixel::FramebufferLayout;
pub use pixel::{FramebufferLayoutError, PixelFormat, PixelFormatError, Rgb};

use crate::mmio::{MmioError, MmioRegion};

/// Bytes in one framebuffer word — the only access width the region supports.
const WORD: usize = size_of::<u32>();

fn framebuffer_access<T>(result: Result<T, MmioError>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("validated framebuffer access left its mapping: {error}"),
    }
}

/// Geometry and pixel layout of a linear framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramebufferInfo {
    /// Visible width in pixels.
    pub width: usize,
    /// Visible height in pixels.
    pub height: usize,
    /// Distance between the starts of consecutive rows, in bytes. Not always
    /// `width * bytes_per_pixel`.
    pub pitch: usize,
    /// Bytes occupied by one pixel.
    pub bytes_per_pixel: usize,
    /// How colour channels sit inside a pixel.
    pub format: PixelFormat,
}

/// A linear framebuffer the bootloader has already mapped for us.
///
/// Once [`attach_back_buffer`](Self::attach_back_buffer) has been called, every
/// write goes to both the device and a shadow copy in RAM, and every *read*
/// comes from the shadow. That is what retires the scroll read-back
/// `001-boot.md` recorded as a limitation: framebuffer memory is uncached, so
/// reading a screen's worth of it to move rows up costs far more than the same
/// bytes in ordinary RAM — and it is the precondition for ever marking the
/// mapping write-combining, where reads are worse still (D-48).
/// A checked MMIO refusal after construction is internal geometry corruption
/// and fails stopped before the shadow can diverge from the device (D-249).
#[derive(Debug)]
pub struct Framebuffer {
    region: MmioRegion,
    info: FramebufferInfo,
    layout: FramebufferLayout,
    /// One `u32` per pixel of the visible area, at the device's own stride, or
    /// empty before a back buffer is attached.
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

    /// Geometry and pixel layout.
    pub const fn info(&self) -> FramebufferInfo {
        self.info
    }

    /// Give the framebuffer a shadow copy in RAM.
    ///
    /// Separate from construction because the framebuffer is built during the
    /// boot handoff, long before there is a heap to build a shadow in. Called
    /// once the heap exists; idempotent, and `false` if the allocation failed,
    /// in which case the framebuffer keeps working exactly as it did before.
    ///
    /// The shadow starts as a copy of what is on screen, so attaching it
    /// mid-boot does not lose the lines already printed.
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

    /// Where the pixels are reachable.
    #[cfg(feature = "selftest")]
    pub fn address(&self) -> u64 {
        self.region.base_address() as u64
    }

    /// Bytes the mapping covers, which every pixel offset must land inside.
    #[cfg(feature = "selftest")]
    pub fn mapped_bytes(&self) -> usize {
        self.layout.span()
    }

    /// Whether writes are being shadowed.
    #[cfg(feature = "selftest")]
    pub fn has_back_buffer(&self) -> bool {
        !self.back.is_empty()
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

    /// The word at `offset`, from the shadow if there is one and from the
    /// device otherwise.
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

    /// The pixel at (`x`, `y`), or `None` outside the visible area.
    ///
    /// Read from the shadow to avoid an uncached device read and make stray
    /// cursor marks observable (D-137).
    #[cfg(feature = "selftest")]
    pub fn pixel_at(&self, x: usize, y: usize) -> Option<u32> {
        Some(self.word_at(self.layout.offset(x, y)?))
    }

    /// Set the pixel at (`x`, `y`), ignoring coordinates outside the visible
    /// area.
    ///
    /// Clipping is deliberate: a glyph drawn near the right or bottom edge
    /// legitimately runs off it. Anything that survives the clip is still
    /// bounds-checked by the underlying region.
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

    /// Copy `rows` of pixels from `from_y` up to `to_y`.
    ///
    /// Pixels move as raw words rather than through [`Rgb`], so nothing is lost
    /// on a display whose channels are narrower than eight bits. Rows exposed
    /// at the bottom are left for the caller to clear.
    ///
    /// # Panics
    ///
    /// Panics if `to_y > from_y`. Source and destination may overlap, but only
    /// upward: copying down would read rows this call had already overwritten.
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
                // From the shadow where there is one. Reading device memory to
                // scroll is what this exists to stop.
                let word = self.word_at(from);
                self.write_word(to, word);
            }
        }
    }
}

/// Why the kernel has no usable framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            Self::NoResponse => {
                core::fmt::Write::write_str(f, "bootloader ignored the framebuffer request")
            }
            Self::NoFramebuffer => {
                core::fmt::Write::write_str(f, "bootloader listed no framebuffers")
            }
            Self::UnsupportedMemoryModel => {
                core::fmt::Write::write_str(f, "framebuffer is not direct RGB")
            }
            Self::UnsupportedBpp(bpp) => write!(f, "unsupported depth: {bpp} bits per pixel"),
            Self::InvalidGeometry(reason) => write!(f, "invalid framebuffer geometry: {reason}"),
            Self::InvalidPixelFormat(reason) => write!(f, "invalid framebuffer format: {reason}"),
        }
    }
}

impl core::error::Error for FramebufferError {}

/// The bootloader's direct map of physical memory into the higher half.
///
/// Every physical address the memory map lists is readable at
/// `physical + offset` before the kernel has page tables of its own — which is
/// what makes the first page table walkable and the first frame writable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectMap {
    offset: u64,
}

impl DirectMap {
    /// Wrap the offset the bootloader reported.
    pub(crate) const fn new(offset: u64) -> Self {
        Self { offset }
    }

    /// The offset itself.
    pub const fn offset(self) -> u64 {
        self.offset
    }

    /// Where `physical` is readable, or `None` if the sum does not fit.
    pub const fn virtual_address(self, physical: u64) -> Option<u64> {
        self.offset.checked_add(physical)
    }
}

/// Where the bootloader placed the kernel image.
///
/// The pair is what lets the kernel map its own sections: the linker gives
/// their virtual addresses, and the difference between these two turns any of
/// them into the physical frame behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageAddress {
    physical: u64,
    virtual_: u64,
}

impl ImageAddress {
    /// Record the pair the bootloader reported.
    pub(crate) const fn new(physical: u64, virtual_: u64) -> Self {
        Self { physical, virtual_ }
    }

    /// The frame behind a virtual address inside the image, or `None` if the
    /// address is below its base.
    pub const fn to_physical(self, address: u64) -> Option<u64> {
        match address.checked_sub(self.virtual_) {
            Some(offset) => self.physical.checked_add(offset),
            None => None,
        }
    }
}

/// One protection class of the loaded kernel image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    /// First virtual address, page-aligned by the linker script.
    pub start: u64,
    /// One past the last virtual address, not necessarily aligned.
    pub end: u64,
}

impl Section {
    /// Bytes from `start` to the end of the page `end` falls in.
    ///
    /// Rounded up because a mapping covers whole pages: the tail of the last
    /// page belongs to this section whether or not anything is in it.
    pub const fn mapped_len(self) -> u64 {
        let len = self.end.saturating_sub(self.start);
        len.next_multiple_of(4096)
    }
}

/// The kernel image, divided by what each part may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sections {
    /// Executable code. Mapped read-only and executable.
    pub text: Section,
    /// Constants. Mapped read-only and never executable.
    pub rodata: Section,
    /// Everything writable — `.requests`, `.data` and `.bss`, which the linker
    /// script keeps contiguous in one segment. Mapped writable and never
    /// executable.
    pub data: Section,
}

/// Where the linker put each protection class of this kernel.
///
/// `entry!` already ties `wost` to the linker script through `ENTRY(kmain)`
/// and the `.requests` section; the six symbols read here are the rest of that
/// contract. A script that does not define them fails to link, which is the
/// failure mode to want — the alternative is a kernel that maps its own text
/// writable because a symbol silently resolved to zero.
pub fn sections() -> Sections {
    // Linker symbols with no storage: only their addresses are taken, which
    // is what makes `u8` harmless and `&raw const` safe where a read is not.
    unsafe extern "C" {
        static __text_start: u8;
        static __text_end: u8;
        static __rodata_start: u8;
        static __rodata_end: u8;
        static __data_start: u8;
        static __data_end: u8;
    }

    Sections {
        text: Section {
            start: &raw const __text_start as u64,
            end: &raw const __text_end as u64,
        },
        rodata: Section {
            start: &raw const __rodata_start as u64,
            end: &raw const __rodata_end as u64,
        },
        data: Section {
            start: &raw const __data_start as u64,
            end: &raw const __data_end as u64,
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
    /// The address of the ACPI root pointer, or why there is none.
    ///
    /// Virtual from base revision 4, which is one of the reasons D-42 asked
    /// for it: the table tree is readable before the kernel maps anything.
    pub rsdp: Result<u64, HandoffError>,
    /// The first module the bootloader loaded — this kernel's initrd, which is
    /// one program (D-100).
    pub module: Result<&'static [u8], HandoffError>,
}

/// Why part of the boot handoff is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            Self::NoResponse => core::fmt::Write::write_str(f, "bootloader ignored the request"),
            Self::Empty => core::fmt::Write::write_str(f, "bootloader answered with nothing"),
            Self::MemoryMap(reason) => write!(f, "invalid memory map: {reason}"),
        }
    }
}

impl core::error::Error for HandoffError {}

/// Items [`entry!`](crate::entry) names in the calling crate.
///
/// Not a stable interface; re-exported only so downstream crates never depend
/// on the bootloader protocol crate directly.
#[doc(hidden)]
pub mod __macro {
    pub use limine::BaseRevision;
    pub use limine::request::{
        ExecutableAddressRequest, FramebufferRequest, HhdmRequest, MemoryMapRequest, ModuleRequest,
        RequestsEndMarker, RequestsStartMarker, RsdpRequest,
    };

    pub use super::limine_glue::{Requests, base_revision};
}

/// Define the kernel entry point.
///
/// Expands to the bootloader request block and the `kmain` symbol named by
/// `ENTRY()` in the linker script, then calls `$path`, which must be
/// `fn(BootInfo) -> !`.
///
/// The expansion lands in the calling crate rather than in `wost` on purpose:
/// `#[used]` statics inside an rlib can be dropped wholesale by the linker,
/// because nothing references the archive member holding them
/// (rust-lang/rust#47384). Every `-rt` crate in the ecosystem does it this way.
/// The `#[allow(unsafe_code)]` attributes are what let the calling crate carry
/// `#![deny(unsafe_code)]`; they cover `wost`-authored code that merely expands
/// there, which is why that crate uses `deny` rather than `forbid`.
///
/// **`allow`, not `expect`:** `unfulfilled_lint_expectations` does not fire for
/// attributes inside a macro expansion, so `expect` would falsely appear
/// checked (D-131).
#[macro_export]
macro_rules! entry {
    ($path:path) => {
        const __WOST_ENTRY_SIGNATURE: fn($crate::boot::BootInfo) -> ! = $path;

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests_start_marker")]
        static __WOST_REQUESTS_START: $crate::boot::__macro::RequestsStartMarker =
            $crate::boot::__macro::RequestsStartMarker::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_BASE_REVISION: $crate::boot::__macro::BaseRevision =
            $crate::boot::__macro::base_revision();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_FRAMEBUFFER: $crate::boot::__macro::FramebufferRequest =
            $crate::boot::__macro::FramebufferRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_MEMORY_MAP: $crate::boot::__macro::MemoryMapRequest =
            $crate::boot::__macro::MemoryMapRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_DIRECT_MAP: $crate::boot::__macro::HhdmRequest =
            $crate::boot::__macro::HhdmRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_IMAGE: $crate::boot::__macro::ExecutableAddressRequest =
            $crate::boot::__macro::ExecutableAddressRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_RSDP: $crate::boot::__macro::RsdpRequest =
            $crate::boot::__macro::RsdpRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_MODULES: $crate::boot::__macro::ModuleRequest =
            $crate::boot::__macro::ModuleRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests_end_marker")]
        static __WOST_REQUESTS_END: $crate::boot::__macro::RequestsEndMarker =
            $crate::boot::__macro::RequestsEndMarker::new();

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        extern "C" fn kmain() -> ! {
            $crate::boot::enter(
                $crate::boot::__macro::Requests {
                    base_revision: &__WOST_BASE_REVISION,
                    framebuffer: &__WOST_FRAMEBUFFER,
                    memory_map: &__WOST_MEMORY_MAP,
                    hhdm: &__WOST_DIRECT_MAP,
                    image: &__WOST_IMAGE,
                    rsdp: &__WOST_RSDP,
                    modules: &__WOST_MODULES,
                },
                __WOST_ENTRY_SIGNATURE,
            )
        }
    };
}
