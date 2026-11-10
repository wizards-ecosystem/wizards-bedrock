use core::num::NonZeroU64;
use core::sync::atomic::Ordering;

use frame_space::frame_identity;

use super::Paddr;
use super::direct::DirectSpan;
use super::frame::{Segment, allocate_contiguous};

/// Frames made reachable by a device for direct memory access.
///
/// A command whose completion is uncertain cannot return its frames to the
/// allocator: the device may still write through the physical address it was
/// given. A live mapping prevents recycling even if its owner is dropped.
/// [`retain`](Self::retain) additionally removes every safe way to access the
/// allocation when completion or revocation is uncertain.
#[derive(PartialEq, Eq, Debug)]
pub struct DmaSegment {
    segment: Option<Segment>,
    mapping_owner: Option<DmaMappingOwner>,
}

/// Stable identity of a VT-d root whose published frames cannot be recycled.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DmaMappingOwner(NonZeroU64);

impl DmaMappingOwner {
    /// Name a root whose published owner permanently withholds its allocation.
    ///
    /// # Panics
    ///
    /// Panics if the root is not frame aligned.
    pub(crate) const fn new(root: Paddr) -> Self {
        let Some(identity) = frame_identity(root) else {
            panic!("a DMA mapping owner root is not frame aligned");
        };
        Self(identity)
    }
}

impl DmaSegment {
    /// Reserve the single mapping lease before its first hardware publication.
    pub(crate) fn begin_mapping(&mut self, owner: DmaMappingOwner) -> bool {
        if self.segment.is_none() || self.mapping_owner.is_some() {
            return false;
        }
        self.mapping_owner = Some(owner);
        true
    }

    /// Whether this exact never-recycled root owns the live mapping lease.
    pub(crate) fn is_mapped_by(&self, owner: DmaMappingOwner) -> bool {
        self.mapping_owner == Some(owner)
    }

    /// End the exact owner's lease after proved rollback or DMA revocation.
    pub(crate) fn release_mapping(&mut self, owner: DmaMappingOwner) -> bool {
        if !self.is_mapped_by(owner) {
            return false;
        }
        self.mapping_owner = None;
        true
    }

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

/// How an IOMMU reads the tables the CPU writes for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
/// a device cannot reach the structure that constrains it. Unpublished tables
/// free their frames on drop; PublishedIommuTable retains every allocation a
/// unit may have observed (D-357).
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

/// A table that a unit may have observed, retained even when its owner drops.
#[derive(Debug)]
pub(crate) struct PublishedIommuTable {
    table: Option<IommuTable>,
}

impl PublishedIommuTable {
    /// Convert unpublished frames before a unit can first observe them.
    pub(crate) const fn new(table: IommuTable) -> Self {
        Self { table: Some(table) }
    }

    /// The stable physical address programmed into hardware.
    pub(crate) const fn address(&self) -> Paddr {
        match &self.table {
            Some(table) => table.address(),
            None => panic!("published table has no frame owner"),
        }
    }

    /// Publish one checked qword through the retained table's original writer.
    pub(crate) fn write_u64(&mut self, index: usize, value: u64) -> bool {
        self.table
            .as_mut()
            .is_some_and(|table| table.write_u64(index, value))
    }

    /// Read one checked qword from the retained allocation.
    pub(crate) fn read_u64(&self, index: usize) -> Option<u64> {
        self.table.as_ref()?.read_u64(index)
    }
}

impl Drop for PublishedIommuTable {
    fn drop(&mut self) {
        if let Some(table) = self.table.take() {
            core::mem::forget(table);
        }
    }
}
