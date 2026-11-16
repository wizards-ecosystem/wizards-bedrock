use core::fmt;
use core::mem::ManuallyDrop;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering, fence};

use ring_proto::SharedLayout;

use super::direct::DirectSpan;
use super::frame::{Segment, release_sealed_run, seal_run};
use super::pool::{ArenaFrame, FrameKind};
use super::{PhysicalAddress, direct_pointer};
use crate::arch::current::paging::{Flush, FrameRef};
use crate::arch::current::shared_memory;

#[derive(PartialEq, Eq, Debug)]
pub struct USegment {
    segment: Segment,
}

impl USegment {
    #[must_use = "a refused conversion frees the run"]
    pub fn zeroed(mut segment: Segment) -> Option<Self> {
        if !segment.zero() {
            return None;
        }
        Some(Self { segment })
    }

    #[must_use = "a refused segment write changed no bytes"]
    pub fn write_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        self.segment.write_at(offset, bytes)
    }

    pub fn address(&self) -> PhysicalAddress {
        self.segment.address()
    }

    pub(crate) fn frames(&self) -> usize {
        self.segment.frames()
    }

    pub(crate) fn len(&self) -> u64 {
        self.segment.len()
    }

    pub(crate) fn commit_mapping(self) {
        core::mem::forget(self);
    }

    // Consuming the run proves no writable mapping of it remains, since mapping consumes it too.
    pub fn seal(self, flush: &mut Flush) -> Sealing<SealedSegment> {
        let Self { segment } = self;
        let mut own = Flush::none();
        seal_run(&segment, &mut own);
        flush.merge(own.duplicate());
        Sealing::new(
            SealedSegment {
                segment: ManuallyDrop::new(segment),
            },
            own,
        )
    }
}

// Until its flush completes another processor may still write the frames, so it is only read.
#[derive(Debug)]
#[must_use = "a seal is usable only once its flush completes"]
pub struct Sealing<T> {
    sealed: T,
    flush: Flush,
}

impl<T> Sealing<T> {
    pub(super) const fn new(sealed: T, flush: Flush) -> Self {
        Self { sealed, flush }
    }

    pub const fn sealed(&self) -> &T {
        &self.sealed
    }

    pub fn finish(self) -> Result<T, Self> {
        if self.flush.is_complete() {
            Ok(self.sealed)
        } else {
            Err(self)
        }
    }
}

// Read-only in every mapping, the direct map included; there is no way back to writable bytes.
#[derive(PartialEq, Eq, Debug)]
pub struct SealedSegment {
    segment: ManuallyDrop<Segment>,
}

impl SealedSegment {
    pub fn address(&self) -> PhysicalAddress {
        self.segment.address()
    }

    pub(crate) fn frames(&self) -> usize {
        self.segment.frames()
    }

    pub(crate) fn len(&self) -> u64 {
        self.segment.len()
    }

    pub(crate) fn commit_mapping(self) {
        core::mem::forget(self);
    }
}

impl Drop for SealedSegment {
    fn drop(&mut self) {
        // SAFETY: this is the one move out of `segment`, and nothing reads the field after it.
        let segment = unsafe { ManuallyDrop::take(&mut self.segment) };
        let mut flush = Flush::none();
        release_sealed_run(segment, &mut flush);
        assert!(
            flush.is_complete(),
            "a sealed run's direct-map change awaits a flush its drop cannot wait for"
        );
    }
}

// The holder count and the shared frame's token, in an arena object frame that holds itself, so
// sharing a frame takes nothing from the kernel heap.
struct Home {
    holders: AtomicUsize,
    frame: ArenaFrame,
    page: ManuallyDrop<ArenaFrame>,
}

// Ring 3 may write these bytes through its mapping, so the kernel reaches them only by copies.
pub(super) struct SharedAllocation(NonNull<Home>);

// SAFETY: the home is reached only through shared references, its fields are atomics and frame
// tokens, and only the last holder moves it out.
unsafe impl Send for SharedAllocation {}
// SAFETY: as for `Send`: every shared access to the home is a read or an atomic.
unsafe impl Sync for SharedAllocation {}

