//! Physical memory: which frames exist, and who owns each one.
//!
//! The boundary this module draws is the one `008-research-notes-2.md` §3.1
//! took from Asterinas and D-43 adopted. The *mechanism* is here: a [`Frame`]
//! is a handle whose existence means "this frame is allocated to me", and
//! whose `Drop` gives it back. The *policy* — how a free frame is found — is
//! `frame_space::FrameSpace`, a `libs/` crate that speaks only in frame
//! indices and is unit-tested on the host. Swapping first-fit for a buddy
//! changes that crate and nothing here.
//!
//! Physical memory is reached through the bootloader's direct map until the
//! kernel builds page tables of its own. [`init`] records the offset; every
//! access through it is bounds-checked against the frame the caller owns.
//!
//! Not yet here, and deliberately: the untyped-memory split (`UFrame`) that
//! keeps user-mapped and DMA-capable frames out of the typed API. It arrives
//! with the first caller that can map a frame into a second address space,
//! which is Phase 5 — writing it now would be a safety contract with nothing
//! to check it against (`005-code-conventions.md`).

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use frame_space::{FRAME_SIZE, FrameSpace, words_for};

use crate::boot::{DirectMap, MemoryMap};
use crate::panic::PanicToken;
use crate::sync::{Once, SpinLock};

/// A physical address.
pub type Paddr = u64;

/// Highest physical address the frame allocator tracks.
///
/// Two bits of `.bss` per frame, so 16 GiB costs one megabyte — which the
/// image does not carry, because both bitmaps start zeroed. Memory above this
/// is reported as ignored at boot rather than silently lost.
pub const CEILING: u64 = 16 << 30;

/// The bookkeeping, sized for [`CEILING`].
type Frames = FrameSpace<{ words_for(CEILING) }>;

static FRAMES: SpinLock<Frames> = SpinLock::new(Frames::new());

/// The direct map's offset, or zero before [`init`] has run.
///
/// An atomic rather than a field of the locked structure: zeroing a frame must
/// not be done while holding the allocator's lock, and the offset never
/// changes once written. Zero is an unambiguous "not yet" — the higher-half
/// direct map is never at zero.
static DIRECT_MAP_OFFSET: AtomicU64 = AtomicU64::new(0);

static INIT: Once = Once::new();

/// What [`init`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Frames the allocator now manages.
    pub managed: usize,
    /// Frames dropped because they lie above [`CEILING`].
    pub ignored: usize,
    /// Memory-map regions the handoff could not carry.
    pub dropped_regions: usize,
}

impl Report {
    /// Bytes under management.
    pub const fn managed_bytes(&self) -> u64 {
        self.managed as u64 * FRAME_SIZE
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} MiB usable in {} frames",
            self.managed_bytes() / (1 << 20),
            self.managed
        )?;
        if self.ignored != 0 {
            write!(
                f,
                ", {} frames above the {} GiB ceiling ignored",
                self.ignored,
                CEILING >> 30
            )?;
        }
        if self.dropped_regions != 0 {
            write!(f, ", {} map regions dropped", self.dropped_regions)?;
        }
        Ok(())
    }
}

/// Take every allocatable region of `map` under management.
///
/// Idempotent: the second caller returns the same report having changed
/// nothing. Only [`MemoryKind::Usable`](crate::boot::MemoryKind::Usable) is
/// inserted, so the kernel image, the bootloader's structures and the ACPI
/// tables are excluded by the map itself rather than by a range this code
/// would have to compute — including the megabyte of bitmap below, which lives
/// in `.bss` inside the loaded image.
pub fn init(map: &MemoryMap, direct: DirectMap) -> Report {
    if !INIT.claim() {
        return report();
    }

    DIRECT_MAP_OFFSET.store(direct.offset(), Ordering::Release);

    let mut ignored = 0;
    let mut frames = FRAMES.lock();
    for region in map.regions().filter(|region| region.kind.is_allocatable()) {
        ignored += frames.insert(region.base, region.len).ignored;
    }
    let managed = frames.managed_count();
    drop(frames);

    Report {
        managed,
        ignored,
        dropped_regions: map.dropped(),
    }
}

