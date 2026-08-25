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

use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::fmt;
use core::sync::atomic::{AtomicU32, Ordering};

use wost::sync::SpinLock;

use handle_space::Rights;

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
    /// One end of a message pipe carrying bytes and handles.
    Channel,
}

impl ObjectType {
    /// The rights that mean something for this kind of object.
    ///
    /// Beyond the universal four, which mean the same for everything. D-78's
    /// rule made checkable: a right must name a distinction the kernel makes
    /// about *this* object, and an event has no pages, so none of the memory
    /// rights apply to one (D-95).
    pub(crate) const fn per_type_rights(self) -> Rights {
        match self {
            Self::Event => Rights::NONE,
            // A channel end is read from and written to, and those are
            // distinctions the kernel makes: a send needs one and a receive
            // the other. They are not syscall names, which is the rule D-78
            // states and WASI's rights violated.
            Self::Channel => Rights::READ.union(Rights::WRITE),
        }
    }
}

/// A kernel object.
#[derive(Debug)]
#[non_exhaustive]
pub(crate) enum Object {
    /// See [`Event`].
    Event(Event),
    /// See [`Channel`].
    Channel(Channel),
}

impl Object {
    /// What kind of object this is.
    pub(crate) const fn kind(&self) -> ObjectType {
        match self {
            Self::Event(_) => ObjectType::Event,
            Self::Channel(_) => ObjectType::Channel,
        }
    }

    /// The event inside, if that is what this is.
    pub(crate) const fn as_event(&self) -> Option<&Event> {
        match self {
            Self::Event(event) => Some(event),
            Self::Channel(_) => None,
        }
    }

    /// The channel end inside, if that is what this is.
    pub(crate) const fn as_channel(&self) -> Option<&Channel> {
        match self {
            Self::Channel(channel) => Some(channel),
            Self::Event(_) => None,
        }
    }
}

/// A thing that can be signalled, and waited on until it is.
///
/// The smallest object in the model, and the one that needs no memory and no
/// address space — which is why it is first: it exercises the handle table,
/// rights, refcounting and lifetime without waiting on ring 3.
///
/// Signalling is **sticky**: a waiter that arrives after the signal does not
/// block at all, which is the difference between an event and a condition
/// variable. That is a useful property and it is *not* what makes the wait
/// free of lost wakeups — [`WaitQueue::wait_unless`] is, by registering before
/// it re-reads the state (D-95).
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

    /// The queue behind this event, for the check that drives its wait window.
    #[cfg(feature = "selftest")]
    pub(crate) const fn waiters_for_test(&self) -> &WaitQueue {
        &self.waiters
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
    /// The re-check happens **after** registering and with the interrupt flag
    /// clear, which is what makes it safe. An earlier version checked first
    /// and claimed stickiness covered the window; it does not, because the
    /// state can only be re-read once a block *returns*, and it returns only
    /// on a wake. A signal landing in that window woke nobody and the thread
    /// slept for ever (D-95).
    pub(crate) fn wait(&self, context: &mut ThreadContext) {
        while !self.is_signalled() {
            self.waiters.wait_unless(context, || self.is_signalled());
        }
    }
}

/// A reference to an object, counted.
pub(crate) type ObjectRef = Arc<Object>;

/// Make a new event, ready to be put in a handle table.
pub(crate) fn event() -> ObjectRef {
    Arc::new(Object::Event(Event::new()))
}

/// A message in flight: bytes, and handles with the rights they were sent with.
///
/// Handles travel as `(object, rights)` rather than as table entries, because a
/// handle value means nothing outside the table that minted it (D-78). What
/// crosses is the object and the authority over it, and the receiving table
/// mints its own value.
#[derive(Debug)]
pub(crate) struct Message {
    bytes: Vec<u8>,
    handles: Vec<(ObjectRef, Rights)>,
}

impl Message {
    /// The most bytes one message carries.
    pub(crate) const MAX_BYTES: usize = 4096;

    /// The most handles one message carries.
    ///
    /// Bounded because a message is kernel memory a sender chooses the size
    /// of, and the quota that will govern that properly is D-44's.
    pub(crate) const MAX_HANDLES: usize = 8;

