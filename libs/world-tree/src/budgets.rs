#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus_spec, verus_verify};

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    pub descendants: u32,
    pub threads: u32,
    pub handles: u32,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Budget {
    Descendants,
    Threads,
    Handles,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BudgetRefusal {
    Exceeded(Budget),
    Overcredited(Budget),
}

// What is left of a world's own limits; a credit never lifts a budget above its limit.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Budgets {
    pub(crate) limits: Limits,
    pub(crate) descendants: u32,
    pub(crate) threads: u32,
}

#[verus_verify]
impl Budgets {
    #[verus_spec(r =>
        ensures
            r.spec_limits() == limits,
            r.spec_descendants() == limits.descendants,
            r.spec_threads() == limits.threads,
    )]
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            descendants: limits.descendants,
            threads: limits.threads,
        }
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_limits(),
    )]
    pub fn limits(self) -> Limits {
        self.limits
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_descendants(),
            r <= self.spec_limits().descendants,
    )]
    pub fn descendants(self) -> u32 {
        proof! {
            use_type_invariant(self);
        }
        self.descendants
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_threads(),
            r <= self.spec_limits().threads,
    )]
    pub fn threads(self) -> u32 {
        proof! {
            use_type_invariant(self);
        }
        self.threads
    }

    // A child and every world it may create join this world's descendant set.
    #[verus_spec(r =>
        ensures
            crate::proof::debit_conserves_the_total(
                self,
                child.descendants + 1,
                child.threads as int,
                r,
            ),
            r is Err ==> child.descendants + 1 > self.spec_descendants()
                || child.threads > self.spec_threads()
                || child.handles > self.spec_limits().handles,
    )]
    pub fn debit_child(self, child: Limits) -> Result<Self, BudgetRefusal> {
        proof! {
            use_type_invariant(self);
        }
        let Some(descendants) = child.descendants.checked_add(1) else {
            return Err(BudgetRefusal::Exceeded(Budget::Descendants));
        };
        if descendants > self.descendants {
            return Err(BudgetRefusal::Exceeded(Budget::Descendants));
        }
        if child.threads > self.threads {
            return Err(BudgetRefusal::Exceeded(Budget::Threads));
        }
        if child.handles > self.limits.handles {
            return Err(BudgetRefusal::Exceeded(Budget::Handles));
        }
        Ok(Self {
            limits: self.limits,
            descendants: self.descendants - descendants,
            threads: self.threads - child.threads,
        })
    }

    #[verus_spec(r =>
        ensures
            crate::proof::credit_conserves_the_total(
                self,
                child.descendants + 1,
                child.threads as int,
                r,
            ),
            r is Err ==> self.spec_descendants() + child.descendants + 1
                > self.spec_limits().descendants
                || self.spec_threads() + child.threads > self.spec_limits().threads,
    )]
    pub fn credit_child(self, child: Limits) -> Result<Self, BudgetRefusal> {
        proof! {
            use_type_invariant(self);
        }
        let Some(descendants) = child.descendants.checked_add(1) else {
            return Err(BudgetRefusal::Overcredited(Budget::Descendants));
        };
        if descendants > self.limits.descendants - self.descendants {
            return Err(BudgetRefusal::Overcredited(Budget::Descendants));
        }
        if child.threads > self.limits.threads - self.threads {
            return Err(BudgetRefusal::Overcredited(Budget::Threads));
        }
        Ok(Self {
            limits: self.limits,
            descendants: self.descendants + descendants,
            threads: self.threads + child.threads,
        })
    }

    #[verus_spec(r =>
        ensures
            crate::proof::debit_conserves_the_total(self, 0, threads as int, r),
            r is Err ==> threads > self.spec_threads(),
    )]
    pub fn debit_threads(self, threads: u32) -> Result<Self, BudgetRefusal> {
        proof! {
            use_type_invariant(self);
        }
        if threads > self.threads {
            return Err(BudgetRefusal::Exceeded(Budget::Threads));
        }
        Ok(Self {
            threads: self.threads - threads,
            ..self
        })
    }

    #[verus_spec(r =>
        ensures
            crate::proof::credit_conserves_the_total(self, 0, threads as int, r),
            r is Err ==> self.spec_threads() + threads > self.spec_limits().threads,
    )]
    pub fn credit_threads(self, threads: u32) -> Result<Self, BudgetRefusal> {
        proof! {
            use_type_invariant(self);
        }
        if threads > self.limits.threads - self.threads {
            return Err(BudgetRefusal::Overcredited(Budget::Threads));
        }
        Ok(Self {
            threads: self.threads + threads,
            ..self
        })
    }
}
