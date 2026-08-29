//! Physical memory: which frames exist, and who owns each one.
//!
//! Mechanism only, per D-43: a [`Frame`] is a handle whose existence means
//! the frame is allocated, and whose `Drop` returns it. The search itself is
//! `frame_space::FrameSpace`, host-tested, and swapping first-fit for a buddy
//! changes that crate and nothing here.
//!
//! Frames are reached through the direct map, whose offset [`init`] records.
//!
//! The untyped split D-44 adopted is [`USegment`]. "Untyped" here means
//! *externally modifiable, plain bytes only* — never *donated for kernel
//! object creation*, which is the reading seL4 gives the word and the one this
//! kernel rejected. A run becomes untyped the moment it is mapped where ring 3
//! can write it, and the type says so from then on (D-107).

pub mod heap;

use alloc::sync::Arc;
use core::fmt;
use core::mem::{align_of, size_of};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use frame_space::{FRAME_SIZE, FrameSpace, words_for};

use crate::boot::{DirectMap, MemoryKind, MemoryMap, MemoryRegion};
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
/// nothing. Only [`MemoryKind::Usable`] is
/// inserted, so the kernel image, the bootloader's structures and the ACPI
/// tables are excluded by the map itself rather than by a range this code
/// would have to compute — including the megabyte of bitmap below, which lives
/// in `.bss` inside the loaded image.
pub fn init(map: &MemoryMap, direct: DirectMap) -> Report {
    if !INIT.claim() {
        return report();
    }

    DIRECT_MAP_OFFSET.store(direct.offset(), Ordering::Release);
    *MAP.lock_leaf() = Some(*map);

    let mut ignored = 0;
    let mut frames = FRAMES.lock_leaf();
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
    let frames = FRAMES.lock_leaf();
    Report {
        managed: frames.managed_count(),
        ..Report::default()
    }
}

/// Frames available to allocate.
pub fn free_frames() -> usize {
    FRAMES.lock_leaf().free_count()
}

/// Read-only access to physical firmware tables.
///
/// This deliberately cannot read usable, bootloader, kernel, reserved, or
/// device memory. That narrower authority prevents safe code from creating a
/// raw direct-map alias of a live Rust allocation or an MMIO register bank.
#[derive(Debug, Clone, Copy)]
pub struct FirmwareMemory(());

impl FirmwareMemory {
    /// Whether every byte is described exclusively as ACPI table storage.
    pub fn contains(&self, address: Paddr, len: usize) -> bool {
        let Ok(len) = u64::try_from(len) else {
            return false;
        };
        if len == 0 {
            return false;
        }
        let Some(end) = address.checked_add(len) else {
            return false;
        };
        let guard = MAP.lock_leaf();
        let Some(map) = guard.as_ref() else {
            return false;
        };
        let overlaps = |region: &MemoryRegion| {
            address < region.base.saturating_add(region.len) && region.base < end
        };
        map.covers(address, end)
            && map
                .regions()
                .filter(overlaps)
                .all(|region| region.kind.is_firmware_table())
    }

    /// Copy table bytes at `address` into `buffer` with volatile reads.
    pub fn read(&self, address: Paddr, buffer: &mut [u8]) -> bool {
        if !self.contains(address, buffer.len()) {
            return false;
        }
        let Some(base) = direct_pointer(address, buffer.len() as u64) else {
            return false;
        };
        for (index, slot) in buffer.iter_mut().enumerate() {
            // SAFETY: `contains` proved this byte is immutable ACPI storage,
            // and `direct_pointer` proved its direct-map address does not
            // overflow. No safe API forms a mutable reference to this range.
            let byte = unsafe { base.add(index) };
            // SAFETY: as above; `byte` is within the checked firmware range.
            *slot = unsafe { byte.read_volatile() };
        }
        true
    }
}

/// Acquire the narrow reader after the memory map and direct map exist.
pub fn firmware_memory() -> Option<FirmwareMemory> {
    (MAP.lock_leaf().is_some() && DIRECT_MAP_OFFSET.load(Ordering::Acquire) != 0)
        .then_some(FirmwareMemory(()))
}