/// Frames managed and free, right now.
fn report() -> Report {
    let frames = FRAMES.lock();
    Report {
        managed: frames.managed_count(),
        ..Report::default()
    }
}

/// Frames available to allocate.
pub fn free_frames() -> usize {
    FRAMES.lock().free_count()
}

/// Frames under management, allocated or not.
pub fn managed_frames() -> usize {
    FRAMES.lock().managed_count()
}

/// Reclaim the frame allocator's lock on the terminal panic path.
///
/// Called by [`panic::reclaim`](crate::panic::reclaim) rather than directly, on
/// the standing obligation `005-code-conventions.md` places on every `static`
/// lock: a panic can interrupt code holding this one, and a handler that
/// reports free memory would otherwise spin forever.
pub(crate) fn force_unlock(panic: &PanicToken) {
    FRAMES.force_unlock(panic);
}

/// One physical frame, owned for as long as this value lives.
///
/// Dropping it returns the frame to the allocator. There is no way to
/// construct one except by allocating, and no way to copy one, so a frame is
/// owned by exactly one `Frame` at a time — which is what lets
/// [`zero`](Self::zero) write through the direct map without asking anything
/// of its caller.
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    index: usize,
}

impl Frame {
    /// Where the frame begins in physical memory.
    pub const fn address(&self) -> Paddr {
        self.index as u64 * FRAME_SIZE
    }

    /// The frame's index — its address divided by the frame size.
    pub const fn index(&self) -> usize {
        self.index
    }

    /// Fill every byte of the frame with `byte`.
    ///
    /// Returns `false` if the direct map cannot reach it, which means either
    /// that [`init`] has not run or that the bootloader's map does not cover
    /// this frame. Neither is recoverable at the call site, but both are
    /// reportable, and a silent no-op would hand out a page table full of
    /// whatever the last owner left.
    pub fn fill(&mut self, byte: u8) -> bool {
        let Some(base) = direct_map_pointer(self.address()) else {
            return false;
        };
        // SAFETY: `self` owns this frame — no other `Frame` can name it, since
        // the allocator hands out one handle per frame and this method needs
        // `&mut self`. `direct_map_pointer` confirmed the bootloader's map
        // covers the whole frame, and the write stays inside it.
        unsafe { core::ptr::write_bytes(base, byte, FRAME_SIZE as usize) };
        true
    }

    /// Fill the frame with zeros.
    pub fn zero(&mut self) -> bool {
        self.fill(0)
    }

    /// Whether every byte of the frame is zero, or `None` if the direct map
    /// cannot reach it.
    ///
    /// The read-back half of [`zero`](Self::zero). Zero-on-allocate is a D-12
    /// obligation, and an obligation nothing can observe is a comment: this is
    /// what lets the boot-time check in `kernel/src/selftest.rs` see it.
    pub fn is_zero(&self) -> Option<bool> {
        /// Words in a frame. Read a word at a time rather than a byte: the
        /// answer is the same and there are eight times fewer accesses.
        const WORDS: usize = FRAME_SIZE as usize / size_of::<u64>();

        let base = direct_map_pointer(self.address())?.cast::<u64>();
        let mut zero = true;
        for index in 0..WORDS {
            // SAFETY: `direct_map_pointer` confirmed the map covers all
            // `FRAME_SIZE` bytes of the frame `self` owns, and `index` stays
            // below the number of words in one frame, so the result is inside
            // that mapping.
            let word = unsafe { base.add(index) };
            // SAFETY: a frame is `FRAME_SIZE`-aligned and so is the direct
            // map, so `word` is `u64`-aligned, and the mapping is readable.
            zero &= unsafe { word.read_volatile() } == 0;
        }
        Some(zero)
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // A refusal here means the bookkeeping disagrees with the handle, which
        // is a bug this crate cannot correct from a `Drop`. Leaking the frame
        // is the safe half of the choice.
        let _ = FRAMES.lock().free(self.index);
    }
}

