use nvme::Capacity;
use wost::arch::current::vtd::{SegmentChangeError, Vtd};
use wost::mm::{self, DmaSegment};
use x86_paging::PAGE_SIZE;

#[derive(Debug)]
pub(super) struct QueuePair {
    pub(super) id: u16,
    pub(super) capacity: Capacity,
    pub(super) submission: DmaSegment,
    pub(super) completion: DmaSegment,
    // Set while a translation may be live or cached; an uncertain change leaves it set.
    submission_mapped: bool,
    completion_mapped: bool,
    // Shadowed because the doorbell is write-only.
    pub(super) tail: u32,
    pub(super) submission_head: u32,
    pub(super) completion_head: u32,
    // A zeroed queue reads phase zero, so the first pass wants one.
    pub(super) phase: bool,
}

impl QueuePair {
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

    pub(super) fn retain(&mut self) {
        self.submission.retain();
        self.completion.retain();
    }

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

    pub(super) fn unmap_from(&mut self, isolation: &mut Vtd) -> bool {
        if self.submission_mapped && isolation.unmap_segment(&mut self.submission).is_ok() {
            self.submission_mapped = false;
        }
        if self.completion_mapped && isolation.unmap_segment(&mut self.completion).is_ok() {
            self.completion_mapped = false;
        }
        !self.has_mappings()
    }
}

impl Drop for QueuePair {
    fn drop(&mut self) {
        self.retain_mapped();
    }
}

const fn frames_for(bytes: u64) -> usize {
    bytes.div_ceil(PAGE_SIZE) as usize
}
