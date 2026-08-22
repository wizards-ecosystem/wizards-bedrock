//! Bootloader handoff.
//!
//! [`entry!`](crate::entry) defines the kernel's entry point and the bootloader
//! request block. Everything the bootloader tells us arrives as [`BootInfo`],
//! whose types name no bootloader at all — swapping Limine out is a change to
//! `limine_glue.rs` and nothing else.

mod limine_glue;

use core::fmt;

pub use limine_glue::enter;
pub use pixel::{PixelFormat, Rgb};

use crate::mmio::MmioRegion;

/// Bytes in one framebuffer word — the only access width the region supports.
const WORD: usize = size_of::<u32>();

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
#[derive(Debug)]
pub struct Framebuffer {
    region: MmioRegion,
    info: FramebufferInfo,
    /// One `u32` per pixel of the visible area, at the device's own stride, or
    /// empty before a back buffer is attached.
    back: alloc::vec::Vec<u32>,
}

impl Framebuffer {
    /// Wrap a mapping the caller has vouched for, with no shadow yet.
    pub(crate) const fn new(region: MmioRegion, info: FramebufferInfo) -> Self {
        Self {
            region,
            info,
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
    pub fn attach_back_buffer(&mut self) -> bool {
        if !self.back.is_empty() {
            return true;
        }

        let words = self.words();
        if self.back.try_reserve_exact(words).is_err() {
            return false;
        }
        // The one read of device memory this design permits, paid once so that
        // no scroll ever pays it again.
        self.back
            .extend((0..words).map(|word| self.region.read_u32(word * WORD).unwrap_or(0)));
        true
    }

    /// Whether writes are being shadowed.
    pub fn has_back_buffer(&self) -> bool {
        !self.back.is_empty()
    }

    /// Words in the shadow: the device's stride times its visible height.
    fn words(&self) -> usize {
        self.info.pitch / WORD * self.info.height
    }

    /// Record a word in the shadow, if there is one.
    fn shadow(&mut self, offset: usize, value: u32) {
        if let Some(word) = self.back.get_mut(offset / WORD) {
            *word = value;
        }
    }

    /// The word at `offset`, from the shadow if there is one and from the
    /// device otherwise.
    fn word_at(&self, offset: usize) -> Option<u32> {
        match self.back.get(offset / WORD) {
            Some(word) => Some(*word),
            None => self.region.read_u32(offset).ok(),
        }
    }

    /// Set the pixel at (`x`, `y`), ignoring coordinates outside the visible
    /// area.
    ///
    /// Clipping is deliberate: a glyph drawn near the right or bottom edge
    /// legitimately runs off it. Anything that survives the clip is still
    /// bounds-checked by the underlying region.
    pub fn write_pixel(&mut self, x: usize, y: usize, colour: Rgb) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }
        let offset = pixel::offset(x, y, self.info.pitch, self.info.bytes_per_pixel);
        let word = self.info.format.pack(colour);
        let _ = self.region.write_u32(offset, word);
        self.shadow(offset, word);
    }

    /// Paint every visible pixel one colour.
    pub fn fill(&mut self, colour: Rgb) {
        self.fill_rows(0, self.info.height, colour);
    }

