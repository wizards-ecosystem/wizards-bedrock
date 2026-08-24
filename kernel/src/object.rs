//! The things a handle can name.
//!
//! `002`'s object model lists seven; this is the first, and the rest arrive
//! with the phases that need them. Every one of them is reached only through a
//! handle carrying rights (pillar 1), so the type here is deliberately not
//! `pub` beyond the crate: nothing outside the handle table should be able to
//! hold one directly.
//!
//! **Lifetime is `Arc`.** D-78 wants `inc_not_zero` from day one so that a
//! lookup racing a close cannot resurrect a corpse, and `Weak::upgrade` is
//! exactly that operation with the atomics already audited. Writing a refcount
//! by hand here would be a novel abstraction with no novelty (D-41).

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::thread::ThreadContext;
use crate::wait::WaitQueue;

/// What kind of thing a handle names.
///
/// The tag is separate from the payload because a handle's *type* is
/// answerable without touching the object — which is what lets a rights check
/// happen before a lookup commits to anything. Headroom is 64 types
/// (D-78): Zircon is at 26 and still growing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum ObjectType {
    /// A thing that can be signalled and waited on.
    Event,
}

/// A kernel object.
#[derive(Debug)]
#[non_exhaustive]
pub(crate) enum Object {
    /// See [`Event`].
    Event(Event),
}

impl Object {
    /// What kind of object this is.
    pub(crate) const fn kind(&self) -> ObjectType {
        match self {
            Self::Event(_) => ObjectType::Event,
        }
    }

    /// The event inside, if that is what this is.
    pub(crate) const fn as_event(&self) -> Option<&Event> {
        match self {
            Self::Event(event) => Some(event),
        }
    }
}

/// A thing that can be signalled, and waited on until it is.
///
/// The smallest object in the model, and the one that needs no memory and no
/// address space — which is why it is first: it exercises the handle table,
/// rights, refcounting and lifetime without waiting on ring 3.
///
/// Signalling is **sticky**. A waiter that arrives after the signal does not
/// block, which is the difference between an event and a condition variable
/// and the reason this one cannot lose a wakeup: the state is the record, and
/// the queue is only how sleepers are told about it.
#[derive(Debug)]
pub(crate) struct Event {
    signalled: AtomicU32,
    waiters: WaitQueue,
}

impl Event {
    /// A new, unsignalled event.
    pub(crate) fn new() -> Self {
        Self {
            signalled: AtomicU32::new(0),
            waiters: WaitQueue::new(),
        }
    }

    /// Whether it has been signalled.
    pub(crate) fn is_signalled(&self) -> bool {
        self.signalled.load(Ordering::Acquire) != 0
    }

    /// Signal it, waking everything waiting. Reports how many woke.
    ///
    /// The store is `Release` and the wait's load is `Acquire`, so a waiter
    /// that sees the signal sees everything the signaller did before it. Safe
    /// from a trap handler, like [`WaitQueue::wake_all`].
    pub(crate) fn signal(&self) -> usize {
        self.signalled.store(1, Ordering::Release);
        self.waiters.wake_all()
    }

    /// Block until it is signalled, returning at once if it already is.
    ///
    /// The check-then-wait is safe here for a reason the wait queue alone does
    /// not give: the signal is sticky, so a signal landing between the check
    /// and the block sets a state this loop re-reads rather than delivering a
    /// wakeup that can be missed.
    pub(crate) fn wait(&self, context: &mut ThreadContext) {
        while !self.is_signalled() {
            self.waiters.wait(context);
        }
    }
}

/// A reference to an object, counted.
pub(crate) type ObjectRef = Arc<Object>;

/// Make a new event, ready to be put in a handle table.
pub(crate) fn event() -> ObjectRef {
    Arc::new(Object::Event(Event::new()))
}
