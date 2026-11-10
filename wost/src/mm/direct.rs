use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use mmio_range::AccessRange;

use super::frame::Segment;
use super::shared::SharedAllocation;
use super::{Paddr, direct_pointer};
use crate::arch::current::cpu;

pub(super) struct DirectSpan<'authority> {
    base: *mut u8,
    len: usize,
    _authority: PhantomData<&'authority ()>,
}

impl<'authority> DirectSpan<'authority> {
    pub(super) fn from_segment(segment: &'authority Segment) -> Option<Self> {
        // SAFETY: a live `Segment` is the allocator's unique handle for every
        // frame in this run and keeps those direct-mapped bytes allocated.
        unsafe { Self::from_proven_run(segment.address(), segment.len()) }
    }

    pub(super) fn from_shared(allocation: &'authority SharedAllocation) -> Option<Self> {
        // SAFETY: `SharedAllocation` retains its segment for this borrow and
        // permits byte access specifically through volatile operations.
        unsafe { Self::from_proven_run(allocation.0.address(), allocation.0.len()) }
    }

    /// # Safety
    ///
    /// The `len` bytes at `physical` must stay allocated and direct-mapped for `'authority`.
    /// They must back no live Rust reference meanwhile, since the span writes them through `&self`.
    pub(super) unsafe fn from_proven_run(physical: Paddr, len: u64) -> Option<Self> {
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

    pub(super) fn fill_volatile(&self, byte: u8) -> bool {
        let Ok(range) = AccessRange::repeated(0, self.len, 1, self.len) else {
            return false;
        };
        range
            .offsets()
            .all(|offset| self.write_byte_volatile(offset, byte))
    }

    pub(super) fn write_volatile_at(&self, offset: usize, bytes: &[u8]) -> bool {
        let Ok(range) = AccessRange::repeated(offset, bytes.len(), 1, self.len) else {
            return false;
        };
        range
            .offsets()
            .zip(bytes.iter().copied())
            .all(|(offset, byte)| self.write_byte_volatile(offset, byte))
    }

    pub(super) fn write_u64_volatile_at(&self, offset: usize, value: u64) -> bool {
        let Some(end) = offset.checked_add(size_of::<u64>()) else {
            return false;
        };
        let destination = self.base.wrapping_add(offset);
        if end > self.len || !(destination as usize).is_multiple_of(align_of::<u64>()) {
            return false;
        }
        let word = destination.cast::<u64>();
        // SAFETY: the span's authority keeps all eight checked bytes live, and
        // the alignment check makes this one indivisible x86-64 qword store.
        unsafe { word.write_volatile(value.to_le()) };
        true
    }

    pub(super) fn read_volatile_at(&self, offset: usize, into: &mut [u8]) -> bool {
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
    pub(super) fn flush_line_at(&self, offset: usize) -> bool {
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
    pub(super) fn flush_all(&self) -> bool {
        let flushed = (0..self.len)
            .step_by(CACHE_LINE)
            .all(|offset| self.flush_line_at(offset));
        core::sync::atomic::fence(Ordering::SeqCst);
        flushed
    }

    #[cfg(feature = "selftest")]
    pub(super) fn all_bytes_equal(&self, byte: u8) -> bool {
        let Ok(range) = AccessRange::repeated(0, self.len, 1, self.len) else {
            return false;
        };
        range
            .offsets()
            .all(|offset| self.read_byte_volatile(offset) == Some(byte))
    }

    #[cfg(feature = "selftest")]
    pub(super) fn starts_with(&self, expected: &[u8]) -> bool {
        let Ok(range) = AccessRange::repeated(0, expected.len(), 1, self.len) else {
            return false;
        };
        range
            .offsets()
            .zip(expected)
            .all(|(offset, wanted)| self.read_byte_volatile(offset) == Some(*wanted))
    }
}

/// Bytes in the `clflush` line every x86-64 processor reports; a smaller line is still covered.
const CACHE_LINE: usize = 64;
