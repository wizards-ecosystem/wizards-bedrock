//! The things a handle can name.
//!
//! `002` sketches the eventual object model; four handle-object types exist
//! today — events, channels, files and directories — and later phases add
//! types only with callers. Each is exposed to a program only through a handle
//! carrying rights (pillar 1), so the type here is deliberately not `pub`
//! beyond the crate: nothing outside the handle table should hold one directly.
//!
//! Stateful objects use `Arc`: D-78 wants `inc_not_zero` from day one so that
//! a lookup racing a close cannot resurrect a corpse, and `Weak::upgrade` is
//! exactly that operation with the atomics already audited. Copyable VFS file
//! and directory identities live directly in an object reference; wrapping
//! one must not turn an OpenAt completion into an allocation.

use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::fmt;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

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
    /// A file: something with bytes in it.
    File,
    /// A directory: something with names in it.
    Directory,
}

impl ObjectType {
    /// The rights that mean something for this kind of object.
    ///
    /// Beyond the universal four, which mean the same for everything. D-78's
    /// rule made checkable: a right must name a distinction the kernel makes
    /// about *this* object. An event exposes no type-specific data operation,
    /// so only the universal rights apply to one (D-95).
    pub(crate) const fn per_type_rights(self) -> Rights {
        match self {
            Self::Event => Rights::NONE,
            // A channel end is read from and written to, and those are
            // distinctions the kernel makes: a send needs one and a receive
            // the other. They are not syscall names, which is the rule D-78
            // states and WASI's rights violated.
            Self::Channel => Rights::READ.union(Rights::WRITE),
            // WRITE, EXECUTE and MAP stay out until an operation consults
            // them. Admitting an inert right would violate D-78 even if no
            // current path happens to grant it.
            Self::File => Rights::READ,
            // MODIFY is likewise only a raw reserved bit while the namespace
            // has no mutating operation.
            Self::Directory => Rights::TRAVERSE.union(Rights::ENUMERATE),
        }
    }
}

/// Name a file with an object reference, ready for a handle table.
pub(crate) const fn file(file: crate::vfs::File) -> ObjectRef {
    ObjectRef::File(file)
}

/// Name a directory with an object reference.
pub(crate) const fn directory(directory: crate::vfs::Directory) -> ObjectRef {
    ObjectRef::Directory(directory)
}

/// A kernel object.
#[derive(Debug)]
#[non_exhaustive]
#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
pub(crate) enum Object {
    /// See [`Event`].
    Event(Event),
    /// See [`Channel`].
    Channel(Channel),
}

#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
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

/// Reached by checks and not by a shipped object operation. No ring opcode
/// creates, signals or waits on an Event yet; the implementation remains in
/// the image so the checks exercise the code a future caller would reach.
#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
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

/// Something a handle can name.
///
/// Events and channels carry mutable state and therefore share a counted
/// object. A file or directory is already a copyable VFS identity, so keeping
/// it inline makes constructing a completion allocation-free.
#[derive(Debug, Clone)]
pub(crate) enum ObjectRef {
    /// A stateful, reference-counted object.
    Shared(Arc<Object>),
    /// A file in some filesystem.
    ///
    /// **The thinner of `014`'s two options**: what names a backend and a node
    /// within it, rather than an inode-like structure the VFS owns. The fatter
    /// one is what a write path and a page cache both want, and this phase has
    /// neither — building it now would be building for a caller that does not
    /// exist (D-41, D-173).
    File(crate::vfs::File),
    /// A directory in some filesystem.
    Directory(crate::vfs::Directory),
}

impl ObjectRef {
    /// What kind of object this is.
    pub(crate) fn kind(&self) -> ObjectType {
        match self {
            Self::Shared(object) => object.kind(),
            Self::File(_) => ObjectType::File,
            Self::Directory(_) => ObjectType::Directory,
        }
    }

    /// Stable identity of this shared object for one machine check.
    #[cfg(feature = "selftest")]
    pub(crate) fn identity_for_test(&self) -> Option<usize> {
        match self {
            Self::Shared(object) => Some(Arc::as_ptr(object) as usize),
            Self::File(_) | Self::Directory(_) => None,
        }
    }

    /// Stable identity of this channel's live peer for one machine check.
    #[cfg(feature = "selftest")]
    pub(crate) fn channel_peer_identity_for_test(&self) -> Option<usize> {
        let channel = self.as_channel()?;
        channel
            .peer
            .lock_leaf()
            .upgrade()
            .map(|peer| Arc::as_ptr(&peer) as usize)
    }

    /// The event inside, if that is what this is.
    #[cfg_attr(not(feature = "selftest"), expect(dead_code))]
    pub(crate) fn as_event(&self) -> Option<&Event> {
        match self {
            Self::Shared(object) => object.as_event(),
            Self::File(_) | Self::Directory(_) => None,
        }
    }

    /// The channel end inside, if that is what this is.
    #[cfg_attr(not(feature = "selftest"), expect(dead_code))]
    pub(crate) fn as_channel(&self) -> Option<&Channel> {
        match self {
            Self::Shared(object) => object.as_channel(),
            Self::File(_) | Self::Directory(_) => None,
        }
    }

