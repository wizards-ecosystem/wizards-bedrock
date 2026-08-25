//! State that belongs to one processor.
//!
//! **The seam, built before there is a second CPU to need it** (D-154). Every
//! value here is per-processor by nature: how many locks *this* CPU holds, how
//! many of them are leaves, where it last broke the leaf rule. On one
//! processor a `static` is indistinguishable from a per-CPU word, which is why
//! they were `static`s — and why the difference is invisible until an AP
//! starts and two CPUs begin reporting each other's numbers.
//!
//! What makes this worth building now rather than then: **every caller goes
//! through [`current`]**. When an AP starts, `current` reads the block out of
//! `gs:` and nothing else in the tree changes. Moving the callers is the
//! expensive part and it is cheaper before the network stack adds three more
//! of them.
//!
//! `gs:` is **not** used yet, and this module does not pretend otherwise.
//! D-81 wrote that `arch::percpu` "has to exist for ring 3 regardless"; it did
//! not, because ring 3 goes through an interrupt gate rather than `syscall`
//! (D-96), so `swapgs` was never needed. There is one processor, one block,
//! and one accessor — the segment register arrives with the second CPU.

use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

/// One processor's own state.
///
/// Atomics rather than plain fields even though a CPU cannot race with itself:
/// a trap can land between any two instructions, and the panic path reads these
/// from a context that interrupted their writer. The cost is the same relaxed
/// load a `static` had.
#[derive(Debug)]
pub struct PerCpu {
    /// Spin locks this CPU is holding, of either kind.
    ///
    /// Read by the two invariants that need it from outside `sync`: setting
    /// the interrupt flag with one held, and switching threads with one held,
    /// both strand a lock on a context that can never resume (D-66).
    spinlocks_held: AtomicUsize,

    /// Of those, how many are **leaf** guards.
    ///
    /// Separate from the count above because the obligation is asymmetric: a
    /// leaf taken while a token-bearing lock is held is the design, and
    /// anything taken while a *leaf* is held is the violation (D-121, D-126).
    leaves_held: AtomicUsize,

    /// Where a lock was taken while a leaf guard was held, if anywhere.
    under_a_leaf: AtomicPtr<core::panic::Location<'static>>,

    /// The outermost leaf guard that was live when that happened.
    the_leaf: AtomicPtr<core::panic::Location<'static>>,

    /// The outermost leaf currently held, so a violation can name what it was
    /// under. Recording every leaf acquisition would name whichever was taken
    /// most recently, held or not.
    last_leaf: AtomicPtr<core::panic::Location<'static>>,
}

impl PerCpu {
    /// An unused block.
    const fn new() -> Self {
        Self {
            spinlocks_held: AtomicUsize::new(0),
            leaves_held: AtomicUsize::new(0),
            under_a_leaf: AtomicPtr::new(core::ptr::null_mut()),
            the_leaf: AtomicPtr::new(core::ptr::null_mut()),
            last_leaf: AtomicPtr::new(core::ptr::null_mut()),
        }
    }

    /// Locks this CPU holds.
    pub fn spinlocks_held(&self) -> usize {
        self.spinlocks_held.load(Ordering::Relaxed)
    }

    /// Note one more lock taken, and report whether a leaf was already held.
    ///
    /// One call rather than a read and a write, because the answer is about the
    /// state *before* this acquisition and a caller doing it in two steps would
    /// have to get the order right.
    pub fn took_lock(&self, leaf: bool, at: &'static core::panic::Location<'static>) -> bool {
        let under_a_leaf = self.leaves_held.load(Ordering::Relaxed) > 0;
        if under_a_leaf {
            self.the_leaf
                .store(self.last_leaf.load(Ordering::Relaxed), Ordering::Release);
            self.under_a_leaf
                .store(core::ptr::from_ref(at).cast_mut(), Ordering::Release);
        }
        self.spinlocks_held.fetch_add(1, Ordering::Relaxed);
        if leaf && self.leaves_held.fetch_add(1, Ordering::Relaxed) == 0 {
            self.last_leaf
                .store(core::ptr::from_ref(at).cast_mut(), Ordering::Relaxed);
        }
        under_a_leaf
    }

    /// Note one lock released.
    pub fn released_lock(&self, leaf: bool) {
        if leaf {
            self.leaves_held.fetch_sub(1, Ordering::Relaxed);
        }
        self.spinlocks_held.fetch_sub(1, Ordering::Relaxed);
    }

    /// Where a lock was taken under a leaf guard, if one ever was.
    pub fn taken_under_a_leaf(&self) -> Option<&'static core::panic::Location<'static>> {
        let at = self.under_a_leaf.load(Ordering::Acquire);
        // SAFETY: only ever stored from `Location::caller()`, which is
        // `'static` by that function's signature, and never freed.
        (!at.is_null()).then(|| unsafe { &*at.cast_const() })
    }

    /// The leaf guard that violation happened under.
    pub fn the_leaf_it_was_under(&self) -> Option<&'static core::panic::Location<'static>> {
        let at = self.the_leaf.load(Ordering::Acquire);
        // SAFETY: as `taken_under_a_leaf`.
        (!at.is_null()).then(|| unsafe { &*at.cast_const() })
    }
}

/// The one processor's block.
///
/// Private, so nothing can reach it except through [`current`] — which is what
/// makes the accessor the only thing that changes when there is a second.
static BOOTSTRAP: PerCpu = PerCpu::new();

/// This processor's own state.
///
/// One instance today. When an AP starts this reads `gs:`, and every caller
/// above it is already written against the right shape (D-154).
pub fn current() -> &'static PerCpu {
    &BOOTSTRAP
}