/// A run of consecutive physical frames, owned for as long as this value
/// lives.
///
/// What a 2 MiB mapping and a multi-page stack are allocated as. The same
/// ownership rule as [`Frame`]: one handle per run, released whole on drop.
#[derive(Debug, PartialEq, Eq)]
pub struct Segment {
    first: usize,
    count: usize,
}

impl Segment {
    /// Where the run begins in physical memory.
    pub const fn address(&self) -> Paddr {
        self.first as u64 * FRAME_SIZE
    }

    /// Frames in the run.
    pub const fn frames(&self) -> usize {
        self.count
    }

    /// Bytes in the run.
    pub const fn len(&self) -> u64 {
        self.count as u64 * FRAME_SIZE
    }

    /// Whether the run is empty. Never true — the allocator refuses a run of
    /// zero frames — but `clippy::len_without_is_empty` asks, and answering is
    /// cheaper than an `allow`.
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Fill every byte of the run with `byte`.
    ///
    /// False if the direct map cannot reach the whole run, having written
    /// nothing.
    pub fn fill(&mut self, byte: u8) -> bool {
        let Some(base) = direct_map_span(self.address(), self.len()) else {
            return false;
        };
        // SAFETY: `self` owns every frame in the run, and `direct_map_span`
        // confirmed the bootloader's map covers all `len()` bytes of it.
        unsafe { core::ptr::write_bytes(base, byte, self.len() as usize) };
        true
    }

    /// Fill the run with zeros.
    pub fn zero(&mut self) -> bool {
        self.fill(0)
    }
}

impl Drop for Segment {
    fn drop(&mut self) {
        // As for `Frame`: a refusal means the bookkeeping disagrees with the
        // handle, which a `Drop` cannot correct. Leaking is the safe half.
        let _ = FRAMES.lock().free_contiguous(self.first, self.count);
    }
}

/// Allocate one frame, with unspecified contents.
pub fn allocate() -> Option<Frame> {
    let index = FRAMES.lock().allocate()?;
    Some(Frame { index })
}

/// Allocate `count` consecutive frames, the first aligned to `align` frames.
///
/// `align` is rounded up to a power of two. A 2 MiB page wants
/// `allocate_contiguous(512, 512)`.
pub fn allocate_contiguous(count: usize, align: usize) -> Option<Segment> {
    let first = FRAMES.lock().allocate_contiguous(count, align)?;
    Some(Segment { first, count })
}

/// Allocate one frame filled with zeros.
///
/// The form page-table code wants: an entry pointing at a frame of stale
/// contents is a mapping to whatever the last owner wrote. Returns `None` if
/// the frame cannot be reached through the direct map, having first given it
/// back — an unzeroable frame is worse than no frame.
pub fn allocate_zeroed() -> Option<Frame> {
    let mut frame = allocate()?;
    frame.zero().then_some(frame)
}

/// Where the direct map makes one frame at `physical` accessible.
fn direct_map_pointer(physical: Paddr) -> Option<*mut u8> {
    direct_map_span(physical, FRAME_SIZE)
}

/// Where the direct map makes `len` bytes at `physical` accessible.
///
/// Checks the whole span, not merely its first byte, so a caller that got a
/// pointer may write `len` bytes through it. `None` before [`init`] has run.
fn direct_map_span(physical: Paddr, len: u64) -> Option<*mut u8> {
    let offset = DIRECT_MAP_OFFSET.load(Ordering::Acquire);
    if offset == 0 {
        return None;
    }
    let end = physical.checked_add(len)?;
    let _ = offset.checked_add(end)?;
    offset
        .checked_add(physical)
        .map(|address| address as *mut u8)
}
