use nvme::Capacity;
use wost::arch::current::vtd::{SegmentChangeError, Vtd};
use wost::mm::{self, DmaSegment};
use x86_paging::PAGE_SIZE;

/// One submission queue and the completion queue that answers it.
#[derive(Debug)]
pub(super) struct QueuePair {
    /// Which queue this is; zero is the admin pair.
    pub(super) id: u16,
    /// How many entries each holds.
    pub(super) capacity: Capacity,
    /// The submission entries, physically contiguous.
    pub(super) submission: DmaSegment,
    /// The completion entries, likewise.
    pub(super) completion: DmaSegment,
    /// Whether each segment may still have a live or cached VT-d translation; an uncertain map or
    /// unmap leaves it set and retains the segment, so neither `Drop` nor teardown recycles it.
    submission_mapped: bool,
    completion_mapped: bool,
    /// Where the host writes next, shadowed because the doorbell is write-only.
    pub(super) tail: u32,
    pub(super) submission_head: u32,
    pub(super) completion_head: u32,
    /// The phase a new completion carries; a zeroed queue reads zero, so the first pass wants one.
    pub(super) phase: bool,
}

impl QueuePair {
    /// Allocate and clear both queues.
    pub(super) fn new(id: u16, capacity: Capacity) -> Option<Self> {
        let submission_frames = frames_for(capacity.submission_bytes());
        let completion_frames = frames_for(capacity.completion_bytes());

        let mut submission = DmaSegment::from(mm::allocate_contiguous(submission_frames, 1)?);
        let mut completion = DmaSegment::from(mm::allocate_contiguous(completion_frames, 1)?);
        // Cleared: an unwritten entry's phase tag says "nothing here yet", and stale memory lies.
        if !submission.zero() || !completion.zero() {
            return None;
        }

        Some(Self {
            id,
            capacity,
            submission,
            completion,
            submission_mapped: false,
            completion_mapped: false,
            tail: 0,
            submission_head: 0,
            completion_head: 0,
            phase: true,
        })
    }

    /// Withhold every queue frame a controller may still reach.
    pub(super) fn retain(&mut self) {
        self.submission.retain();
        self.completion.retain();
    }

    /// Withhold only segments which may still be named by the domain.
    pub(super) fn retain_mapped(&mut self) {
        if self.submission_mapped {
            self.submission.retain();
        }
        if self.completion_mapped {
            self.completion.retain();
        }
    }

    pub(super) const fn has_mappings(&self) -> bool {
        self.submission_mapped || self.completion_mapped
    }

    /// Make both queues reachable by the device, or neither.
    #[must_use = "unmapped queues cannot be published to the controller"]
    pub(super) fn map_into(&mut self, isolation: &mut Vtd) -> bool {
        if self.has_mappings() {
            return false;
        }
        match isolation.map_segment_for_device_read(&mut self.submission) {
            Ok(()) => self.submission_mapped = true,
            Err(SegmentChangeError::Refused(_)) => return false,
            Err(_) => {
                self.submission_mapped = true;
                self.submission.retain();
                return false;
            }
        }
        let completion_mapped = match isolation.map_segment_for_device_write(&mut self.completion) {
            Ok(()) => {
                self.completion_mapped = true;
                true
            }
            Err(SegmentChangeError::Refused(_)) => false,
            Err(_) => {
                self.completion_mapped = true;
                self.completion.retain();
                false
            }
        };
        if !completion_mapped {
            if isolation.unmap_segment(&mut self.submission).is_ok() {
                self.submission_mapped = false;
            }
            return false;
        }
        true
    }

    /// Remove and invalidate every queue translation before its frames drop.
    pub(super) fn unmap_from(&mut self, isolation: &mut Vtd) -> bool {
        if self.submission_mapped && isolation.unmap_segment(&mut self.submission).is_ok() {
            self.submission_mapped = false;
        }
        if self.completion_mapped && isolation.unmap_segment(&mut self.completion).is_ok() {
            self.completion_mapped = false;
        }
        !self.has_mappings()
    }

    /// Every frame of both queues, for a check that reads the domain back.
    #[cfg(feature = "selftest")]
    pub(super) fn frames(&self, into: &mut alloc::vec::Vec<u64>) -> Option<()> {
        for queue in [&self.submission, &self.completion] {
            let (address, len) = queue.address().zip(queue.len())?;
            into.extend((address..address + len).step_by(PAGE_SIZE as usize));
        }
        Some(())
    }
}

impl Drop for QueuePair {
    fn drop(&mut self) {
        self.retain_mapped();
    }
}

/// Frames enough to hold `bytes`.
const fn frames_for(bytes: u64) -> usize {
    bytes.div_ceil(PAGE_SIZE) as usize
}
