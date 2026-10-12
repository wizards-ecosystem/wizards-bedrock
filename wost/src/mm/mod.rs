//! Physical memory: which frames exist, and who owns each one.
//!
//! A [`Frame`] owns one allocator entry until drop; `frame_space::FrameSpace`
//! supplies the host-tested search. Frames are reached through the direct map
//! whose offset [`init`] records.
//!
//! [`USegment`] marks bytes as externally modifiable. Such bytes cannot become
//! kernel-owned again until [`USegment::reclaim`] clears them (D-44, D-107).

pub mod heap;

use alloc::sync::Arc;
use core::fmt;
use core::marker::PhantomData;
use core::mem::{align_of, size_of};
use core::sync::atomic::{AtomicU64, Ordering};

use frame_space::{FRAME_SIZE, FrameSpace, words_for};
use mmio_range::AccessRange;
use ring_proto::SharedLayout;

use crate::arch::current::{cpu, shared_memory};
#[cfg(feature = "selftest")]
use crate::boot::MemoryKind;
use crate::boot::{DirectMap, MemoryMap, MemoryRegion};
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
}

impl Report {
    /// Bytes under management.
    const fn managed_bytes(&self) -> u64 {
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
        Ok(())
    }
}

/// Take every exclusively allocatable whole-frame run under management.
///
/// The map arbitrates overlaps first, excluding every frame touched by a
/// kernel, boot, firmware, or other non-allocatable claim.
///
/// # Panics
///
/// Panics on a duplicate initialization attempt.
pub fn init(map: &MemoryMap, direct: DirectMap) -> Report {
    assert!(INIT.claim(), "physical memory initialized twice");

    DIRECT_MAP_OFFSET.store(direct.offset(), Ordering::Release);
    *MAP.lock_leaf() = Some(*map);

    let mut frames = FRAMES.lock_leaf();
    let ignored = seed_frames(&mut frames, map);
    let managed = frames.managed_count();
    drop(frames);

    Report { managed, ignored }
}

fn seed_frames<const WORDS: usize>(frames: &mut FrameSpace<WORDS>, map: &MemoryMap) -> usize {
    let mut ignored = 0;
    for run in map.allocatable_runs() {
        ignored += frames.insert(run.base(), run.bytes()).ignored;
    }
    ignored
}

#[cfg(feature = "selftest")]
/// Check that a reserved overlap is never seeded into the frame allocator.
pub fn overlapping_allocator_claims_are_arbitrated_for_test() -> bool {
    let map = match MemoryMap::try_from_regions([
        MemoryRegion {
            base: FRAME_SIZE,
            len: FRAME_SIZE * 4,
            kind: MemoryKind::Usable,
        },
        MemoryRegion {
            base: FRAME_SIZE * 2,
            len: FRAME_SIZE,
            kind: MemoryKind::Reserved,
        },
    ]) {
        Ok(map) => map,
        Err(_) => return false,
    };
    let mut frames = FrameSpace::<1>::new();
    let ignored = seed_frames(&mut frames, &map);
    let first = frames.allocate();
    let second = frames.allocate();
    let third = frames.allocate();
    ignored == 0
        && frames.managed_count() == 3
        && [first, second, third] == [Some(1), Some(3), Some(4)]
        && frames.allocate().is_none()
        && !frames.is_managed(2)
}

/// Frames available to allocate.
#[cfg(feature = "selftest")]
pub fn free_frames() -> usize {
    FRAMES.lock_leaf().free_count()
}

/// Allocation-free read-only access to physical firmware tables at handoff.
///
/// This deliberately cannot read usable, bootloader, kernel, reserved, or
/// device memory. It borrows the copied boot map directly so the platform
/// contract can be checked before the allocator records global memory state.
#[derive(Debug, Clone, Copy)]
pub struct EarlyFirmwareMemory<'map> {
    map: &'map MemoryMap,
    direct: DirectMap,
}

impl<'map> EarlyFirmwareMemory<'map> {
    /// Bind the complete copied memory map to its bootloader direct map.
    pub const fn new(map: &'map MemoryMap, direct: DirectMap) -> Self {
        Self { map, direct }
    }

    /// Whether every byte is described exclusively as ACPI table storage.
    pub fn contains(&self, address: Paddr, len: usize) -> bool {
        firmware_contains(self.map, address, len)
    }

    /// Copy table bytes at `address` into `buffer` with volatile reads.
    #[must_use = "a refused early firmware read copied no bytes"]
    pub fn read(&self, address: Paddr, buffer: &mut [u8]) -> bool {
        if !self.contains(address, buffer.len()) {
            return false;
        }
        let Ok(len) = u64::try_from(buffer.len()) else {
            return false;
        };
        let Some(end) = address.checked_add(len) else {
            return false;
        };
        if self.direct.virtual_address(end).is_none() {
            return false;
        }
        let Some(base) = self.direct.virtual_address(address) else {
            return false;
        };
        read_firmware_bytes(base as *const u8, buffer);
        true
    }
}