/// Whether `start..end` is disjoint from every RAM or firmware allocation.
pub(crate) fn device_range_is_eligible(start: Paddr, end: Paddr) -> bool {
    if start >= end {
        return false;
    }
    let guard = MAP.lock_leaf();
    let Some(map) = guard.as_ref() else {
        return false;
    };
    map.regions().all(|region| {
        let overlaps = start < region.base.saturating_add(region.len) && region.base < end;
        !overlaps
            || matches!(
                region.kind,
                crate::boot::MemoryKind::Reserved | crate::boot::MemoryKind::Unknown(_)
            )
    })
}

/// The memory map [`init`] was given, for ownership and firmware checks.
static MAP: SpinLock<Option<MemoryMap>> = SpinLock::new(None);

/// Whether the memory map covers `start..end` **with memory**.
///
/// One of the queries that replaced `with_memory_map` (D-121). The closure
/// form is gone: it handed a caller `&MemoryMap` under a leaf guard, whose
/// obligation is that nothing is called while it is live — and a closure is a
/// call by construction. Nothing checked it, and one caller was walking page
/// tables under it.
///
/// `false` before the map is up, which is the honest answer: until then
/// nothing has described anything.
pub fn map_covers_memory(start: Paddr, end: Paddr) -> bool {
    MAP.lock_leaf()
        .as_ref()
        .is_some_and(|map| map.covers_memory(start, end))
}

/// Where the boot map covers `len` bytes with memory, or `None` before it is up.
///
/// See [`crate::boot::MemoryMap::first_memory_run`] for why a caller asks
/// rather than names an address (D-177).
pub fn first_memory_run(len: u64) -> Option<Paddr> {
    MAP.lock_leaf()
        .as_ref()
        .and_then(|map| map.first_memory_run(len))
}

/// The direct map [`init`] recorded, or `None` before it has run.
///
/// Handed back so callers that already hold a physical address can name the
/// same byte virtually — which is what the read-back check on the direct map's
/// own permissions needs.
pub fn direct_map() -> Option<DirectMap> {
    match DIRECT_MAP_OFFSET.load(Ordering::Acquire) {
        0 => None,
        offset => Some(DirectMap::new(offset)),
    }
}

