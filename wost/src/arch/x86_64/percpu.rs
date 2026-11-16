use core::sync::atomic::{AtomicU64, Ordering};

use lock_order::HeldRanks;

use crate::mm::PhysicalAddress;

// No physical address reaches this, so the next user root always differs from it.
const UNKNOWN_ROOT: u64 = u64::MAX;

// Atomics, so trap and panic paths observe interrupted updates.
#[derive(Debug)]
pub(crate) struct PerCpu {
    held_ranks: AtomicU64,
    last_user_root: AtomicU64,
}

impl PerCpu {
    const fn new() -> Self {
        Self {
            held_ranks: AtomicU64::new(0),
            last_user_root: AtomicU64::new(UNKNOWN_ROOT),
        }
    }

    #[must_use = "a changed user root needs a predictor barrier"]
    pub(crate) fn enter_user_root(&self, root: PhysicalAddress) -> bool {
        self.last_user_root.swap(root.get(), Ordering::Relaxed) != root.get()
    }

    // A released root's frame can return as another space's root, which must still barrier.
    pub(crate) fn forget_user_root(&self, root: PhysicalAddress) {
        let _ = self.last_user_root.compare_exchange(
            root.get(),
            UNKNOWN_ROOT,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }

    pub(crate) fn held_ranks(&self) -> HeldRanks {
        HeldRanks::from_bits(self.held_ranks.load(Ordering::Relaxed))
    }

    pub(crate) fn set_held_ranks(&self, held: HeldRanks) {
        self.held_ranks.store(held.bits(), Ordering::Relaxed);
    }
}

static BOOTSTRAP: PerCpu = PerCpu::new();

pub(crate) fn current() -> &'static PerCpu {
    &BOOTSTRAP
}