/// Read-only access to physical firmware tables after memory initialization.
///
/// This carries the same narrow authority as [`EarlyFirmwareMemory`] without
/// borrowing the boot handoff after the global map has accepted it.
#[derive(Debug, Clone, Copy)]
pub struct FirmwareMemory(());

impl FirmwareMemory {
    /// Whether every byte is described exclusively as ACPI table storage.
    pub fn contains(&self, address: Paddr, len: usize) -> bool {
        let guard = MAP.lock_leaf();
        let Some(map) = guard.as_ref() else {
            return false;
        };
        firmware_contains(map, address, len)
    }

    /// Copy table bytes at `address` into `buffer` with volatile reads.
    #[must_use = "a refused firmware read copied no bytes"]
    pub fn read(&self, address: Paddr, buffer: &mut [u8]) -> bool {
        if !self.contains(address, buffer.len()) {
            return false;
        }
        let Some(base) = direct_pointer(address, buffer.len() as u64) else {
            return false;
        };
        read_firmware_bytes(base, buffer);
        true
    }
}

fn firmware_contains(map: &MemoryMap, address: Paddr, len: usize) -> bool {
    let Ok(len) = u64::try_from(len) else {
        return false;
    };
    if len == 0 {
        return false;
    }
    let Some(end) = address.checked_add(len) else {
        return false;
    };
    let overlaps = |region: &MemoryRegion| address < region.base + region.len && region.base < end;
    map.covers(address, end)
        && map
            .regions()
            .filter(overlaps)
            .all(|region| region.kind.is_firmware_table())
}

fn read_firmware_bytes(base: *const u8, buffer: &mut [u8]) {
    for (index, slot) in buffer.iter_mut().enumerate() {
        let byte = base.wrapping_add(index);
        // SAFETY: the only callers first proved every byte immutable ACPI
        // storage and proved the direct-map address range cannot overflow.
        *slot = unsafe { byte.read_volatile() };
    }
}

/// Acquire the narrow reader after the memory map and direct map exist.
pub fn firmware_memory() -> Option<FirmwareMemory> {
    (MAP.lock_leaf().is_some() && DIRECT_MAP_OFFSET.load(Ordering::Acquire) != 0)
        .then_some(FirmwareMemory(()))
}

/// Whether `start..end` is disjoint from every RAM or firmware allocation.
pub(crate) fn device_range_is_eligible(start: Paddr, end: Paddr) -> bool {
    let guard = MAP.lock_leaf();
    let Some(map) = guard.as_ref() else {
        return false;
    };
    map.allows_device_mapping(start, end)
}

/// The memory map [`init`] was given, for ownership and firmware checks.
static MAP: SpinLock<Option<MemoryMap>> = SpinLock::new(None);

/// The one cache type the direct map gives `start..end`, if it has one.
pub(crate) fn map_direct_cache(start: Paddr, end: Paddr) -> Option<crate::boot::DirectMapCache> {
    MAP.lock_leaf()
        .as_ref()
        .and_then(|map| map.direct_map_cache(start, end))
}

/// Where the boot map covers `len` bytes with memory, or `None` before it is up.
///
/// See [`crate::boot::MemoryMap::first_memory_run`] for why a caller asks
/// rather than names an address (D-177).
#[cfg(feature = "selftest")]
pub fn first_memory_run(len: u64) -> Option<Paddr> {
    MAP.lock_leaf()
        .as_ref()
        .and_then(|map| map.first_memory_run(len))
}

/// The direct map [`init`] recorded, or `None` before it has run.
///
/// Lets a physical-address owner name the same byte for permission readback.
pub fn direct_map() -> Option<DirectMap> {
    match DIRECT_MAP_OFFSET.load(Ordering::Acquire) {
        0 => None,
        offset => Some(DirectMap::new(offset)),
    }
}

/// Reclaim the frame allocator's lock on the terminal panic path.
///
/// A panic can interrupt either lock's holder, so reporting must recover them
/// before it reads allocator state.
pub(crate) fn force_unlock(panic: &PanicToken) {
    FRAMES.force_unlock(panic);
    MAP.force_unlock(panic);
}