/// Reclaim the frame allocator's lock on the terminal panic path.
///
/// Called by the terminal panic boundary rather than directly, on
/// the standing obligation `005-code-conventions.md` places on every `static`
/// lock: a panic can interrupt code holding this one, and a handler that
/// reports free memory would otherwise spin forever.
pub(crate) fn force_unlock(panic: &PanicToken) {
    FRAMES.force_unlock(panic);
    MAP.force_unlock(panic);
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
    /// Rebuild the handle for a frame this crate allocated and forgot.
    ///
    /// # Safety
    ///
    /// `address` must be a frame [`allocate`] or [`allocate_zeroed`] returned
    /// and whose handle was forgotten, and no other `Frame` may name it.
    /// Dropping the result frees it, so a second handle over one frame is a
    /// double free — and one built over a frame nothing allocated corrupts the
    /// allocator's records.
    pub(crate) const unsafe fn from_raw(address: Paddr) -> Self {
        Self {
            index: (address / FRAME_SIZE) as usize,
        }
    }

    /// Where the frame begins in physical memory.
    pub const fn address(&self) -> Paddr {
        self.index as u64 * FRAME_SIZE
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
        // `&mut self`. The frame is allocatable memory, and every allocatable
        // region is direct-mapped, so the whole of it is reachable through the
        // pointer just formed.
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
            // SAFETY: the frame `self` owns is allocatable memory and so is
            // direct-mapped whichever map is in force, and `index` stays below
            // the number of words in one frame, so the result is inside it.
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
        let _ = FRAMES.lock_leaf().free(self.index);
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
    #[expect(
        clippy::len_without_is_empty,
        reason = "allocation refuses a zero-frame segment"
    )]
    pub const fn len(&self) -> u64 {
        self.count as u64 * FRAME_SIZE
    }

    /// Fill every byte of the run with `byte`.
    ///
    /// False if the direct map cannot reach the whole run, having written
    /// nothing.
    pub fn fill(&mut self, byte: u8) -> bool {
        let Some(base) = direct_pointer(self.address(), self.len()) else {
            return false;
        };
        // SAFETY: `self` owns every frame in the run, and `direct_pointer`
        // confirmed the bootloader's map covers all `len()` bytes of it.
        unsafe { core::ptr::write_bytes(base, byte, self.len() as usize) };
        true
    }

    /// Fill the run with zeros.
    pub fn zero(&mut self) -> bool {
        self.fill(0)
    }

    /// Write `bytes` at `offset` within the run.
    ///
    /// The one way to put chosen content into frames from safe code, and what
    /// loading a program is made of: the kernel may not hold a raw pointer
    /// (D-13), and the image has to be in the pages *before* they are mapped
    /// read-only and executable rather than mapped writable and demoted
    /// afterwards (D-96).
    ///
    /// `false` if the write would run past the run, or if the direct map
    /// cannot reach it.
    pub fn write_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        let Some(end) = offset.checked_add(bytes.len()) else {
            return false;
        };
        if end as u64 > self.len() {
            return false;
        }
        let Some(base) = direct_pointer(self.address(), self.len()) else {
            return false;
        };
        // SAFETY: the bounds check above places `offset` inside the run, and
        // `self` owns every frame of it — the direct map covers every frame
        // the allocator manages, so the whole run is mapped. `direct_pointer`
        // did the arithmetic and refused an overflow; it checked no coverage,
        // and its doc says so (D-140).
        let destination = unsafe { base.add(offset) };
        // SAFETY: `self` owns every frame in the run — one handle per run, and
        // this takes `&mut self` — and `bytes` ends inside it. The two cannot
        // overlap: `bytes` is the caller's and this is the run.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
        true
    }

    /// Read `into.len()` bytes from `offset` within the run.
    ///
    /// The counterpart to [`write_at`](Self::write_at).
    ///
    /// [`Shared`] instead has volatile payload copies and atomic publication
    /// indices, because ordinary copies do not define the compiler/SMP
    /// contract for memory ring 3 can change (D-221). A [`USegment`] has no
    /// reader at all: the only way to get bytes out of one is
    /// [`reclaim`](USegment::reclaim), which clears them first, so a frame
    /// that held a program's stack cannot come back to the kernel still
    /// holding it (D-107).
    ///
    /// The first version of this comment said the exception did not exist
    /// (D-115).
    ///
    /// `false` if the read would run past the run, or if the direct map cannot
    /// reach it, having written nothing into `into`.
    pub fn read_at(&self, offset: usize, into: &mut [u8]) -> bool {
        let Some(end) = offset.checked_add(into.len()) else {
            return false;
        };
        if end as u64 > self.len() {
            return false;
        }
        let Some(base) = direct_pointer(self.address(), self.len()) else {
            return false;
        };
        // SAFETY: as the write above — the bounds check places `offset` inside
        // the run, `self` owns every frame of it, and the direct map covers
        // every frame the allocator manages. `direct_pointer` did arithmetic,
        // not a coverage check (D-140).
        let source = unsafe { base.add(offset) };
        // SAFETY: `self` owns every frame in the run — one handle per run —
        // and `into` is the caller's, so the two cannot overlap.
        unsafe { core::ptr::copy_nonoverlapping(source, into.as_mut_ptr(), into.len()) };
        true
    }
}

