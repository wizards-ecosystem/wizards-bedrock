#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod budgets;
mod creation;
mod descendants;
// The crate's only unverified code: Verus cannot check formatting.
#[cfg_attr(verus_only, verifier::external)]
mod fmt;
mod path;
mod process;
pub mod proof;
mod state;
mod teardown;
#[cfg(test)]
mod tests;

use vstd::prelude::{verus_spec, verus_verify};

pub use budgets::{Budget, BudgetRefusal, Budgets, Limits};
pub use creation::{Creation, CreationEvent, CreationRefusal, CreationStep};
pub use descendants::{DescendantRefusal, Descendants};
pub use path::{MAX_DEPTH, Path, TooDeep};
pub use process::{ProcessState, Status};
pub use state::{Admission, Reclaimed, WorldEvent, WorldState};
pub use teardown::{ProcessStep, Progress, WorldStep};

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NodeRef {
    pub node: u32,
    pub generation: u32,
}

#[verus_verify]
const UNUSED: NodeRef = NodeRef {
    node: 0,
    generation: 0,
};

#[verus_verify]
impl NodeRef {
    #[verus_spec(r =>
        ensures
            r == (self == other),
    )]
    const fn same(self, other: Self) -> bool {
        self.node == other.node && self.generation == other.generation
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Unexpected;