/// One physical frame, owned for as long as this value lives.
///
/// It cannot be copied or safely constructed except by allocation, so mutable
/// access through the direct map has one owner.
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
    #[must_use = "a refused frame fill changed no bytes"]
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
    #[must_use = "a refused frame clear changed no bytes"]
    pub fn zero(&mut self) -> bool {
        self.fill(0)
    }

    /// Whether every byte of the frame is zero, or `None` if the direct map
    /// cannot reach it.
    ///
    /// This is the machine witness for the D-12 zero-on-allocation boundary.
    #[cfg(feature = "selftest")]
    pub fn is_zero(&self) -> Option<bool> {
        const WORDS: usize = FRAME_SIZE as usize / size_of::<u64>();

        let base = direct_map_pointer(self.address())?.cast::<u64>();
        let mut zero = true;
        for index in 0..WORDS {
            let word = base.wrapping_add(index);
            // SAFETY: the frame `self` owns is allocatable memory and so is
            // direct-mapped whichever map is in force, and `index` stays below
            // the number of words in one frame. The direct map preserves the
            // frame's `u64` alignment.
            zero &= unsafe { word.read_volatile() } == 0;
        }
        Some(zero)
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        let released = FRAMES.lock_leaf().free(self.index);
        assert!(
            released.is_ok(),
            "frame ownership ledger refused a live handle: {released:?}"
        );
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
    #[must_use = "a refused segment fill changed no bytes"]
    pub fn fill(&mut self, byte: u8) -> bool {
        let Some(base) = direct_pointer(self.address(), self.len()) else {
            return false;
        };
        // SAFETY: a live `Segment` proves the allocator admitted and reserved
        // every frame in this run; allocator-managed frames are direct-mapped.
        // `direct_pointer` separately proved the address arithmetic.
        unsafe { core::ptr::write_bytes(base, byte, self.len() as usize) };
        true
    }

    /// Fill the run with zeros.
    #[must_use = "a refused segment clear changed no bytes"]
    pub fn zero(&mut self) -> bool {
        #[cfg(feature = "selftest")]
        if refuse_external_zero_for_test(self.address()) {
            return false;
        }
        self.fill(0)
    }

    /// Write `bytes` at `offset` within the run.
    ///
    /// This is the safe loading path: bytes enter before pages become
    /// read-only and executable (D-13, D-96).
    ///
    /// `false` if the write would run past the run, or if the direct map
    /// cannot reach it.
    #[must_use = "a refused segment write changed no bytes"]
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
        let destination = base.wrapping_add(offset);
        // SAFETY: the checked write lies in this uniquely owned allocator run,
        // which is direct-mapped, and the caller's bytes cannot overlap it.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
        true
    }

    /// Read `into.len()` bytes from `offset` within the run.
    ///
    /// [`SharedBytes`] uses volatile copies and [`SharedRing`] reserves its
    /// publication words for the architecture boundary. [`USegment`] exposes
    /// no reader (D-107, D-221).
    ///
    /// `false` if the read would run past the run, or if the direct map cannot
    /// reach it, having written nothing into `into`.
    #[cfg(feature = "selftest")]
    #[must_use = "a refused segment read copied no bytes"]
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
        let source = base.wrapping_add(offset);
        // SAFETY: the checked read lies in this owned allocator run, which is
        // direct-mapped, and the caller's output cannot overlap it.
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
        #[cfg(feature = "selftest")]
        observe_external_run_release_for_test(self.address());
        let released = FRAMES.lock_leaf().free_contiguous(self.first, self.count);
        assert!(
            released.is_ok(),
            "segment ownership ledger refused a live handle: {released:?}"
        );
    }
}

struct DirectSpan<'authority> {
    base: *mut u8,
    len: usize,
    _authority: PhantomData<&'authority ()>,
}

impl<'authority> DirectSpan<'authority> {
    fn from_segment(segment: &'authority Segment) -> Option<Self> {
        // SAFETY: a live `Segment` is the allocator's unique handle for every
        // frame in this run and keeps those direct-mapped bytes allocated.
        unsafe { Self::from_proven_run(segment.address(), segment.len()) }
    }

    fn from_shared(allocation: &'authority SharedAllocation) -> Option<Self> {
        // SAFETY: `SharedAllocation` retains its segment for this borrow and
        // permits byte access specifically through volatile operations.
        unsafe { Self::from_proven_run(allocation.0.address(), allocation.0.len()) }
    }

    unsafe fn from_proven_run(physical: Paddr, len: u64) -> Option<Self> {
        let len = usize::try_from(len).ok()?;
        let base = direct_pointer(physical, len as u64)?;
        Some(Self {
            base,
            len,
            _authority: PhantomData,
        })
    }

    fn write_byte_volatile(&self, offset: usize, byte: u8) -> bool {
        if offset >= self.len {
            return false;
        }
        let destination = self.base.wrapping_add(offset);
        // SAFETY: the span's authority keeps the run live, and the checked
        // offset names exactly one byte within it.
        unsafe { destination.write_volatile(byte) };
        true
    }

    fn read_byte_volatile(&self, offset: usize) -> Option<u8> {
        if offset >= self.len {
            return None;
        }
        let source = self.base.wrapping_add(offset);
        // SAFETY: the span's authority keeps the run live, and the checked
        // offset names exactly one byte within it.
        Some(unsafe { source.read_volatile() })
    }

