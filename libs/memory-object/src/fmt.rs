use core::fmt;

use crate::{MapperRefusal, NoAddress, QuotaRefusal, RecordRefusal, StateRefusal};

impl fmt::Display for StateRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sealed => "the object is sealed",
            Self::Busy => "the object still has a write reference",
            Self::Saturated => "the object's writer count is at its maximum",
            Self::NoWriter => "the object holds no write reference to release",
        })
    }
}

impl core::error::Error for StateRefusal {}

impl fmt::Display for QuotaRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoQuota => "the quota cannot cover the pages",
            Self::Overcredited => "the credit exceeds the pages the quota has used",
        })
    }
}

impl core::error::Error for QuotaRefusal {}

impl fmt::Display for RecordRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Overlap => "the extent overlaps a record",
            Self::Full => "the space holds its maximum number of records",
            Self::NotFound => "no record has exactly that extent",
            Self::NotCovered => "no single record covers the extent",
        })
    }
}

impl core::error::Error for RecordRefusal {}

impl fmt::Display for MapperRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stale => "the mapper slot has moved to another generation",
            Self::Revoking => "the owner is revoking the mapper slot",
            Self::Unexpected => "the event has no transition from this mapper slot state",
        })
    }
}

impl core::error::Error for MapperRefusal {}

impl fmt::Display for NoAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no aligned gap in the placement range fits the pages")
    }
}

impl core::error::Error for NoAddress {}