    /// A message of `bytes` and nothing else.
    ///
    /// # Errors
    ///
    /// [`ChannelError::TooLarge`] past [`MAX_BYTES`](Self::MAX_BYTES), and
    /// [`ChannelError::OutOfMemory`] if the copy cannot be made.
    pub(crate) fn new(bytes: &[u8]) -> Result<Self, ChannelError> {
        if bytes.len() > Self::MAX_BYTES {
            return Err(ChannelError::TooLarge(bytes.len()));
        }
        let mut copy = Vec::new();
        copy.try_reserve_exact(bytes.len())
            .map_err(|_| ChannelError::OutOfMemory)?;
        copy.extend_from_slice(bytes);
        Ok(Self {
            bytes: copy,
            handles: Vec::new(),
        })
    }

    /// Add a handle to the message, with the rights it is being sent with.
    ///
    /// The rights are the caller's to state and there is no default (D-110).
    /// Fuchsia RFC-0028's finding is the reason: a default has unclear
    /// behaviour that is inconsistent across object types, and the place that
    /// shows is exactly here, where one object's "everything" is another's
    /// "far too much".
    ///
    /// # Errors
    ///
    /// [`ChannelError::TooManyHandles`] past
    /// [`MAX_HANDLES`](Self::MAX_HANDLES), and [`ChannelError::OutOfMemory`].
    pub(crate) fn attach(&mut self, object: ObjectRef, rights: Rights) -> Result<(), ChannelError> {
        if self.handles.len() >= Self::MAX_HANDLES {
            return Err(ChannelError::TooManyHandles);
        }
        self.handles
            .try_reserve(1)
            .map_err(|_| ChannelError::OutOfMemory)?;
        self.handles.push((object, rights));
        Ok(())
    }

    /// The bytes it carries.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Take the handles out, leaving the bytes.
    pub(crate) fn take_handles(&mut self) -> Vec<(ObjectRef, Rights)> {
        core::mem::take(&mut self.handles)
    }
}

/// Why a channel operation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelError {
    /// More bytes than one message carries.
    TooLarge(usize),
    /// More handles than one message carries.
    TooManyHandles,
    /// The other end is gone, so nothing will ever read this.
    PeerClosed,
    /// The other end's queue is full.
    Full,
    /// No memory for the message.
    OutOfMemory,
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(bytes) => write!(f, "{bytes} bytes is more than a message carries"),
            Self::TooManyHandles => f.write_str("more handles than a message carries"),
            Self::PeerClosed => f.write_str("the other end is closed"),
            Self::Full => f.write_str("the other end's queue is full"),
            Self::OutOfMemory => f.write_str("no memory for the message"),
        }
    }
}

/// What one end of a channel holds.
#[derive(Debug)]
struct Inbox {
    messages: VecDeque<Message>,
}

/// One end of a bidirectional message pipe.
///
/// The IPC primitive (`002`). Two ends are made together and each holds a weak
/// reference to the other: a send appends to the *peer's* inbox, and a receive
/// takes from its own. Weak rather than strong because two ends holding each
/// other strongly is a cycle `Arc` will not collect, and an end whose far side
/// is dropped must report `PeerClosed` rather than leak (D-110).
#[derive(Debug)]
pub(crate) struct Channel {
    inbox: SpinLock<Inbox>,
    waiters: WaitQueue,
    peer: SpinLock<Weak<Object>>,
}

impl Channel {
    /// The most messages one end will hold before refusing.
    ///
    /// Backpressure, not a promise: a sender that fills it is told `Full` and
    /// nothing is dropped. Silently dropping is what io_uring's `CQ_NODROP`
    /// exists to avoid, and the answer is the same here (D-79).
    pub(crate) const DEPTH: usize = 16;

    /// `None` if the inbox's room cannot be taken.
    ///
    /// The room is taken **here**, once, and never again: `DEPTH` is the most
    /// the queue will hold, so a `push_back` below `DEPTH` can never grow the
    /// deque and therefore never reaches the allocator. That is what lets the
    /// send path hold a leaf guard at all — `try_reserve` under one was a
    /// `lock_leaf` violation, and the fix is not to do it rather than to
    /// document it (D-85, D-110).
    fn new() -> Option<Self> {
        let mut messages = VecDeque::new();
        messages.try_reserve_exact(Self::DEPTH).ok()?;
        Some(Self {
            inbox: SpinLock::new(Inbox { messages }),
            waiters: WaitQueue::new(),
            peer: SpinLock::new(Weak::new()),
        })
    }