    fn fill_volatile(&self, byte: u8) -> bool {
        let Ok(range) = AccessRange::repeated(0, self.len, 1, self.len) else {
            return false;
        };
        range
            .offsets()
            .all(|offset| self.write_byte_volatile(offset, byte))
    }

    fn write_volatile_at(&self, offset: usize, bytes: &[u8]) -> bool {
        let Ok(range) = AccessRange::repeated(offset, bytes.len(), 1, self.len) else {
            return false;
        };
        range
            .offsets()
            .zip(bytes.iter().copied())
            .all(|(offset, byte)| self.write_byte_volatile(offset, byte))
    }

    fn write_u64_volatile_at(&self, offset: usize, value: u64) -> bool {
        let Some(end) = offset.checked_add(core::mem::size_of::<u64>()) else {
            return false;
        };
        let destination = self.base.wrapping_add(offset);
        if end > self.len || !(destination as usize).is_multiple_of(core::mem::align_of::<u64>()) {
            return false;
        }
        // SAFETY: the span's authority keeps all eight checked bytes live, and
        // the alignment check makes this one indivisible x86-64 qword store.
        unsafe { destination.cast::<u64>().write_volatile(value.to_le()) };
        true
    }

    fn read_volatile_at(&self, offset: usize, into: &mut [u8]) -> bool {
        let Ok(range) = AccessRange::repeated(offset, into.len(), 1, self.len) else {
            return false;
        };
        for (offset, byte) in range.offsets().zip(into) {
            let Some(value) = self.read_byte_volatile(offset) else {
                return false;
            };
            *byte = value;
        }
        true
    }

    /// Push the cache line holding `offset` to memory for a non-snooping reader.
    fn flush_line_at(&self, offset: usize) -> bool {
        if offset >= self.len {
            return false;
        }
        let line = self.base.wrapping_add(offset);
        // SAFETY: the span's authority keeps the run live in the direct map, and
        // the checked offset names a byte within it, so the line is mapped.
        unsafe { cpu::flush_cache_line(line) };
        true
    }

    /// Push every line of the span to memory, ordered before whatever follows.
    fn flush_all(&self) -> bool {
        let flushed = (0..self.len)
            .step_by(CACHE_LINE)
            .all(|offset| self.flush_line_at(offset));
        core::sync::atomic::fence(Ordering::SeqCst);
        flushed
    }

    #[cfg(feature = "selftest")]
    fn all_bytes_equal(&self, byte: u8) -> bool {
        let Ok(range) = AccessRange::repeated(0, self.len, 1, self.len) else {
            return false;
        };
        range
            .offsets()
            .all(|offset| self.read_byte_volatile(offset) == Some(byte))
    }

    #[cfg(feature = "selftest")]
    fn starts_with(&self, expected: &[u8]) -> bool {
        let Ok(range) = AccessRange::repeated(0, expected.len(), 1, self.len) else {
            return false;
        };
        range
            .offsets()
            .zip(expected)
            .all(|(offset, wanted)| self.read_byte_volatile(offset) == Some(*wanted))
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
    #[must_use = "a refused DMA-segment fill changed no bytes"]
    pub fn fill(&mut self, byte: u8) -> bool {
        let Some(segment) = self.segment.as_mut() else {
            return false;
        };
        let Some(span) = DirectSpan::from_segment(segment) else {
            return false;
        };
        span.fill_volatile(byte)
    }

    /// Fill every available byte with zeros.
    #[must_use = "a refused DMA-segment clear changed no bytes"]
    pub fn zero(&mut self) -> bool {
        self.fill(0)
    }

    /// Write `bytes` at `offset` while the run is host-owned.
    #[must_use = "a refused DMA-segment write changed no bytes"]
    pub fn write_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        let Some(segment) = self.segment.as_mut() else {
            return false;
        };
        let Some(span) = DirectSpan::from_segment(segment) else {
            return false;
        };
        span.write_volatile_at(offset, bytes)
    }

    /// Read bytes while the run is host-owned.
    #[must_use = "a refused DMA-segment read copied no bytes"]
    pub fn read_at(&self, offset: usize, into: &mut [u8]) -> bool {
        let Some(segment) = self.segment.as_ref() else {
            return false;
        };
        let Some(span) = DirectSpan::from_segment(segment) else {
            return false;
        };
        span.read_volatile_at(offset, into)
    }

    /// Order earlier volatile host writes before an MMIO submission doorbell.
    pub fn publish_to_device() {
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);
    }

    /// Order later volatile completion and payload reads after phase ownership.
    pub fn observe_device() {
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Acquire);
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

/// Bytes in one processor cache line, the granule `clflush` moves.
///
/// Every x86-64 processor reports 64, and the flush walks a table at this
/// stride; a smaller line would be flushed in full by the same walk.
const CACHE_LINE: usize = 64;

