use core::sync::atomic::{AtomicU64, Ordering};

use tlb_generation::{Action, Extent, catch_up};

use super::super::{cpu, interrupt, percpu};
use crate::mm::PhysicalAddress;
use crate::sync::ThreadContext;

// Every shootdown takes the next generation; a processor records the newest one it flushed for.
static REQUESTED: AtomicU64 = AtomicU64::new(0);

// Processors that may hold a stale translation, by index, and the generation each must reach.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "a translation may survive until the flush completes"]
pub struct Flush {
    cpus: u64,
    generation: u64,
}

impl Flush {
    pub const fn none() -> Self {
        Self {
            cpus: 0,
            generation: 0,
        }
    }

    pub const fn merge(&mut self, other: Self) {
        self.cpus |= other.cpus;
        if other.generation > self.generation {
            self.generation = other.generation;
        }
    }

    pub(crate) const fn duplicate(&self) -> Self {
        Self {
            cpus: self.cpus,
            generation: self.generation,
        }
    }

    // Every other processor past the newest shootdown requested so far, so past every earlier one.
    pub(crate) fn barrier() -> Self {
        Self {
            cpus: everywhere_else(),
            generation: REQUESTED.load(Ordering::Acquire),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.laggards() == 0
    }

    pub fn wait(self, context: &mut ThreadContext) {
        let _ = context;
        self.settle();
    }

    fn laggards(&self) -> u64 {
        percpu::behind(self.cpus, self.generation)
    }

    // Waits only where the statement allows: interrupts on and no spin lock held.
    pub(crate) fn settle(self) {
        if self.is_complete() {
            return;
        }
        assert!(
            interrupt::are_enabled() && !crate::sync::holds_spin_lock(),
            "a flush is awaited only with interrupts on and no spin lock held"
        );
        // A merged token may name a processor that answered an older generation; ask it again.
        percpu::send_shootdown(self.laggards());
        while !self.is_complete() {
            core::hint::spin_loop();
        }
    }
}

pub(super) fn shoot(targets: u64) -> Flush {
    if targets == 0 {
        return Flush::none();
    }
    let generation = REQUESTED.fetch_add(1, Ordering::AcqRel) + 1;
    percpu::send_shootdown(targets);
    Flush {
        cpus: targets,
        generation,
    }
}

pub(super) fn everywhere_else() -> u64 {
    percpu::others()
}

pub(super) fn running(root: PhysicalAddress) -> u64 {
    percpu::others_running(root)
}

// The shootdown vector's handler: one whole flush covers every generation requested so far.
pub(crate) fn answer() {
    let latest = REQUESTED.load(Ordering::Acquire);
    let seen = percpu::current().flushed();
    if let (Action::Whole | Action::Range, newest) = catch_up(seen, latest, latest, Extent::Whole) {
        cpu::reload_page_table();
        percpu::current().record_flush(newest);
    }
}
