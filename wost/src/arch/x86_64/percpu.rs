//! Per-processor lock accounting.
//!
//! Every caller goes through [`current`], keeping today's singleton behind the
//! lookup that a future multiprocessor implementation must replace (D-154).

#[cfg(feature = "selftest")]
use core::sync::atomic::AtomicPtr;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Whether a spin guard may be followed by another lock acquisition.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LockKind {
    /// A token-bearing guard under which a leaf may still be taken.
    Ordered,
    /// A final guard under which no other lock may be taken.
    Leaf,
}

/// One processor's own state.
///
/// Atomics make interrupted updates observable to trap and panic paths.
#[derive(Debug)]
pub struct PerCpu {
    /// Spin locks this CPU is holding, of either kind.
    spinlocks_held: AtomicUsize,

    /// Of those, how many are **leaf** guards.
    #[cfg(feature = "selftest")]
    leaves_held: AtomicUsize,

    /// Where a lock was taken while a leaf guard was held, if anywhere.
    #[cfg(feature = "selftest")]
    under_a_leaf: AtomicPtr<core::panic::Location<'static>>,

    /// The outermost leaf guard that was live when that happened.
    #[cfg(feature = "selftest")]
    the_leaf: AtomicPtr<core::panic::Location<'static>>,

    /// The outermost leaf currently held, so a violation can name what it was
    /// under. Recording every leaf acquisition would name whichever was taken
    /// most recently, held or not.
    #[cfg(feature = "selftest")]
    last_leaf: AtomicPtr<core::panic::Location<'static>>,
}

impl PerCpu {
    /// An unused block.
    const fn new() -> Self {
        Self {
            spinlocks_held: AtomicUsize::new(0),
            #[cfg(feature = "selftest")]
            leaves_held: AtomicUsize::new(0),
            #[cfg(feature = "selftest")]
            under_a_leaf: AtomicPtr::new(core::ptr::null_mut()),
            #[cfg(feature = "selftest")]
            the_leaf: AtomicPtr::new(core::ptr::null_mut()),
            #[cfg(feature = "selftest")]
            last_leaf: AtomicPtr::new(core::ptr::null_mut()),
        }
    }

    /// Locks this CPU holds.
    pub(crate) fn spinlocks_held(&self) -> usize {
        self.spinlocks_held.load(Ordering::Relaxed)
    }

    /// Note one more lock taken.
    pub(crate) fn took_lock(
        &self,
        #[cfg(feature = "selftest")] kind: LockKind,
        #[cfg(feature = "selftest")] at: &'static core::panic::Location<'static>,
    ) {
        #[cfg(feature = "selftest")]
        {
            if self.leaves_held.load(Ordering::Relaxed) > 0 {
                self.the_leaf
                    .store(self.last_leaf.load(Ordering::Relaxed), Ordering::Release);
                self.under_a_leaf
                    .store(core::ptr::from_ref(at).cast_mut(), Ordering::Release);
            }
            if kind == LockKind::Leaf && self.leaves_held.fetch_add(1, Ordering::Relaxed) == 0 {
                self.last_leaf
                    .store(core::ptr::from_ref(at).cast_mut(), Ordering::Relaxed);
            }
        }
        self.spinlocks_held.fetch_add(1, Ordering::Relaxed);
    }

    /// Note one lock released.
    pub(crate) fn released_lock(&self, #[cfg(feature = "selftest")] kind: LockKind) {
        #[cfg(feature = "selftest")]
        if kind == LockKind::Leaf {
            self.leaves_held.fetch_sub(1, Ordering::Relaxed);
        }
        self.spinlocks_held.fetch_sub(1, Ordering::Relaxed);
    }

    /// Where a lock was taken under a leaf guard, if one ever was.
    #[cfg(feature = "selftest")]
    pub fn taken_under_a_leaf(&self) -> Option<&'static core::panic::Location<'static>> {
        let at = self.under_a_leaf.load(Ordering::Acquire);
        // SAFETY: only ever stored from `Location::caller()`, which is
        // `'static` by that function's signature, and never freed.
        (!at.is_null()).then(|| unsafe { &*at.cast_const() })
    }

    /// The leaf guard that violation happened under.
    #[cfg(feature = "selftest")]
    pub fn the_leaf_it_was_under(&self) -> Option<&'static core::panic::Location<'static>> {
        let at = self.the_leaf.load(Ordering::Acquire);
        // SAFETY: as `taken_under_a_leaf`.
        (!at.is_null()).then(|| unsafe { &*at.cast_const() })
    }
}

/// The one processor's block.
static BOOTSTRAP: PerCpu = PerCpu::new();

/// This processor's own state.
pub fn current() -> &'static PerCpu {
    &BOOTSTRAP
}