/// How an IOMMU reads the tables the CPU writes for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableWalks {
    /// The unit snoops the processor caches, so a volatile write is visible to
    /// it as soon as it lands.
    Coherent,
    /// The unit reads memory directly, so every written line is flushed out of
    /// the caches before the unit may depend on it.
    Uncached,
}

/// A run of kernel-owned frames an IOMMU walks: a root, context, or
/// second-level table.
///
/// The CPU writes it through the direct map and the unit reads it at the
/// physical address, so it is never mapped into any device's translation and
/// a device cannot reach the structure that constrains it. Its frames free on
/// drop with the rest of the domain (D-357).
#[derive(Debug)]
pub(crate) struct IommuTable {
    frames: Segment,
    walks: TableWalks,
}

impl IommuTable {
    /// Allocate `frames` contiguous zeroed frames for one structure or arena,
    /// with the zeros already visible to a unit that walks `walks`.
    pub(crate) fn new(frames: usize, walks: TableWalks) -> Option<Self> {
        let mut segment = allocate_contiguous(frames, 1)?;
        if !segment.fill(0) {
            return None;
        }
        let table = Self {
            frames: segment,
            walks,
        };
        let published = match walks {
            TableWalks::Coherent => true,
            TableWalks::Uncached => {
                DirectSpan::from_segment(&table.frames).is_some_and(|span| span.flush_all())
            }
        };
        published.then_some(table)
    }

    /// The physical address programmed into the unit or a parent entry.
    pub(crate) const fn address(&self) -> Paddr {
        self.frames.address()
    }

    /// Overwrite the eight-byte entry at `index`, visibly to the unit.
    #[must_use = "a refused IOMMU-table write changed no bytes"]
    pub(crate) fn write_u64(&mut self, index: usize, value: u64) -> bool {
        let walks = self.walks;
        let Some(span) = DirectSpan::from_segment(&self.frames) else {
            return false;
        };
        let Some(offset) = index.checked_mul(core::mem::size_of::<u64>()) else {
            return false;
        };
        if !span.write_u64_volatile_at(offset, value) {
            return false;
        }
        match walks {
            TableWalks::Coherent => true,
            TableWalks::Uncached => {
                let flushed = span.flush_line_at(offset);
                core::sync::atomic::fence(Ordering::SeqCst);
                flushed
            }
        }
    }

    /// Read the eight-byte entry at `index`.
    pub(crate) fn read_u64(&self, index: usize) -> Option<u64> {
        let span = DirectSpan::from_segment(&self.frames)?;
        let mut bytes = [0u8; 8];
        span.read_volatile_at(index * 8, &mut bytes)
            .then(|| u64::from_le_bytes(bytes))
    }
}

/// A run of frames whose bytes are ring 3's.
///
/// Its contents are outside kernel control and cannot be read as trusted data.
/// Conversion is one-way except through [`reclaim`](Self::reclaim), which
/// clears every byte before returning kernel ownership (D-44, D-107).
#[derive(Debug, PartialEq, Eq)]
pub struct USegment {
    segment: Option<Segment>,
}

impl USegment {
    fn segment(&self) -> &Segment {
        let Some(segment) = self.segment.as_ref() else {
            panic!("an external segment no longer owns its frames");
        };
        segment
    }

    /// Where the run begins in physical memory.
    pub fn address(&self) -> Paddr {
        self.segment().address()
    }

    /// Frames in the run.
    pub(crate) fn frames(&self) -> usize {
        self.segment().frames()
    }

    /// Bytes in the run.
    pub(crate) fn len(&self) -> u64 {
        self.segment().len()
    }

    /// Pin this run for sharing, giving up the right to map it as owned.
    ///
    /// Consuming, with no conversion back. `map_shared` writes no `OWNED` bit,
    /// while `map_user` consumes a `USegment` and writes one, so one run cannot
    /// enter both ownership paths (D-115).
    #[must_use]
    pub fn share(self) -> Shared {
        Shared(Arc::new(SharedAllocation(self)))
    }

    /// Take the run back for the kernel, with every byte cleared.
    ///
    /// `None` if clearing cannot reach the run. Such frames are withheld rather
    /// than recycled with ring-3 bytes still present (D-12, D-107).
    pub fn reclaim(mut self) -> Option<Segment> {
        let Some(mut segment) = self.segment.take() else {
            panic!("an external segment no longer owns its frames");
        };
        if segment.zero() {
            return Some(segment);
        }
        // Returning a failed clear would expose the previous external owner's
        // bytes through the allocator's next handle (D-115).
        core::mem::forget(segment);
        None
    }

    /// Commit this run to page-table ownership after the mapping exists.
    pub(crate) fn commit_mapping(mut self) {
        let Some(segment) = self.segment.take() else {
            panic!("an external segment no longer owns its frames");
        };
        core::mem::forget(segment);
    }
}