impl Segment {
    /// Rebuild the handle for a run this crate allocated.
    ///
    /// # Safety
    ///
    /// `count` frames from `address` must be a run that
    /// [`allocate_contiguous`] returned and whose handle was forgotten, and
    /// no other `Segment` may name any of them. Dropping the result frees
    /// them, so a second handle over the same run is a double free.
    pub(crate) const unsafe fn from_raw(address: Paddr, count: usize) -> Self {
        Self {
            first: (address / FRAME_SIZE) as usize,
            count,
        }
    }
}

impl Drop for Segment {
    fn drop(&mut self) {
        // As for `Frame`: a refusal means the bookkeeping disagrees with the
        // handle, which a `Drop` cannot correct. Leaking is the safe half.
        let _ = FRAMES.lock_leaf().free_contiguous(self.first, self.count);
    }
}

/// Frames made reachable by a device for direct memory access.
///
/// A command whose completion is uncertain cannot return its frames to the
/// allocator: the device may still write through the physical address it was
/// given. [`retain`](Self::retain) makes that failure mode safe by permanently
/// withholding the allocation and removing every safe way to access it.
#[derive(Debug, PartialEq, Eq)]
pub struct DmaSegment {
    segment: Option<Segment>,
}

impl DmaSegment {
    /// Where the run begins, unless an uncertain device still owns it.
    pub const fn address(&self) -> Option<Paddr> {
        match &self.segment {
            Some(segment) => Some(segment.address()),
            None => None,
        }
    }

    /// Bytes in the run, unless uncertain device ownership retained it.
    #[expect(
        clippy::len_without_is_empty,
        reason = "None means retained by a device, not an empty collection"
    )]
    pub const fn len(&self) -> Option<u64> {
        match &self.segment {
            Some(segment) => Some(segment.len()),
            None => None,
        }
    }

    /// Whether the run is unavailable because a device may still reach it.
    pub const fn is_retained(&self) -> bool {
        self.segment.is_none()
    }

    /// Fill every available byte with `byte`.
    pub fn fill(&mut self, byte: u8) -> bool {
        self.segment
            .as_mut()
            .is_some_and(|segment| segment.fill(byte))
    }

    /// Fill every available byte with zeros.
    pub fn zero(&mut self) -> bool {
        self.fill(0)
    }

    /// Write `bytes` at `offset` while the run is host-owned.
    pub fn write_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        self.segment
            .as_mut()
            .is_some_and(|segment| segment.write_at(offset, bytes))
    }

    /// Read bytes while the run is host-owned.
    pub fn read_at(&self, offset: usize, into: &mut [u8]) -> bool {
        self.segment
            .as_ref()
            .is_some_and(|segment| segment.read_at(offset, into))
    }

    /// Permanently withhold the run from both safe access and recycling.
    ///
    /// Idempotent, so a controller can retain everything it owns without
    /// distinguishing the object that made its state uncertain.
    pub fn retain(&mut self) {
        if let Some(segment) = self.segment.take() {
            core::mem::forget(segment);
        }
    }
}

impl From<Segment> for DmaSegment {
    fn from(segment: Segment) -> Self {
        Self {
            segment: Some(segment),
        }
    }
}

/// A run of frames whose bytes are ring 3's.
///
/// D-44's untyped split, and the whole of what "untyped" means here: the
/// contents are outside the kernel's control, so nothing may be read out of
/// one and believed. It is *not* seL4's meaning — a `USegment` is never
/// retyped into a kernel object, because this kernel does not build objects
/// out of donated memory (D-107).
///
/// One-way by construction. A `Segment` becomes a `USegment` when it is handed
/// to a user mapping, and the only road back is
/// [`reclaim`](Self::reclaim), which zeroes first: a frame that held a
/// program's stack and is handed to the kernel still holding it is how a
/// process reads another's memory.
#[derive(Debug)]
pub struct USegment(Segment);

impl USegment {
    /// Where the run begins in physical memory.
    pub const fn address(&self) -> Paddr {
        self.0.address()
    }