impl SharedAllocation {
    fn new(frame: ArenaFrame, page: ArenaFrame) -> Self {
        assert!(
            page.kind() == FrameKind::ArenaObject,
            "a shared frame's home opened over a {:?} frame",
            page.kind()
        );
        let Some(home) = direct_pointer(page.address(), FRAME_BYTES)
            .and_then(|base| NonNull::new(base.cast::<Home>()))
        else {
            panic!("a shared frame's home lost its direct-map address");
        };
        // SAFETY: the page is a fresh object frame this token alone names, page aligned and larger
        // than `Home`, and nothing reads it before this write; the token moves into its own bytes.
        unsafe {
            home.write(Home {
                holders: AtomicUsize::new(1),
                frame,
                page: ManuallyDrop::new(page),
            });
        };
        Self(home)
    }

    fn home(&self) -> &Home {
        // SAFETY: this holder keeps the count above zero, so the home stays written and in place.
        unsafe { self.0.as_ref() }
    }

    fn another(&self) -> Self {
        let before = self.home().holders.fetch_add(1, Ordering::Relaxed);
        assert!(before < usize::MAX / 2, "a shared frame ran out of holders");
        Self(self.0)
    }

    pub(super) fn address(&self) -> PhysicalAddress {
        self.home().frame.address()
    }

    pub(super) const LEN: u64 = frame_space::FRAME_SIZE;
}

impl Drop for SharedAllocation {
    fn drop(&mut self) {
        if self.home().holders.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        // SAFETY: this was the last holder, so no other reference to the home remains, and this
        // read moves it out once; the home's own frame is then dropped last.
        let Home { frame, page, .. } = unsafe { self.0.as_ptr().read() };
        drop(frame);
        drop(ManuallyDrop::into_inner(page));
    }
}

impl fmt::Debug for SharedAllocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SharedAllocation")
            .field(&self.home().frame)
            .finish()
    }
}

const FRAME_BYTES: u64 = frame_space::FRAME_SIZE;

impl ArenaFrame {
    // Zeroed when taken, so ring 3 sees only zeros and what the kernel writes afterwards; `home`
    // is an object frame that keeps the count.
    #[must_use]
    pub fn share(self, home: Self) -> Shared {
        Shared(SharedAllocation::new(self, home))
    }
}

// Keeps a shared frame for a space that maps it, so the frame outlives every leaf naming it.
#[derive(Debug)]
pub struct SharedFrame(SharedAllocation);

impl SharedFrame {
    pub fn frame(&self) -> FrameRef<'_> {
        FrameRef::Writable(&self.0.home().frame)
    }
}

#[derive(Debug)]
pub struct Shared(SharedAllocation);

impl Shared {
    pub fn into_bytes(self) -> SharedBytes {
        SharedBytes(self.0)
    }

    pub fn into_ring<const WORDS: usize>(
        self,
        layout: SharedLayout<WORDS>,
    ) -> Result<SharedRing<WORDS>, Self> {
        if usize::try_from(SharedAllocation::LEN).ok() != Some(layout.bytes()) {
            return Err(self);
        }
        Ok(SharedRing {
            allocation: self.0,
            layout,
        })
    }
}

#[derive(Debug)]
pub struct SharedBytes(SharedAllocation);

impl SharedBytes {
    pub fn address(&self) -> PhysicalAddress {
        self.0.address()
    }

    #[expect(
        clippy::len_without_is_empty,
        reason = "shared allocations cannot contain zero frames"
    )]
    pub fn len(&self) -> u64 {
        SharedAllocation::LEN
    }

    #[must_use = "a refused shared-memory write changed no bytes"]
    pub fn write_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        write_shared(&self.0, offset, bytes)
    }

    #[must_use = "a refused shared-memory read copied no bytes"]
    pub fn read_at(&self, offset: usize, into: &mut [u8]) -> bool {
        read_shared(&self.0, offset, into)
    }

    pub fn frame(&self) -> SharedFrame {
        SharedFrame(self.0.another())
    }
}

#[derive(Debug)]
pub struct SharedRing<const WORDS: usize> {
    allocation: SharedAllocation,
    layout: SharedLayout<WORDS>,
}

impl<const WORDS: usize> SharedRing<WORDS> {
    pub fn address(&self) -> PhysicalAddress {
        self.allocation.address()
    }