impl Drop for USegment {
    fn drop(&mut self) {
        let Some(mut segment) = self.segment.take() else {
            return;
        };
        if !segment.zero() {
            #[cfg(feature = "selftest")]
            let address = segment.address();
            core::mem::forget(segment);
            #[cfg(feature = "selftest")]
            observe_external_run_withheld_for_test(address);
        }
    }
}

#[derive(Debug)]
struct SharedAllocation(USegment);

/// A non-accessing ownership reference retained by a shared mapping.
#[derive(Debug)]
pub(crate) struct SharedKeepalive {
    _allocation: Arc<SharedAllocation>,
}

/// A pinned shared allocation awaiting an irreversible access partition.
#[derive(Debug)]
pub struct Shared(Arc<SharedAllocation>);

impl Shared {
    /// Consume the allocation into volatile byte-only access.
    pub fn into_bytes(self) -> SharedBytes {
        SharedBytes(self.0)
    }

    /// Consume the allocation into one checked ring partition.
    ///
    /// Returns the unchanged owner if the layout describes a different number
    /// of bytes.
    pub fn into_ring<const WORDS: usize>(
        self,
        layout: SharedLayout<WORDS>,
    ) -> Result<SharedRing<WORDS>, Self> {
        if usize::try_from(self.0.0.len()).ok() != Some(layout.bytes()) {
            return Err(self);
        }
        Ok(SharedRing {
            allocation: self.0,
            layout,
        })
    }
}

/// A pinned allocation with volatile byte access and no publication-word door.
#[derive(Debug)]
pub struct SharedBytes(Arc<SharedAllocation>);

impl SharedBytes {
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
    #[must_use = "a refused shared-memory write changed no bytes"]
    pub fn write_volatile_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        write_shared_volatile(&self.0, offset, bytes)
    }

    /// Volatile-read `into.len()` bytes from `offset` within the run.
    #[must_use = "a refused shared-memory read copied no bytes"]
    pub fn read_volatile_at(&self, offset: usize, into: &mut [u8]) -> bool {
        read_shared_volatile(&self.0, offset, into)
    }

    /// Retain the allocation without exposing its bytes.
    pub(crate) fn keepalive(&self) -> SharedKeepalive {
        SharedKeepalive {
            _allocation: Arc::clone(&self.0),
        }
    }
}

/// A pinned allocation partitioned into payload bytes and publication words.
#[derive(Debug)]
pub struct SharedRing<const WORDS: usize> {
    allocation: Arc<SharedAllocation>,
    layout: SharedLayout<WORDS>,
}

impl<const WORDS: usize> SharedRing<WORDS> {
    /// Where the run begins in physical memory.
    pub fn address(&self) -> Paddr {
        self.allocation.0.address()
    }

    /// Bytes in the run.
    #[expect(
        clippy::len_without_is_empty,
        reason = "shared allocations cannot contain zero frames"
    )]
    pub fn len(&self) -> u64 {
        self.allocation.0.len()
    }

    /// Volatile-write bytes wholly within this layout's payload partition.
    #[must_use = "a refused ring-payload write changed no bytes"]
    pub fn write_payload_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        if !self.layout.contains_payload(offset, bytes.len()) {
            return false;
        }
        write_shared_volatile(&self.allocation, offset, bytes)
    }

    /// Volatile-read bytes wholly within this layout's payload partition.
    #[must_use = "a refused ring-payload read copied no bytes"]
    pub fn read_payload_at(&self, offset: usize, into: &mut [u8]) -> bool {
        if !self.layout.contains_payload(offset, into.len()) {
            return false;
        }
        read_shared_volatile(&self.allocation, offset, into)
    }

    /// Acquire-load one aligned little-endian publication index.
    ///
    /// `None` means the offset is not one of this layout's publication words
    /// or the word is not reachable through the direct map. An acquired
    /// producer tail makes its earlier volatile payload writes visible; an
    /// acquired consumer head makes its earlier payload reads precede reuse.
    pub fn load_acquire_u32(&self, offset: usize) -> Option<u32> {
        if !self.layout.contains_publication_word(offset) {
            return None;
        }
        let address = self.publication_address(offset)?;
        // SAFETY: `publication_address` proves this exact word stays mapped
        // and aligned for this borrow; `SharedAllocation` keeps its bytes
        // external rather than representing them as a Rust object.
        let value = unsafe { shared_memory::load_acquire_u32(address) };
        Some(u32::from_le(value))
    }

    /// Release-store one aligned little-endian publication index.
    ///
    /// `false` means the offset is not one of this layout's publication words
    /// or the word is not reachable through the direct map. A producer
    /// publishes completed volatile payload writes through a tail; a consumer
    /// publishes completed payload reads before the producer may reuse them.
    #[must_use = "a refused shared-memory publication stored no index"]
    pub fn store_release_u32(&self, offset: usize, value: u32) -> bool {
        if !self.layout.contains_publication_word(offset) {
            return false;
        }
        let Some(address) = self.publication_address(offset) else {
            return false;
        };
        // SAFETY: `publication_address` proves this exact word stays mapped
        // and aligned for this borrow; `SharedAllocation` keeps its bytes
        // external rather than representing them as a Rust object.
        unsafe { shared_memory::store_release_u32(address, value.to_le()) };
        true
    }

    /// The aligned publication address at `offset`, if wholly inside the run.
    fn publication_address(&self, offset: usize) -> Option<usize> {
        let end = offset.checked_add(size_of::<u32>())?;
        if end as u64 > self.len() {
            return None;
        }
        let base = direct_pointer(self.address(), self.len())?;
        let address = base.addr().checked_add(offset)?;
        if !address.is_multiple_of(align_of::<u32>()) {
            return None;
        }
        Some(address)
    }

    /// Retain the allocation without exposing its bytes.
    pub(crate) fn keepalive(&self) -> SharedKeepalive {
        SharedKeepalive {
            _allocation: Arc::clone(&self.allocation),
        }
    }
}