    /// Pin this run for sharing, giving up the right to map it as owned.
    ///
    /// Consuming, and [`Shared`] is not convertible back. That is the whole
    /// mechanism: `map_shared` borrows and writes no `OWNED` bit, `map_user`
    /// consumes and writes one — so a run passed to both would be mapped
    /// twice, and unmapping the owned mapping would return frames the shared
    /// mapping still reaches. Ring 3 would then be writing memory the
    /// allocator had handed to a page table (D-115).
    #[must_use]
    pub fn share(self) -> Shared {
        Shared(Arc::new(SharedAllocation(self.0)))
    }

    /// Take the run back for the kernel, with every byte cleared.
    ///
    /// The zeroing is the conversion, not a courtesy attached to it: what
    /// makes this sound is that nothing a program left behind survives into
    /// the frame's next life (D-12's zero-on-allocate, applied at the other
    /// end).
    ///
    /// `None` if the direct map cannot reach the run, in which case the frames
    /// are **leaked rather than freed**: they still hold ring 3's bytes, and
    /// returning those to the allocator is the disclosure this exists to
    /// prevent.
    pub fn reclaim(self) -> Option<Segment> {
        let mut segment = self.0;
        if segment.zero() {
            return Some(segment);
        }
        // The clear failed, so these frames still hold a program's bytes. They
        // must not go back to the allocator: leaking is the safe half, exactly
        // as it is in `Frame`'s and `Segment`'s own `Drop` when the bookkeeping
        // disagrees. Dropping here would have returned them unzeroed, which is
        // the one path where this type's whole purpose lapsed silently
        // (D-115).
        core::mem::forget(segment);
        None
    }
}

/// A run pinned for sharing: mapped into an address space without being given
/// to it, and never mappable as owned.
///
/// The kernel keeps these frames and writes them through the direct map while
/// ring 3 reads them through the mapping — which is what a shared ring is
/// (D-113). Reachable only through [`USegment::share`], and with no road back,
/// so the aliasing that would let one run be both shared and owned is not
/// expressible.
#[derive(Debug)]
struct SharedAllocation(Segment);

/// A non-accessing ownership reference retained by a shared mapping.
#[derive(Debug)]
pub(crate) struct SharedKeepalive {
    _allocation: Arc<SharedAllocation>,
}

/// A pinned run with a narrow shared-memory access boundary and non-accessing
/// mapping owners.
#[derive(Debug)]
pub struct Shared(Arc<SharedAllocation>);

impl Shared {
    /// Where the run begins in physical memory.
    pub fn address(&self) -> Paddr {
        self.0.0.address()
    }