    #[expect(
        clippy::len_without_is_empty,
        reason = "shared allocations cannot contain zero frames"
    )]
    pub fn len(&self) -> u64 {
        SharedAllocation::LEN
    }

    #[must_use = "a refused ring-payload write changed no bytes"]
    pub fn write_payload_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        if !self.layout.contains_payload(offset, bytes.len()) {
            return false;
        }
        write_shared(&self.allocation, offset, bytes)
    }

    #[must_use = "a refused ring-payload read copied no bytes"]
    pub fn read_payload_at(&self, offset: usize, into: &mut [u8]) -> bool {
        if !self.layout.contains_payload(offset, into.len()) {
            return false;
        }
        read_shared(&self.allocation, offset, into)
    }

    pub fn load_acquire_u32(&self, offset: usize) -> Option<u32> {
        if !self.layout.contains_publication_word(offset) {
            return None;
        }
        let address = self.publication_address(offset)?;
        // SAFETY: `publication_address` proves this exact word mapped and aligned for this borrow,
        // and `SharedAllocation` keeps its bytes external, never a Rust object.
        let value = unsafe { shared_memory::load_acquire_u32(address) };
        Some(u32::from_le(value))
    }

    #[must_use = "a refused shared-memory publication stored no index"]
    pub fn store_release_u32(&self, offset: usize, value: u32) -> bool {
        if !self.layout.contains_publication_word(offset) {
            return false;
        }
        let Some(address) = self.publication_address(offset) else {
            return false;
        };
        // SAFETY: `publication_address` proves this exact word mapped and aligned for this borrow,
        // and `SharedAllocation` keeps its bytes external, never a Rust object.
        unsafe { shared_memory::store_release_u32(address, value.to_le()) };
        true
    }

    // A pair of adjacent publication words, read as one aligned `u64`.
    pub fn load_acquire_u64(&self, offset: usize) -> Option<u64> {
        let address = self.publication_pair_address(offset)?;
        // SAFETY: `publication_pair_address` proves these eight bytes mapped and aligned for this
        // borrow, and `SharedAllocation` keeps its bytes external, never a Rust object.
        let value = unsafe { shared_memory::load_acquire_u64(address) };
        Some(u64::from_le(value))
    }

    // Raises the pair to at least `value` and returns what it holds afterwards; it never lowers it.
    pub fn raise_u64(&self, offset: usize, value: u64) -> Option<u64> {
        let address = self.publication_pair_address(offset)?;
        // SAFETY: as in `load_acquire_u64`; the bytes stay mapped for this borrow.
        let mut seen = u64::from_le(unsafe { shared_memory::load_acquire_u64(address) });
        while seen < value {
            // SAFETY: the same eight bytes, still mapped and aligned for this borrow.
            let found = unsafe {
                shared_memory::compare_exchange_u64(address, seen.to_le(), value.to_le())
            };
            let found = u64::from_le(found);
            if found == seen {
                return Some(value);
            }
            seen = found;
        }
        Some(seen)
    }

    fn publication_pair_address(&self, offset: usize) -> Option<usize> {
        let high = offset.checked_add(size_of::<u32>())?;
        if !self.layout.contains_publication_word(offset)
            || !self.layout.contains_publication_word(high)
        {
            return None;
        }
        let end = offset.checked_add(size_of::<u64>())?;
        if end as u64 > self.len() {
            return None;
        }
        let base = direct_pointer(self.address(), self.len())?;
        let address = base.addr().checked_add(offset)?;
        address.is_multiple_of(align_of::<u64>()).then_some(address)
    }

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

    pub fn frame(&self) -> SharedFrame {
        SharedFrame(self.allocation.another())
    }
}

fn write_shared(allocation: &SharedAllocation, offset: usize, bytes: &[u8]) -> bool {
    let Some(span) = DirectSpan::from_shared(allocation) else {
        return false;
    };
    span.write_at(offset, bytes)
}

fn read_shared(allocation: &SharedAllocation, offset: usize, into: &mut [u8]) -> bool {
    let Some(span) = DirectSpan::from_shared(allocation) else {
        return false;
    };
    span.read_at(offset, into)
}
