use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    pub descendants: u32,
    pub threads: u32,
    pub handles: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Budget {
    Descendants,
    Threads,
    Handles,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BudgetRefusal {
    Exceeded(Budget),
    Overcredited(Budget),
}

// What is left of a world's own limits; a credit never lifts a budget above its limit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Budgets {
    limits: Limits,
    descendants: u32,
    threads: u32,
}

impl Budgets {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            descendants: limits.descendants,
            threads: limits.threads,
        }
    }

    pub fn limits(self) -> Limits {
        self.limits
    }

    pub fn descendants(self) -> u32 {
        self.descendants
    }

    pub fn threads(self) -> u32 {
        self.threads
    }

    // A child and every world it may create join this world's descendant set.
    pub fn debit_child(self, child: Limits) -> Result<Self, BudgetRefusal> {
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

    pub fn credit_child(self, child: Limits) -> Result<Self, BudgetRefusal> {
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

    pub fn debit_threads(self, threads: u32) -> Result<Self, BudgetRefusal> {
        if threads > self.threads {
            return Err(BudgetRefusal::Exceeded(Budget::Threads));
        }
        Ok(Self {
            threads: self.threads - threads,
            ..self
        })
    }

    pub fn credit_threads(self, threads: u32) -> Result<Self, BudgetRefusal> {
        if threads > self.limits.threads - self.threads {
            return Err(BudgetRefusal::Overcredited(Budget::Threads));
        }
        Ok(Self {
            threads: self.threads + threads,
            ..self
        })
    }
}

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
