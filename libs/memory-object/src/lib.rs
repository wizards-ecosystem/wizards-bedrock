#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

// The crate's only unverified code: Verus cannot check formatting.
#[cfg_attr(verus_only, verifier::external)]
mod fmt;
mod mapper;
mod placement;
pub mod proof;
mod quota;
mod records;
mod state;

pub use mapper::{MapperEvent, MapperRefusal, MapperSlot, MapperState};
pub use placement::{NoAddress, PLACEMENT_SAMPLES, Placed, place};
pub use quota::{Quota, QuotaRefusal};
pub use records::{Extent, RECORDS, Record, RecordRefusal, Records};
pub use state::{ObjectState, Seal, StateRefusal, StateWord};
