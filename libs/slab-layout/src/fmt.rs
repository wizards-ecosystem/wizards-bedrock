use core::fmt;

use crate::PlacementError;

impl fmt::Display for PlacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidAlignment(align) => {
                write!(
                    formatter,
                    "alignment {align} is not a non-zero power of two"
                )
            }
        }
    }
}

impl core::error::Error for PlacementError {}