    /// Paint `rows` of pixels starting at row `y` one colour.
    pub fn fill_rows(&mut self, y: usize, rows: usize, colour: Rgb) {
        let word = self.info.format.pack(colour);
        for row in y..(y + rows).min(self.info.height) {
            let start = row * self.info.pitch;
            let _ = self.region.fill_u32(start, self.info.width, word);
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
        assert!(to_y <= from_y, "copy_rows moves rows upward only");

        let FramebufferInfo {
            width,
            pitch,
            bytes_per_pixel,
            ..
        } = self.info;
        for row in 0..rows {
            for column in 0..width {
                let from = pixel::offset(column, from_y + row, pitch, bytes_per_pixel);
                let to = pixel::offset(column, to_y + row, pitch, bytes_per_pixel);
                // From the shadow where there is one. Reading device memory to
                // scroll is what this exists to stop.
                if let Some(word) = self.word_at(from) {
                    let _ = self.region.write_u32(to, word);
                    self.shadow(to, word);
                }
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
    /// The mapping does not start on a 4-byte boundary, so 32-bit pixel writes
    /// would be misaligned.
    MisalignedAddress,
    /// The row pitch is not a multiple of four bytes, so every row after the
    /// first would be misaligned.
    MisalignedPitch(usize),
}

impl fmt::Display for FramebufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoResponse => f.write_str("bootloader ignored the framebuffer request"),
            Self::NoFramebuffer => f.write_str("bootloader listed no framebuffers"),
            Self::UnsupportedMemoryModel => f.write_str("framebuffer is not direct RGB"),
            Self::UnsupportedBpp(bpp) => write!(f, "unsupported depth: {bpp} bits per pixel"),
            Self::MisalignedAddress => f.write_str("framebuffer is not 4-byte aligned"),
            Self::MisalignedPitch(pitch) => {
                write!(f, "row pitch of {pitch} bytes is not a multiple of four")
            }
        }
    }
}

impl core::error::Error for FramebufferError {}

/// What a region of the physical address space is for.
///
/// Named after what the kernel may do with a region rather than after any one
/// bootloader's constants. `Unknown` carries the raw value so an unrecognised
/// type is reportable rather than silently folded into `Reserved` — the
/// discipline `005-code-conventions.md` requires of our own `#[non_exhaustive]`
/// enums, applied to a foreign one (D-42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MemoryKind {
    /// Free RAM, ours to hand out.
    Usable,
    /// The bootloader's own structures. Usable once nothing refers to them.
    BootloaderReclaimable,
    /// The kernel image and any modules loaded with it.
    KernelAndModules,
    /// A framebuffer.
    Framebuffer,
    /// ACPI tables. Usable once they have been parsed.
    AcpiReclaimable,
    /// ACPI non-volatile storage. Must survive.
    AcpiNvs,
    /// Reserved by firmware. Not ours.
    Reserved,
    /// Known-bad memory.
    BadMemory,
    /// A type this kernel does not recognise, with the value the bootloader
    /// gave. Treated as [`Reserved`](Self::Reserved) everywhere it matters.
    Unknown(u64),
}

impl MemoryKind {
    /// Whether the frame allocator may hand this region out.
    ///
    /// Only [`Usable`](Self::Usable). The reclaimable kinds are deliberately
    /// excluded: bootloader structures are still being read when the allocator
    /// is built, and ACPI tables have not been parsed yet. Reclaiming either is
    /// a later decision with a precondition attached, not a default.
    pub const fn is_allocatable(self) -> bool {
        matches!(self, Self::Usable)
    }

    /// Whether the kernel's own direct map should cover this region.
    ///
    /// Everything that is memory, and nothing that is not (D-45). The
    /// exclusions are the point: `Reserved` is where device holes live, and
    /// mapping one here would give it the direct map's cache type instead of
    /// the one its device needs — so devices are mapped deliberately, through
    /// `MmioRegion`, and never reached by accident.
    ///
    /// The inclusions are equally deliberate. The kernel is *executing on* a
    /// stack the bootloader placed in `BootloaderReclaimable`, and *logging
    /// through* a `Framebuffer` region; a direct map of only `Usable` memory
    /// would triple-fault the instant `CR3` changed.
    pub const fn is_direct_mapped(self) -> bool {
        matches!(
            self,
            Self::Usable
                | Self::BootloaderReclaimable
                | Self::KernelAndModules
                | Self::Framebuffer
                | Self::AcpiReclaimable
                | Self::AcpiNvs
        )
    }
}

impl fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Usable => f.write_str("usable"),
            Self::BootloaderReclaimable => f.write_str("bootloader reclaimable"),
            Self::KernelAndModules => f.write_str("kernel and modules"),
            Self::Framebuffer => f.write_str("framebuffer"),
            Self::AcpiReclaimable => f.write_str("acpi reclaimable"),
            Self::AcpiNvs => f.write_str("acpi nvs"),
            Self::Reserved => f.write_str("reserved"),
            Self::BadMemory => f.write_str("bad memory"),
            Self::Unknown(value) => write!(f, "unknown type {value}"),
        }
    }
}

