use alloc::sync::Arc;
use core::mem::ManuallyDrop;

use ring_proto::SharedLayout;

use super::direct::DirectSpan;
use super::frame::{Segment, release_sealed_run, seal_run};
use super::{PhysicalAddress, direct_pointer};
use crate::arch::current::paging::Flush;
use crate::arch::current::shared_memory;

#[derive(PartialEq, Eq, Debug)]
pub struct USegment {
    segment: ManuallyDrop<Segment>,
}

impl USegment {
    #[must_use = "a refused conversion frees the run"]
    pub fn zeroed(mut segment: Segment) -> Option<Self> {
        if !segment.zero() {
            return None;
        }
        Some(Self {
            segment: ManuallyDrop::new(segment),
        })
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

    // `map_shared` writes no `OWNED` and `map_user` takes a `USegment`: one mapping path per run.
    #[must_use]
    pub fn share(self) -> Shared {
        Shared(Arc::new(SharedAllocation(self)))
    }

    pub(crate) fn commit_mapping(self) {
        core::mem::forget(self);
    }

    // Consuming the run proves no writable mapping of it remains, since mapping consumes it too.
    pub fn seal(self, flush: &mut Flush) -> SealedSegment {
        let mut run = ManuallyDrop::new(self);
        // SAFETY: `run` is never dropped, so this is the one move out of its segment.
        let segment = unsafe { ManuallyDrop::take(&mut run.segment) };
        seal_run(&segment, flush);
        SealedSegment {
            segment: ManuallyDrop::new(segment),
        }
    }
}

impl Drop for USegment {
    fn drop(&mut self) {
        if self.segment.zero() {
            // SAFETY: this is the one drop of `segment`, and nothing reads the field after it.
            unsafe { ManuallyDrop::drop(&mut self.segment) };
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

#[derive(Debug)]
pub(super) struct SharedAllocation(pub(super) USegment);

#[derive(Debug)]
pub(crate) struct SharedKeepalive {
    _allocation: Arc<SharedAllocation>,
}

#[derive(Debug)]
pub struct Shared(Arc<SharedAllocation>);

impl Shared {
    pub fn into_bytes(self) -> SharedBytes {
        SharedBytes(self.0)
    }

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

#[derive(Debug)]
pub struct SharedBytes(Arc<SharedAllocation>);

impl SharedBytes {
    pub fn address(&self) -> PhysicalAddress {
        self.0.0.address()
    }

    #[expect(
        clippy::len_without_is_empty,
        reason = "shared allocations cannot contain zero frames"
    )]
    pub fn len(&self) -> u64 {
        self.0.0.len()
    }

    #[must_use = "a refused shared-memory write changed no bytes"]
    pub fn write_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        write_shared(&self.0, offset, bytes)
    }

    #[must_use = "a refused shared-memory read copied no bytes"]
    pub fn read_at(&self, offset: usize, into: &mut [u8]) -> bool {
        read_shared(&self.0, offset, into)
    }

    pub(crate) fn keepalive(&self) -> SharedKeepalive {
        SharedKeepalive {
            _allocation: Arc::clone(&self.0),
        }
    }
}

#[derive(Debug)]
pub struct SharedRing<const WORDS: usize> {
    allocation: Arc<SharedAllocation>,
    layout: SharedLayout<WORDS>,
}

impl<const WORDS: usize> SharedRing<WORDS> {
    pub fn address(&self) -> PhysicalAddress {
        self.allocation.0.address()
    }

    #[expect(
        clippy::len_without_is_empty,
        reason = "shared allocations cannot contain zero frames"
    )]
    pub fn len(&self) -> u64 {
        self.allocation.0.len()
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

    pub(crate) fn keepalive(&self) -> SharedKeepalive {
        SharedKeepalive {
            _allocation: Arc::clone(&self.allocation),
        }
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
