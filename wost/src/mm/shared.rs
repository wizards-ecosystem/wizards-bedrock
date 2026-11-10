use alloc::sync::Arc;
use core::mem::{align_of, size_of};

use ring_proto::SharedLayout;

use super::direct::DirectSpan;
use super::frame::Segment;
#[cfg(feature = "selftest")]
use super::selftest::observe_external_run_withheld_for_test;
use super::{Paddr, direct_pointer};
use crate::arch::current::shared_memory;

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
pub(super) struct SharedAllocation(pub(super) USegment);

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
