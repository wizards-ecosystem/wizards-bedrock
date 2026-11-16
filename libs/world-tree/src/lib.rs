#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod budgets;
mod creation;
mod descendants;
mod path;
mod process;
mod state;
mod teardown;
#[cfg(test)]
mod tests;

use core::fmt;

pub use budgets::{Budget, BudgetRefusal, Budgets, Limits};
pub use creation::{Creation, CreationEvent, CreationRefusal, CreationStep};
pub use descendants::{DescendantRefusal, Descendants};
pub use path::{MAX_DEPTH, Path, TooDeep};
pub use process::{ProcessState, Status};
pub use state::{Admission, Reclaimed, WorldEvent, WorldState};
pub use teardown::{ProcessStep, Progress, WorldStep};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NodeRef {
    pub node: u32,
    pub generation: u32,
}

const UNUSED: NodeRef = NodeRef {
    node: 0,
    generation: 0,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Unexpected;

impl fmt::Display for Unexpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the event has no transition from this state")
    }
}

impl core::error::Error for Unexpected {}
