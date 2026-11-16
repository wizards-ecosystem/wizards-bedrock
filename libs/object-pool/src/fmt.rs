use core::fmt;

use crate::Refusal;

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full => "no slot on the page is free",
            Self::Stale => "the slot holds no live object of that generation",
            Self::Condemned => "the object's strong count already reached zero",
            Self::Saturated => "the object's strong count is at its maximum",
            Self::Held => "the object still has strong references",
        })
    }
}

impl core::error::Error for Refusal {}
