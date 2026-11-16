use alloc::sync::Arc;
use core::mem::ManuallyDrop;

use ring_proto::SharedLayout;

use super::direct::DirectSpan;
use super::frame::Segment;
use super::{PhysicalAddress, direct_pointer};
use crate::arch::current::shared_memory;

#[derive(PartialEq, Eq, Debug)]
pub struct USegment {
    segment: ManuallyDrop<Segment>,
}

impl USegment {
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
}

impl Drop for USegment {
    fn drop(&mut self) {
        if self.segment.zero() {
            // SAFETY: this is the one drop of `segment`, and nothing reads the field after it.
            unsafe { ManuallyDrop::drop(&mut self.segment) };
        }
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

impl From<Segment> for USegment {
    fn from(segment: Segment) -> Self {
        Self {
            segment: ManuallyDrop::new(segment),
        }
    }
}