/// One entry of the physical memory map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegion {
    /// First physical byte.
    pub base: u64,
    /// Length in bytes.
    pub len: u64,
    /// What the region is for.
    pub kind: MemoryKind,
}

/// Regions [`MemoryMap`] can hold.
///
/// Firmware maps run to a few dozen entries; the largest seen from OVMF is
/// under thirty. Overflow is counted and reported rather than truncated
/// silently, so a machine that exceeds this says so in the boot log.
const MAX_REGIONS: usize = 128;

/// The physical memory map, copied out of the bootloader's structures.
///
/// Copied rather than borrowed on purpose: the map itself lives in
/// bootloader-reclaimable memory, and a kernel that intends to reclaim that
/// memory cannot also be holding a reference into it.
#[derive(Clone, Copy)]
pub struct MemoryMap {
    regions: [MemoryRegion; MAX_REGIONS],
    len: usize,
    dropped: usize,
}

impl MemoryMap {
    /// An empty map.
    pub(crate) const fn new() -> Self {
        Self {
            regions: [MemoryRegion {
                base: 0,
                len: 0,
                kind: MemoryKind::Reserved,
            }; MAX_REGIONS],
            len: 0,
            dropped: 0,
        }
    }

    /// Append a region, counting it as dropped if there is no room.
    pub(crate) const fn push(&mut self, region: MemoryRegion) {
        if self.len == MAX_REGIONS {
            self.dropped += 1;
            return;
        }
        self.regions[self.len] = region;
        self.len += 1;
    }

    /// Every region the bootloader reported, in the order it reported them.
    pub fn regions(&self) -> impl Iterator<Item = MemoryRegion> + '_ {
        self.regions[..self.len].iter().copied()
    }

    /// Regions the bootloader reported that did not fit.
    ///
    /// Non-zero means the kernel is running on less memory than the machine
    /// has, which is a bug in this module's region limit rather than in the
    /// machine.
    pub const fn dropped(&self) -> usize {
        self.dropped
    }

    /// Total bytes across regions the frame allocator may hand out.
    pub fn allocatable_bytes(&self) -> u64 {
        self.regions()
            .filter(|region| region.kind.is_allocatable())
            .map(|region| region.len)
            .sum()
    }
}

/// Counts and totals. Printing 128 regions from a panic handler helps nobody.
impl fmt::Debug for MemoryMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryMap")
            .field("regions", &self.len)
            .field("dropped", &self.dropped)
            .field("allocatable", &self.allocatable_bytes())
            .finish()
    }
}

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

    /// Where the image begins in physical memory.
    pub const fn physical_base(self) -> u64 {
        self.physical
    }

    /// Where the image begins in virtual memory — the linker's base.
    pub const fn virtual_base(self) -> u64 {
        self.virtual_
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
    // These are linker-defined symbols with no storage of their own. Only
    // their addresses are ever taken — never their contents — which is what
    // makes `u8` the conventional and harmless declared type, and what makes
    // `&raw const` safe where a read would not be.
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
}

/// Why part of the boot handoff is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HandoffError {
    /// The bootloader ignored the request.
    NoResponse,
    /// The bootloader answered, but with nothing in it.
    Empty,
}

impl fmt::Display for HandoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoResponse => f.write_str("bootloader ignored the request"),
            Self::Empty => f.write_str("bootloader answered with nothing"),
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
        ExecutableAddressRequest, FramebufferRequest, HhdmRequest, MemoryMapRequest,
        RequestsEndMarker, RequestsStartMarker, RsdpRequest,
    };

    pub use super::limine_glue::base_revision;
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
        #[unsafe(link_section = ".requests_end_marker")]
        static __WOST_REQUESTS_END: $crate::boot::__macro::RequestsEndMarker =
            $crate::boot::__macro::RequestsEndMarker::new();

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        extern "C" fn kmain() -> ! {
            $crate::boot::enter(
                &__WOST_BASE_REVISION,
                &__WOST_FRAMEBUFFER,
                &__WOST_MEMORY_MAP,
                &__WOST_DIRECT_MAP,
                &__WOST_IMAGE,
                &__WOST_RSDP,
                __WOST_ENTRY_SIGNATURE,
            )
        }
    };
}
