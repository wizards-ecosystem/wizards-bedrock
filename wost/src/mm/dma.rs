use core::mem::ManuallyDrop;
use core::num::NonZeroU64;
use core::sync::atomic::Ordering;

use frame_space::frame_identity;

use super::PhysicalAddress;
use super::direct::DirectSpan;
use super::frame::{Segment, allocate_contiguous};

// A mapped or uncertain segment never returns its frames: the device may still write them.
#[derive(PartialEq, Eq, Debug)]
pub struct DmaSegment {
    segment: Option<Segment>,
    mapping_owner: Option<DmaMappingOwner>,
}

// Unique for the boot because the root it names is never freed.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DmaMappingOwner(NonZeroU64);

impl DmaMappingOwner {
    pub(crate) const fn new(root: PhysicalAddress) -> Self {
        let Some(identity) = frame_identity(root.get()) else {
            panic!("a DMA mapping owner root is not frame aligned");
        };
        Self(identity)
    }
}

impl DmaSegment {
    pub(crate) fn begin_mapping(&mut self, owner: DmaMappingOwner) -> bool {
        if self.segment.is_none() || self.mapping_owner.is_some() {
            return false;
        }
        self.mapping_owner = Some(owner);
        true
    }

    pub(crate) fn is_mapped_by(&self, owner: DmaMappingOwner) -> bool {
        self.mapping_owner == Some(owner)
    }

    pub(crate) fn release_mapping(&mut self, owner: DmaMappingOwner) -> bool {
        if !self.is_mapped_by(owner) {
            return false;
        }
        self.mapping_owner = None;
        true
    }

    pub const fn address(&self) -> Option<PhysicalAddress> {
        match &self.segment {
            Some(segment) => Some(segment.address()),
            None => None,
        }
    }

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

    pub const fn is_retained(&self) -> bool {
        self.segment.is_none()
    }

    #[must_use = "a refused DMA-segment fill changed no bytes"]
    pub fn fill(&mut self, byte: u8) -> bool {
        let Some(segment) = self.segment.as_mut() else {
            return false;
        };
        let Some(span) = DirectSpan::from_segment(segment) else {
            return false;
        };
        span.fill(byte)
    }

    #[must_use = "a refused DMA-segment clear changed no bytes"]
    pub fn zero(&mut self) -> bool {
        self.fill(0)
    }

    #[must_use = "a refused DMA-segment write changed no bytes"]
    pub fn write_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        let Some(segment) = self.segment.as_mut() else {
            return false;
        };
        let Some(span) = DirectSpan::from_segment(segment) else {
            return false;
        };
        span.write_at(offset, bytes)
    }

    #[must_use = "a refused DMA-segment read copied no bytes"]
    pub fn read_at(&self, offset: usize, into: &mut [u8]) -> bool {
        let Some(segment) = self.segment.as_ref() else {
            return false;
        };
        let Some(span) = DirectSpan::from_segment(segment) else {
            return false;
        };
        span.read_at(offset, into)
    }

    pub fn retain(&mut self) {
        if let Some(segment) = self.segment.take() {
            core::mem::forget(segment);
        }
    }
}

impl Drop for DmaSegment {
    fn drop(&mut self) {
        if self.mapping_owner.is_some() {
            self.retain();
        }
    }
}

impl From<Segment> for DmaSegment {
    fn from(segment: Segment) -> Self {
        Self {
            segment: Some(segment),
            mapping_owner: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TableWalks {
    Coherent,
    Uncached,
}

// Never mapped into any device's translation, so no device reaches the tables over it.
#[derive(Debug)]
pub(crate) struct IommuTable {
    frames: Segment,
    walks: TableWalks,
}

impl IommuTable {
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

    pub(crate) const fn address(&self) -> PhysicalAddress {
        self.frames.address()
    }

    #[must_use = "a refused IOMMU-table write changed no bytes"]
    pub(crate) fn write_u64(&mut self, index: usize, value: u64) -> bool {
        let walks = self.walks;
        let Some(span) = DirectSpan::from_segment(&self.frames) else {
            return false;
        };
        let Some(offset) = index.checked_mul(size_of::<u64>()) else {
            return false;
        };
        if !span.write_u64(offset, value) {
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

    pub(crate) fn read_u64(&self, index: usize) -> Option<u64> {
        let span = DirectSpan::from_segment(&self.frames)?;
        let mut bytes = [0u8; 8];
        span.read_at(index.checked_mul(8)?, &mut bytes)
            .then(|| u64::from_le_bytes(bytes))
    }
}

// A unit may have observed it, so its frames are never freed.
#[derive(Debug)]
pub(crate) struct PublishedIommuTable {
    table: ManuallyDrop<IommuTable>,
}

impl PublishedIommuTable {
    pub(crate) const fn new(table: IommuTable) -> Self {
        Self {
            table: ManuallyDrop::new(table),
        }
    }

    pub(crate) fn address(&self) -> PhysicalAddress {
        self.table.address()
    }

    pub(crate) fn write_u64(&mut self, index: usize, value: u64) -> bool {
        self.table.write_u64(index, value)
    }

    pub(crate) fn read_u64(&self, index: usize) -> Option<u64> {
        self.table.read_u64(index)
    }
}
