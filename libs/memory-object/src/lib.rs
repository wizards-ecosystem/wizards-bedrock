#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod mapper;
mod placement;
mod quota;
mod records;
mod state;
#[cfg(test)]
mod tests;

pub use mapper::{MapperEvent, MapperRefusal, MapperSlot, MapperState};
pub use placement::{NoAddress, PLACEMENT_SAMPLES, Placed, place};
pub use quota::{Quota, QuotaRefusal};
pub use records::{Extent, RECORDS, Record, RecordRefusal, Records};
pub use state::{ObjectState, Seal, StateRefusal, StateWord};
