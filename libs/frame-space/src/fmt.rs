use core::fmt;

use crate::{FrameSpace, FreeError};

impl fmt::Display for FreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotManaged(frame) => write!(formatter, "frame {frame} is not managed memory"),
            Self::AlreadyFree(frame) => write!(formatter, "frame {frame} is already free"),
            Self::NotCharged(frame) => {
                write!(formatter, "frame {frame} is not charged to the reservation")
            }
        }
    }
}

impl core::error::Error for FreeError {}

// Counts only: the bitmaps run to megabytes and a panic handler may print one.
impl<const WORDS: usize> fmt::Debug for FrameSpace<WORDS> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrameSpace")
            .field("managed", &self.managed_count())
            .field("free", &self.free_count())
            .finish()
    }
}