fn write_shared_volatile(allocation: &SharedAllocation, offset: usize, bytes: &[u8]) -> bool {
    let Some(span) = DirectSpan::from_shared(allocation) else {
        return false;
    };
    span.write_volatile_at(offset, bytes)
}

fn read_shared_volatile(allocation: &SharedAllocation, offset: usize, into: &mut [u8]) -> bool {
    let Some(span) = DirectSpan::from_shared(allocation) else {
        return false;
    };
    span.read_volatile_at(offset, into)
}

impl From<Segment> for USegment {
    fn from(segment: Segment) -> Self {
        Self {
            segment: Some(segment),
        }
    }
}

/// Copy the boot map so policy queries run after its leaf guard is released.
pub(crate) fn memory_map_snapshot() -> Option<MemoryMap> {
    *MAP.lock_leaf()
}

/// A copy of the boot memory map for machine checks.
#[cfg(feature = "selftest")]
pub fn memory_map_for_test() -> Option<MemoryMap> {
    memory_map_snapshot()
}

#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_INACTIVE: u64 = u64::MAX;
#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_ARMED: u64 = 1;
#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_REFUSED: u64 = 2;
#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_WITHHELD: u64 = 3;
#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_RELEASED: u64 = 4;
#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_PHASE_MASK: u64 = FRAME_SIZE - 1;

#[cfg(feature = "selftest")]
static EXTERNAL_ZERO_REFUSAL: AtomicU64 = AtomicU64::new(EXTERNAL_ZERO_INACTIVE);

#[cfg(feature = "selftest")]
fn external_zero_state(address: Paddr, phase: u64) -> Option<u64> {
    if address >= CEILING
        || !address.is_multiple_of(FRAME_SIZE)
        || phase == 0
        || phase > EXTERNAL_ZERO_PHASE_MASK
    {
        return None;
    }
    Some(address | phase)
}