    /// Bytes in the run.
    #[expect(
        clippy::len_without_is_empty,
        reason = "shared allocations cannot contain zero frames"
    )]
    pub fn len(&self) -> u64 {
        self.0.0.len()
    }

    /// Volatile-write `bytes` at `offset` within the run.
    ///
    /// This is the payload half of a shared-memory publication protocol. It
    /// prevents the compiler from inventing, merging or eliding accesses, but
    /// it is not a memory-ordering edge: a producer must follow the copy with
    /// [`store_release_u32`](Self::store_release_u32), and a consumer must
    /// first observe that index through
    /// [`load_acquire_u32`](Self::load_acquire_u32).
    pub fn write_volatile_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        let Some(end) = offset.checked_add(bytes.len()) else {
            return false;
        };
        if end as u64 > self.len() {
            return false;
        }
        let Some(base) = direct_pointer(self.address(), self.len()) else {
            return false;
        };
        for (index, byte) in bytes.iter().copied().enumerate() {
            let destination = base.wrapping_add(offset + index);
            // SAFETY: the bounds check places every destination inside the
            // allocated run, and the direct map covers allocator-owned frames.
            // Volatile access is required because ring 3 may observe the byte.
            unsafe { destination.write_volatile(byte) };
        }
        true
    }

    /// Volatile-read `into.len()` bytes from `offset` within the run.
    ///
    /// **Every byte read here is ring 3's**, and the rule is that the copy is
    /// validated after it is taken and never re-read (D-79). This is the
    /// payload half of the protocol; the consumer must first acquire-load the
    /// producer's index with
    /// [`load_acquire_u32`](Self::load_acquire_u32).
    pub fn read_volatile_at(&self, offset: usize, into: &mut [u8]) -> bool {
        let Some(end) = offset.checked_add(into.len()) else {
            return false;
        };
        if end as u64 > self.len() {
            return false;
        }
        let Some(base) = direct_pointer(self.address(), self.len()) else {
            return false;
        };
        for (index, byte) in into.iter_mut().enumerate() {
            let source = base.wrapping_add(offset + index);
            // SAFETY: the bounds check places every source inside the
            // allocated run, and the direct map covers allocator-owned frames.
            // Volatile access is required because ring 3 may rewrite the byte.
            *byte = unsafe { source.read_volatile() };
        }
        true
    }

    /// Acquire-load one aligned little-endian publication index.
    ///
    /// `None` means the four-byte word is out of bounds, misaligned or not
    /// reachable through the direct map. An acquired producer tail makes its
    /// earlier volatile payload writes visible; an acquired consumer head
    /// makes its earlier payload reads precede slot reuse.
    pub fn load_acquire_u32(&self, offset: usize) -> Option<u32> {
        let word = self.atomic_u32(offset)?;
        Some(u32::from_le(word.load(Ordering::Acquire)))
    }

    /// Release-store one aligned little-endian publication index.
    ///
    /// `false` means the four-byte word is out of bounds, misaligned or not
    /// reachable through the direct map. A producer publishes its completed
    /// volatile payload writes through a tail; a consumer publishes completed
    /// payload reads through a head before the producer may reuse the slot.
    pub fn store_release_u32(&self, offset: usize, value: u32) -> bool {
        let Some(word) = self.atomic_u32(offset) else {
            return false;
        };
        word.store(value.to_le(), Ordering::Release);
        true
    }

    /// The aligned atomic word at `offset`, if it is wholly inside the run.
    fn atomic_u32(&self, offset: usize) -> Option<&AtomicU32> {
        let end = offset.checked_add(size_of::<u32>())?;
        if end as u64 > self.len() {
            return None;
        }
        let base = direct_pointer(self.address(), self.len())?;
        let pointer = base.wrapping_add(offset).cast::<AtomicU32>();
        if !(pointer as usize).is_multiple_of(align_of::<AtomicU32>()) {
            return None;
        }
        // SAFETY: the checked word lies inside this allocation and has
        // `AtomicU32` alignment. Shared-memory clients access publication
        // indices as one aligned 32-bit atomic word under D-221.
        Some(unsafe { &*pointer })
    }

    /// Retain the allocation without exposing its bytes.
    pub(crate) fn keepalive(&self) -> SharedKeepalive {
        SharedKeepalive {
            _allocation: Arc::clone(&self.0),
        }
    }
}

impl From<Segment> for USegment {
    fn from(segment: Segment) -> Self {
        Self(segment)
    }
}

/// The one place a `USegment` gives up its inner run without zeroing.
///
/// `pub(crate)` because the only sound caller is a mapping operation, which
/// hands the frames to an address space and keeps no handle — the frames do
/// not come back to the kernel, so there is nothing to clear yet.
impl USegment {
    pub(crate) fn into_inner(self) -> Segment {
        self.0
    }
}

/// The base of the first region satisfying `wanted` and at least `len` long.
///
/// A query rather than a closure over the map. `with_memory_map` hands a
/// caller `&MemoryMap` under a leaf guard, whose obligation is that *nothing
/// is called while it is live* — and a closure is a call by construction. A
/// `fn` pointer over one region is narrow enough to audit: it cannot capture,
/// so it cannot reach a lock a caller happened to be holding (D-105, D-121).
pub fn first_region(wanted: fn(&MemoryRegion) -> bool, len: u64) -> Option<Paddr> {
    let guard = MAP.lock_leaf();
    let map = guard.as_ref()?;
    map.regions()
        .find(|region| wanted(region) && region.len >= len)
        .map(|region| region.base)
}