    /// Put `message` in the far end's inbox, and wake anything waiting there.
    ///
    /// # Errors
    ///
    /// [`ChannelError::PeerClosed`] if the other end is gone,
    /// [`ChannelError::Full`] at [`DEPTH`](Self::DEPTH), and
    /// [`ChannelError::OutOfMemory`].
    pub(crate) fn send(&self, message: Message) -> Result<(), ChannelError> {
        let peer = self
            .peer
            .lock_leaf()
            .upgrade()
            .ok_or(ChannelError::PeerClosed)?;
        let far = peer.as_channel().ok_or(ChannelError::PeerClosed)?;

        {
            let mut inbox = far.inbox.lock_leaf();
            if inbox.messages.len() >= Self::DEPTH {
                return Err(ChannelError::Full);
            }
            // Cannot allocate: `new` reserved `DEPTH` and the refusal above
            // keeps the length below it, so nothing here reaches the heap
            // while a leaf guard is held.
            debug_assert!(inbox.messages.capacity() >= Self::DEPTH);
            inbox.messages.push_back(message);
        }
        far.waiters.wake_all();
        Ok(())
    }

    /// Take the oldest message from this end's inbox, or `None` if it is empty.
    pub(crate) fn receive(&self) -> Option<Message> {
        self.inbox.lock_leaf().messages.pop_front()
    }

    /// Whether anything is waiting to be read here.
    pub(crate) fn has_message(&self) -> bool {
        !self.inbox.lock_leaf().messages.is_empty()
    }

    /// Whether the far end is gone.
    pub(crate) fn is_orphaned(&self) -> bool {
        self.peer.lock_leaf().upgrade().is_none()
    }

    /// Block until a message arrives at this end, then take it.
    ///
    /// Registers before it re-reads, for the reason D-95 gives: the state can
    /// only be re-read once a block returns, and it returns only on a wake, so
    /// a check-then-register loses a message that lands in the window.
    ///
    /// `None` only if the far end closed with nothing left to read.
    pub(crate) fn receive_blocking(&self, context: &mut ThreadContext) -> Option<Message> {
        loop {
            if let Some(message) = self.receive() {
                return Some(message);
            }
            if self.is_orphaned() {
                return None;
            }
            self.waiters
                .wait_unless(context, || self.has_message() || self.is_orphaned());
        }
    }

    /// The queue behind this end, for the check that drives its wait window.
    #[cfg(feature = "selftest")]
    pub(crate) const fn waiters_for_test(&self) -> &WaitQueue {
        &self.waiters
    }

    /// Wait for a message without taking one first.
    ///
    /// [`receive_blocking`](Self::receive_blocking) takes the message before it
    /// ever waits, so it cannot reach the wait with the condition already true
    /// — and that is the one state where registering-then-re-reading and a
    /// plain wait differ. The check needs to reach it, so this exists (D-110).
    #[cfg(feature = "selftest")]
    pub(crate) fn wait_for_message(&self, context: &mut ThreadContext) {
        self.waiters
            .wait_unless(context, || self.has_message() || self.is_orphaned());
    }
}

/// Make a pair of channel ends, each naming the other.
///
/// The only way to make a `Channel`: an end with no peer would be a pipe with
/// one side, and there is no operation that could give it one later.
pub(crate) fn channel() -> Option<(ObjectRef, ObjectRef)> {
    let first: ObjectRef = Arc::new(Object::Channel(Channel::new()?));
    let second: ObjectRef = Arc::new(Object::Channel(Channel::new()?));
    if let (Some(one), Some(two)) = (first.as_channel(), second.as_channel()) {
        *one.peer.lock_leaf() = Arc::downgrade(&second);
        *two.peer.lock_leaf() = Arc::downgrade(&first);
    }
    Some((first, second))
}
