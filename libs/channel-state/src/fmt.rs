use core::fmt;

use crate::{CellRefusal, InboxRefusal, NotArmed, Unexpected};

impl fmt::Display for Unexpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the event has no transition from this state")
    }
}

impl core::error::Error for Unexpected {}

impl fmt::Display for CellRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stale => "the event names an earlier call",
            Self::Unexpected => "the event has no transition from the cell's state",
            Self::Exhausted => "the cell has used its last call generation",
        })
    }
}

impl core::error::Error for CellRefusal {}

impl fmt::Display for InboxRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "the message exceeds 2048 bytes or 16 handles",
            Self::Full => "the inbox has no room for the message",
        })
    }
}

impl core::error::Error for InboxRefusal {}

impl fmt::Display for NotArmed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the receiver is not armed on this endpoint")
    }
}

impl core::error::Error for NotArmed {}