/// The first page-aligned address past everything the map describes.
///
/// `None` before the map is up, and `None` where the top of the highest region
/// cannot be rounded up — the input is a `saturating_add` of the bootloader's
/// own numbers, and `next_multiple_of` panics a debug build on `u64::MAX`.
pub fn past_every_region() -> Option<Paddr> {
    let guard = MAP.lock_leaf();
    let map = guard.as_ref()?;
    map.regions()
        .map(|region| region.base.saturating_add(region.len))
        .max()?
        .checked_next_multiple_of(FRAME_SIZE)
}

/// One stretch of physical memory the direct map covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    /// First byte, rounded down to a frame.
    pub base: Paddr,
    /// One past the last, rounded up to a frame.
    pub end: Paddr,
    /// What it is for, which decides what protection it wants.
    pub kind: MemoryKind,
}

/// The most direct-mapped extents this will report.
pub const MAX_EXTENTS: usize = 64;

/// Every stretch the direct map covers, copied out.
///
/// **Copied, so nothing runs under the guard.** The caller of this walks page
/// tables for each extent, which is a second structure with its own reachable
/// locks — and `with_memory_map` used to hand it `&MemoryMap` and let it do
/// that *while holding* the map's leaf guard, whose whole obligation is that
/// nothing is called while it is live (D-105, D-121).
///
/// Sixty-four extents of thirty-two bytes is half a page of stack,
/// which the one caller — a boot-time walk on a thirty-two kilobyte stack —
/// has room for. Past the ceiling the count reports more than were returned,
/// so a caller can refuse rather than silently walk part of memory.
///
/// Returns the extents and how many the map actually had.
pub fn direct_mapped_extents() -> ([Extent; MAX_EXTENTS], usize) {
    const NOTHING: Extent = Extent {
        base: 0,
        end: 0,
        kind: MemoryKind::Reserved,
    };

    let mut extents = [NOTHING; MAX_EXTENTS];
    let mut found = 0;
    let guard = MAP.lock_leaf();
    let Some(map) = guard.as_ref() else {
        return (extents, 0);
    };
    for region in map
        .regions()
        .filter(|region| region.kind.is_direct_mapped())
    {
        if let Some(slot) = extents.get_mut(found) {
            *slot = Extent {
                base: region.base & !(FRAME_SIZE - 1),
                end: region
                    .base
                    .saturating_add(region.len)
                    // Checked, not `next_multiple_of`: the input is a
                    // `saturating_add` of the bootloader's own numbers, and
                    // rounding a value near `u64::MAX` up panics with overflow
                    // checks on — which the debug boot has. In release it
                    // wraps to zero and the region is silently left out, so
                    // the two builds disagree about what is mapped (D-124).
                    .checked_next_multiple_of(FRAME_SIZE)
                    .unwrap_or(u64::MAX),
                kind: region.kind,
            };
        }
        found += 1;
    }
    (extents, found)
}

/// Allocate one frame, with unspecified contents.
pub fn allocate() -> Option<Frame> {
    let index = FRAMES.lock_leaf().allocate()?;
    Some(Frame { index })
}

/// Allocate `count` consecutive frames, the first aligned to `align` frames.
///
/// `align` is rounded up to a power of two. A 2 MiB page wants
/// `allocate_contiguous(512, 512)`.
pub fn allocate_contiguous(count: usize, align: usize) -> Option<Segment> {
    let first = FRAMES.lock_leaf().allocate_contiguous(count, align)?;
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
    direct_pointer(physical, FRAME_SIZE)
}

/// Where the direct map makes `len` bytes at `physical` accessible.
///
/// Arithmetic only. Whether the map actually *reaches* those bytes is the
/// caller's to establish: [`FirmwareMemory`] checks the memory map, and a
/// [`Frame`] carries it by construction.
pub(crate) fn direct_pointer(physical: Paddr, len: u64) -> Option<*mut u8> {
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