#[cfg(feature = "selftest")]
fn refuse_external_zero_for_test(address: Paddr) -> bool {
    let (Some(armed), Some(refused)) = (
        external_zero_state(address, EXTERNAL_ZERO_ARMED),
        external_zero_state(address, EXTERNAL_ZERO_REFUSED),
    ) else {
        return false;
    };
    EXTERNAL_ZERO_REFUSAL
        .compare_exchange(armed, refused, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

#[cfg(feature = "selftest")]
fn observe_external_run_withheld_for_test(address: Paddr) {
    let (Some(refused), Some(withheld)) = (
        external_zero_state(address, EXTERNAL_ZERO_REFUSED),
        external_zero_state(address, EXTERNAL_ZERO_WITHHELD),
    ) else {
        return;
    };
    let _ = EXTERNAL_ZERO_REFUSAL.compare_exchange(
        refused,
        withheld,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

#[cfg(feature = "selftest")]
fn observe_external_run_release_for_test(address: Paddr) {
    let Some(released) = external_zero_state(address, EXTERNAL_ZERO_RELEASED) else {
        return;
    };
    loop {
        let current = EXTERNAL_ZERO_REFUSAL.load(Ordering::Acquire);
        if current == EXTERNAL_ZERO_INACTIVE
            || current & !EXTERNAL_ZERO_PHASE_MASK != address
            || current & EXTERNAL_ZERO_PHASE_MASK == EXTERNAL_ZERO_RELEASED
        {
            return;
        }
        if EXTERNAL_ZERO_REFUSAL
            .compare_exchange(current, released, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return;
        }
    }
}

#[cfg(feature = "selftest")]
#[derive(Debug)]
struct ExternalZeroRefusal {
    address: Paddr,
    count: usize,
    active: bool,
}

#[cfg(feature = "selftest")]
impl ExternalZeroRefusal {
    fn arm(segment: &Segment) -> Option<Self> {
        let address = segment.address();
        let armed = external_zero_state(address, EXTERNAL_ZERO_ARMED)?;
        EXTERNAL_ZERO_REFUSAL
            .compare_exchange(
                EXTERNAL_ZERO_INACTIVE,
                armed,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()?;
        Some(Self {
            address,
            count: segment.frames(),
            active: true,
        })
    }

    fn state(&self, phase: u64) -> u64 {
        self.address | phase
    }

    fn is_withheld_with(&self, expected: &[u8]) -> bool {
        if EXTERNAL_ZERO_REFUSAL.load(Ordering::Acquire) != self.state(EXTERNAL_ZERO_WITHHELD) {
            return false;
        }
        let first = (self.address / FRAME_SIZE) as usize;
        let Some(end) = first.checked_add(self.count) else {
            return false;
        };
        let held = {
            let frames = FRAMES.lock_leaf();
            (first..end).all(|frame| frames.is_managed(frame) && !frames.is_free(frame))
        };
        if !held {
            return false;
        }
        let Some(len) = (self.count as u64).checked_mul(FRAME_SIZE) else {
            return false;
        };
        let Ok(expected_len) = u64::try_from(expected.len()) else {
            return false;
        };
        if expected_len > len {
            return false;
        }
        // SAFETY: the refusal state and allocator ledger above prove this exact
        // run is withheld and cannot be reused while the observation runs.
        let Some(span) = (unsafe { DirectSpan::from_proven_run(self.address, len) }) else {
            return false;
        };
        span.starts_with(expected)
    }

    fn recover(&mut self) -> bool {
        if !self.active {
            return false;
        }
        let current = EXTERNAL_ZERO_REFUSAL.load(Ordering::Acquire);
        if current == self.state(EXTERNAL_ZERO_RELEASED) {
            if EXTERNAL_ZERO_REFUSAL
                .compare_exchange(
                    current,
                    EXTERNAL_ZERO_INACTIVE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                self.active = false;
            }
            return false;
        }
        if current != self.state(EXTERNAL_ZERO_WITHHELD) {
            return false;
        }
        if EXTERNAL_ZERO_REFUSAL
            .compare_exchange(
                current,
                EXTERNAL_ZERO_INACTIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }

        // SAFETY: `WITHHELD` is published only after the sole `Segment` handle
        // is forgotten. `Segment::drop` publishes `RELEASED` instead, so this
        // state proves no allocator owner can name these exact frames.
        let mut segment = unsafe { Segment::from_raw(self.address, self.count) };
        if !segment.zero() {
            core::mem::forget(segment);
            EXTERNAL_ZERO_REFUSAL.store(current, Ordering::Release);
            return false;
        }
        drop(segment);
        self.active = false;
        true
    }
}

#[cfg(feature = "selftest")]
impl Drop for ExternalZeroRefusal {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let armed = self.state(EXTERNAL_ZERO_ARMED);
        if EXTERNAL_ZERO_REFUSAL
            .compare_exchange(
                armed,
                EXTERNAL_ZERO_INACTIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.active = false;
            return;
        }
        assert!(
            self.recover(),
            "a refused external clear could not recover its withheld run"
        );
    }
}

/// Force one external run's clear to refuse, then recover it after observation.
#[cfg(feature = "selftest")]
#[must_use = "the refusal proof reports whether the dirty run stayed withheld"]
pub fn refused_external_clear_withholds_for_test(segment: Segment, expected_prefix: &[u8]) -> bool {
    let address = segment.address();
    let count = segment.frames();
    let Some(mut refusal) = ExternalZeroRefusal::arm(&segment) else {
        return false;
    };
    drop(<USegment as From<Segment>>::from(segment));
    let withheld = refusal.is_withheld_with(expected_prefix);
    let recovered = refusal.recover();
    withheld && recovered && freed_external_run_is_zero_for_test(address, count)
}

/// Whether an exact released run is free and contains only zero bytes.
#[cfg(feature = "selftest")]
pub fn freed_external_run_is_zero_for_test(address: Paddr, count: usize) -> bool {
    if count == 0 || !address.is_multiple_of(FRAME_SIZE) {
        return false;
    }
    let Some(len) = (count as u64).checked_mul(FRAME_SIZE) else {
        return false;
    };
    let first = (address / FRAME_SIZE) as usize;
    let Some(end) = first.checked_add(count) else {
        return false;
    };
    let frames = FRAMES.lock_leaf();
    if !(first..end).all(|frame| frames.is_free(frame)) {
        return false;
    }
    // SAFETY: the allocator lock keeps this verified free run from being
    // allocated until the scan completes, and managed frames are direct-mapped.
    let Some(span) = (unsafe { DirectSpan::from_proven_run(address, len) }) else {
        return false;
    };
    span.all_bytes_equal(0)
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
/// Returns an unreachable frame to the allocator rather than exposing stale
/// contents through a new mapping.
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
/// [`Frame`] or [`Segment`] carries it by construction.
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
