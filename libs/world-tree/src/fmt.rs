use core::fmt;

use crate::{Budget, BudgetRefusal, DescendantRefusal, TooDeep, Unexpected};

impl fmt::Display for Unexpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the event has no transition from this state")
    }
}

impl core::error::Error for Unexpected {}

impl fmt::Display for TooDeep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the parent already has the maximum number of ancestors")
    }
}

impl core::error::Error for TooDeep {}

impl fmt::Display for DescendantRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Frozen => "the owning world is not live, so its descendant set is frozen",
            Self::Present => "the world is already a descendant",
            Self::Full => "the descendant set is at capacity",
            Self::Absent => "the world is not a descendant",
        })
    }
}

impl core::error::Error for DescendantRefusal {}

impl fmt::Display for BudgetRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (verb, budget) = match self {
            Self::Exceeded(budget) => ("exceeds", budget),
            Self::Overcredited(budget) => ("overfills", budget),
        };
        let budget = match budget {
            Budget::Descendants => "descendants",
            Budget::Threads => "threads",
            Budget::Handles => "handles per table",
        };
        write!(f, "the change {verb} the {budget} budget")
    }
}

impl core::error::Error for BudgetRefusal {}