    /// The file inside, if that is what this is.
    pub(crate) const fn as_file(&self) -> Option<&crate::vfs::File> {
        match self {
            Self::File(file) => Some(file),
            Self::Shared(_) | Self::Directory(_) => None,
        }
    }

    /// The directory inside, if that is what this is.
    pub(crate) const fn as_directory(&self) -> Option<&crate::vfs::Directory> {
        match self {
            Self::Directory(directory) => Some(directory),
            Self::Shared(_) | Self::File(_) => None,
        }
    }

    /// A weak reference for lifetime checks on stateful objects.
    #[cfg(feature = "selftest")]
    pub(crate) fn downgrade_for_test(&self) -> Option<Weak<Object>> {
        match self {
            Self::Shared(object) => Some(Arc::downgrade(object)),
            Self::File(_) | Self::Directory(_) => None,
        }
    }
}

/// Make a new event, ready to be put in a handle table.
#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
pub(crate) fn event() -> ObjectRef {
    ObjectRef::Shared(Arc::new(Object::Event(Event::new())))
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

/// Reached by channel checks and not by ring 3. Phase 7 needs real asynchronous
/// send and receive operations before a program can construct or consume one;
/// the implementation remains in the image so those mechanics stay exercised.
#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
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
#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
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
#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
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
#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
pub(crate) struct Channel {
    inbox: SpinLock<Inbox>,
    waiters: WaitQueue,
    peer: SpinLock<Weak<Object>>,
}

static LIVE_CHANNELS: AtomicUsize = AtomicUsize::new(0);

/// As [`HandleTable`](crate::handle::HandleTable)'s: the checks reach these
/// and the shipped kernel has no syscall that does yet (D-116).
#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
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
        LIVE_CHANNELS.fetch_add(1, Ordering::AcqRel);
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
    /// [`ChannelError::PeerClosed`] if the other end is gone, and
    /// [`ChannelError::Full`] at [`DEPTH`](Self::DEPTH).
    ///
    /// **Not `OutOfMemory`**, which this listed and cannot return: the inbox
    /// is reserved to `DEPTH` at construction precisely so `push_back` never
    /// reaches the allocator under a leaf guard, and the comment three lines
    /// into the body already said so (D-145).
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
                // **Drained again, not simply refused.** The take above and
                // this test are two separate lock acquisitions with interrupts
                // on between them, so a peer can send *and then* close in the
                // window — and returning `None` there loses a message that is
                // sitting in the queue, while the doc says `None` means the
                // far end closed with nothing left to read (D-145).
                return self.receive();
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

impl Drop for Channel {
    /// Tell the far end, which is now waiting on a peer that does not exist.
    ///
    /// [`receive_blocking`](Channel::receive_blocking) promises `None` when the
    /// other end is gone, and [`WaitQueue::wait_unless`] re-reads its condition
    /// only after a wake — so without this the promise held for a receiver that
    /// had not blocked yet and for no other. One that had was never told, and
    /// slept for the rest of the boot holding its slot. `user::reclaim` drops a
    /// dead program's handle table so that the parent's end learns its peer is
    /// gone, and that was the sentence this makes true (D-127).
    ///
    /// `get_mut` rather than a guard, and it is not a convenience: a `Drop`
    /// runs wherever the last reference dies, so a lock taken here is a lock
    /// taken under whatever that caller happened to be holding — which is the
    /// one thing `lock_leaf` forbids (D-121, D-126). Nothing is locked here at
    /// all.
    ///
    /// The recursion is two deep and stops: reaching the peer takes a strong
    /// reference, and releasing it can destroy the peer in turn — whose own
    /// drop finds this end's count already at zero and upgrades nothing.
    fn drop(&mut self) {
        let peer = self.peer.get_mut().upgrade();
        if let Some(far) = peer.as_ref().and_then(|object| object.as_channel()) {
            far.waiters.wake_all();
        }
        LIVE_CHANNELS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Channel ends whose storage is still live.
#[cfg(feature = "selftest")]
pub(crate) fn live_channels_for_test() -> usize {
    LIVE_CHANNELS.load(Ordering::Acquire)
}

/// Make a pair of channel ends, each naming the other.
///
/// The only way to make a `Channel`: an end with no peer would be a pipe with
/// one side, and there is no operation that could give it one later.
///
/// `None` rather than an unlinked pair if the two allocations somehow do not
/// hold what they were just built from. Unreachable, and written as a refusal
/// anyway: the alternative returned two ends that satisfy the type and violate
/// the invariant, and every `send` on one would report `PeerClosed` for a peer
/// that was never opened — a failure arriving arbitrarily far from its cause
/// (D-127).
pub(crate) fn channel() -> Option<(ObjectRef, ObjectRef)> {
    let first = Arc::new(Object::Channel(Channel::new()?));
    let second = Arc::new(Object::Channel(Channel::new()?));
    let (Some(one), Some(two)) = (first.as_channel(), second.as_channel()) else {
        return None;
    };
    *one.peer.lock_leaf() = Arc::downgrade(&second);
    *two.peer.lock_leaf() = Arc::downgrade(&first);
    Some((ObjectRef::Shared(first), ObjectRef::Shared(second)))
}
