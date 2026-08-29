//! The handle table: one global arena, and who owns which slot.
//!
//! A process's external-object authority is exactly its handle table (`002`
//! pillar 1), so this is where handle-carried authority is kept. D-78 settled
//! the shape and `libs/handle-space` owns the arithmetic; what is here is the
//! arena, the lock, and the ownership comparison that turns a decoded slot
//! into a permission.
//!
//! **One arena, not one array per table.** Zircon has no per-process handle
//! array, and two independent lines of reasoning arrive at copying that: the
//! generation tag kills use-after-free on reuse, and the non-dense encoding
//! makes allocation commute, which is what lets this scale later without an
//! interface change (D-70 rule 7).
//!
//! **A table is a refcounted object, not a field of a process.** Zircon's
//! strict per-process invariant was the first casualty of its own
//! compatibility layer, and sharing one later is free if nothing assumes
//! otherwise now (D-78).
//!
//! Lock order: `ARENA` is a leaf (D-85). Everything it does is
//! take-mutate-return, and the object it drops on close is released *after*
//! the guard is gone — because a `Drop` may reach the frame allocator, and a
//! guard held across one is the shape this kernel does not have.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use handle_space::{Mixer, Quota, Rights, SLOTS, Slot};
use wost::sync::SpinLock;

use crate::object::{ObjectRef, ObjectType};

/// Which table a slot belongs to.
///
/// The single comparison a lookup makes beyond arithmetic. Tables are numbered
/// rather than addressed so that the arena holds no pointer back to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableId(u64);

/// Why a handle operation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum HandleError {
    /// The value is not a well-formed handle at all.
    Malformed,
    /// It names no live entry belonging to this table.
    ///
    /// Empty, reused and foreign slots deliberately have one answer. Keeping
    /// the ownership check while collapsing its diagnostic removes the
    /// global-arena occupancy oracle without letting authority cross (D-222).
    Stale,
    /// It does not carry the rights the operation needs.
    Denied,
    /// A right was asked for that means nothing about this kind of object.
    Meaningless,
    /// This table has as many handles as its quota allows.
    QuotaExhausted,
    /// Committing this table's quota would exceed the arena's maximum.
    ArenaFull,
    /// The arena could not allocate backing for this table's quota.
    OutOfMemory,
    /// Every non-repeating table identity has been issued.
    IdentityExhausted,
}

impl core::fmt::Display for HandleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match *self {
            Self::Malformed => "not a handle",
            Self::Stale => "handle names nothing live in this table",
            Self::Denied => "handle does not carry the right",
            Self::Meaningless => "right means nothing for this object type",
            Self::QuotaExhausted => "handle quota exhausted",
            Self::ArenaFull => "the handle arena is full",
            Self::OutOfMemory => "no memory for the handle arena",
            Self::IdentityExhausted => "handle table identities exhausted",
        })
    }
}

impl core::error::Error for HandleError {}

/// What one arena slot holds while it is live.
#[derive(Debug)]
struct Entry {
    owner: TableId,
    object: ObjectRef,
    rights: Rights,
}

/// One slot of the global arena.
///
/// The generation advances on every *close*, not on every open, so a value
/// handed out is invalidated the moment its handle is closed rather than when
/// the slot is next used. A slot that is freed and never reused still refuses
/// its old handle.
#[derive(Debug)]
struct ArenaSlot {
    generation: u32,
    entry: Option<Entry>,
}

/// The arena every table's handles live in.
struct Arena {
    slots: Vec<ArenaSlot>,
    /// Where the next search starts. Deliberately not "lowest free": the
    /// encoding forbids a dense promise and so does this (D-70 rule 7).
    hint: usize,
    /// Sum of every live table's quota, never greater than `slots.capacity()`.
    committed: usize,
    /// The next never-reused ownership identity, or the exhausted sentinel.
    next_table: u64,
}

impl Arena {
    const fn new() -> Self {
        Self {
            slots: Vec::new(),
            hint: 0,
            committed: 0,
            next_table: 0,
        }
    }

    /// Take a free slot **and fill it** without allocating.
    ///
    /// One call, under one guard, and that is the whole of the fix. `take`
    /// used to find a slot and return its index without marking it, and
    /// `insert` then dropped the guard and re-acquired it to write the entry.
    /// Two callers interleaving in that window both received the same index —
    /// `take` scans for `entry.is_none()`, which was still true — and the
    /// second write **overwrote a live entry, dropping it in place under the
    /// leaf guard**. An `ObjectRef` dropped there can reach `Channel::drop`
    /// and then `THREADS`, which is the lock-under-a-leaf class D-121, D-126
    /// and D-128 exist for (D-141).
    ///
    /// The room was committed when the table was constructed: growing here
    /// would allocate in trap context and under the `ARENA` leaf (D-182).
    fn claim(&mut self, entry: Entry) -> Result<(usize, u32), Entry> {
        let Some(index) = self.find_free() else {
            return Err(entry);
        };
        let slot = &mut self.slots[index];
        let generation = slot.generation;
        slot.entry = Some(entry);
        Ok((index, generation))
    }

    /// A free slot's index, extending only into already allocated capacity.
    fn find_free(&mut self) -> Option<usize> {
        let count = self.slots.len();
        for step in 0..count {
            let index = (self.hint + step) % count;
            if self.slots[index].entry.is_none() {
                self.hint = (index + 1) % count;
                return Some(index);
            }
        }
        if count >= SLOTS as usize || count == self.slots.capacity() {
            // Reaching this after a quota charge violates the commitment
            // invariant; the entry is returned so it can drop outside the
            // leaf rather than making the invariant failure a lock inversion.
            return None;
        }
        self.slots.push(ArenaSlot {
            generation: 0,
            entry: None,
        });
        Some(count)
    }

    /// Required commitment for one more table while an identity remains.
    fn table_requirement(&self, quota: usize) -> Result<(usize, u64), HandleError> {
        let successor = handle_space::next_table_identity(self.next_table)
            .ok_or(HandleError::IdentityExhausted)?;
        let required = self
            .committed
            .checked_add(quota)
            .ok_or(HandleError::ArenaFull)?;
        if required > SLOTS as usize {
            return Err(HandleError::ArenaFull);
        }
        Ok((required, successor))
    }

    /// Commit quota and its unique owner as one arena transaction.
    fn commit_table(&mut self, required: usize, successor: u64) -> TableId {
        let id = TableId(self.next_table);
        self.next_table = successor;
        self.committed = required;
        id
    }
}

static ARENA: SpinLock<Arena> = SpinLock::new(Arena::new());

/// Commit backing for a table's entire quota before the table is reachable.
///
/// Ring operations run in trap context, where the allocator correctly refuses
/// re-entry. Charging the whole quota here makes every later insertion
/// allocation-free: arena capacity is at least the sum of live table quotas,
/// which is at least the live handles and promised slots (D-182).
///
/// Construction can race construction. A candidate buffer is allocated with
/// no lock held, then the required commitment is re-read. Whichever buffer is
/// not retained is freed only after the leaf guard has gone.
///
/// # Errors
///
/// [`HandleError::ArenaFull`] if the combined quotas exceed the arena,
/// [`HandleError::OutOfMemory`] if backing cannot be allocated, and
/// [`HandleError::IdentityExhausted`] if no unique owner remains.
fn commit_table(quota: usize) -> Result<TableId, HandleError> {
    loop {
        let wanted = {
            let mut arena = ARENA.lock_leaf();
            let (required, successor) = arena.table_requirement(quota)?;
            if arena.slots.capacity() >= required {
                return Ok(arena.commit_table(required, successor));
            }
            required
        };

        let mut room = Vec::new();
        if room.try_reserve_exact(wanted).is_err() {
            // Another constructor may have installed enough backing while
            // this allocation was attempted. Recheck before reporting an
            // exhaustion the machine has already resolved.
            let mut arena = ARENA.lock_leaf();
            let (required, successor) = arena.table_requirement(quota)?;
            if arena.slots.capacity() >= required {
                return Ok(arena.commit_table(required, successor));
            }
            return Err(HandleError::OutOfMemory);
        }

        let mut arena = ARENA.lock_leaf();
        let (required, successor) = arena.table_requirement(quota)?;
        if arena.slots.capacity() >= required {
            let id = arena.commit_table(required, successor);
            drop(arena);
            drop(room);
            return Ok(id);
        }
        if room.capacity() < required {
            drop(arena);
            drop(room);
            continue;
        }

        room.append(&mut arena.slots);
        let id = arena.commit_table(required, successor);
        let discarded = core::mem::replace(&mut arena.slots, room);
        drop(arena);
        drop(discarded);
        return Ok(id);
    }
}

/// A process's authority: the handles it holds, and how many it may hold.
///
/// Not `Clone`: two tables with one identity would be two processes with one
/// set of permissions.
#[derive(Debug)]
pub(crate) struct HandleTable {
    id: TableId,
    mixer: Mixer,
    quota: usize,
    /// Handles held and slots promised, packed by [`Quota`].
    ///
    /// An atomic rather than a plain field, and that is what makes the table
    /// usable through an `Arc` after the program is running. Every method
    /// below takes `&self`, so a completion can install a handle into a table
    /// the spawner long since gave away — which is the whole of D-151 and was
    /// impossible while the count needed `&mut` (D-163).
    counts: AtomicU64,
}

/// The half every spawned program uses: `user::spawn` builds a table and puts
/// its bootstrap channel in one, and `user::orphan` asks what that handle named
/// on the way out. It carried an `#[allow(dead_code)]` saying the shipped
/// kernel reached none of this, which stopped being true when D-116 made the
/// shipped kernel run a program — and an `allow` that stops being needed says
/// nothing (D-130).
impl HandleTable {
    /// A table holding nothing, able to hold `quota` handles.
    ///
    /// The quota is D-44's inherited obligation, answered here rather than at
    /// the frame allocator: a process's kernel memory is bounded by what its
    /// table can name. It is a policy number and stays revisable, which is
    /// exactly why D-44 refused to make it an ABI decision instead.
    ///
    /// The mixer is derived from the identity rather than from entropy: there
    /// is no entropy source until `RDSEED` arrives with the drivers, and a
    /// table-derived constant is what the property needs. Its 32 bits may
    /// repeat while the 64-bit owner never does; it removes the accidental
    /// case, while the owner comparison remains the authority boundary
    /// (D-204).
    ///
    /// # Errors
    ///
    /// [`HandleError::ArenaFull`] if live table quotas leave no room,
    /// [`HandleError::OutOfMemory`] if the quota cannot be backed, and
    /// [`HandleError::IdentityExhausted`] if every unique owner was issued.
    pub(crate) fn new(quota: usize) -> Result<Self, HandleError> {
        let id = commit_table(quota)?;
        Ok(Self {
            id,
            mixer: Mixer::for_table(id.0 as u32),
            quota,
            counts: AtomicU64::new(Quota::EMPTY),
        })
    }

    /// How many handles this table holds.
    ///
    /// Held, not charged: a promise is not yet a handle, and a check counting
    /// what a program can *reach* must not count what it has been promised.
    pub(crate) fn live(&self) -> usize {
        Quota::live(self.counts.load(Ordering::Acquire)) as usize
    }

    /// Slots promised to operations that have not finished.
    #[cfg(feature = "selftest")]
    pub(crate) fn promised(&self) -> usize {
        Quota::reserved(self.counts.load(Ordering::Acquire)) as usize
    }

    /// Apply one [`Quota`] transition, or report that it was refused.
    ///
    /// A compare-and-exchange loop because the two counts share a word: moving
    /// a promise to a handle is one write, so no reader ever sees a table that
    /// holds neither.
    fn step(&self, transition: impl Fn(u64) -> Option<u64>) -> bool {
        let mut seen = self.counts.load(Ordering::Acquire);
        loop {
            let Some(next) = transition(seen) else {
                return false;
            };
            match self
                .counts
                .compare_exchange_weak(seen, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return true,
                Err(current) => seen = current,
            }
        }
    }

    /// Promise a slot to an operation being admitted.
    ///
    /// The reservation is a value with a destructor, so the promise is given
    /// back on **every** path out — refused, cancelled, or dropped on the way
    /// through a `?`. D-151 asks for exactly that and names io_uring's
    /// `-ENFILE`-at-completion as the shape of getting it wrong; making it a
    /// `Drop` obligation rather than a rule means the paths do not have to be
    /// enumerated in order to be covered (D-163).
    ///
    /// # Errors
    ///
    /// [`HandleError::QuotaExhausted`] when the table is full, which is the
    /// answer at *submission* — the one moment the program can act on it.
    pub(crate) fn reserve_slot(self: &Arc<Self>) -> Result<SlotReservation, HandleError> {
        if !self.step(|bits| Quota::reserve(bits, self.quota)) {
            return Err(HandleError::QuotaExhausted);
        }
        Ok(SlotReservation {
            table: Arc::clone(self),
            spent: false,
        })
    }

    /// Put `object` in this table with `rights`, and return its handle.
    ///
    /// Rights that mean nothing for this kind of object are refused rather
    /// than granted and ignored — D-78's rule, enforced (D-95).
    ///
    /// # Errors
    ///
    /// [`HandleError::Meaningless`] for a right this object type makes no
    /// distinction about, [`HandleError::QuotaExhausted`] if the table is full,
    /// and [`HandleError::ArenaFull`] only if the backing invariant is broken.
    pub(crate) fn insert(&self, object: ObjectRef, rights: Rights) -> Result<u32, HandleError> {
        if !rights.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        // The charge comes first, so the commitment invariant guarantees the
        // arena claim that follows and an invariant refusal can put it back.
        if !self.step(|bits| Quota::take(bits, self.quota)) {
            return Err(HandleError::QuotaExhausted);
        }
        match self.fill(object, rights) {
            Ok(handle) => Ok(handle),
            Err(error) => {
                self.step(Quota::close);
                Err(error)
            }
        }
    }

    /// Put `object` in a slot this table has already been charged for.
    ///
    /// Touches no count. Its two callers each own the accounting:
    /// [`insert`](Self::insert) has taken the room outright, and
    /// [`install`](Self::install) holds a promise.
    fn fill(&self, object: ObjectRef, rights: Rights) -> Result<u32, HandleError> {
        let entry = Entry {
            owner: self.id,
            object,
            rights,
        };
        let claimed = {
            let mut arena = ARENA.lock_leaf();
            arena.claim(entry)
        };
        debug_assert!(
            claimed.is_ok(),
            "a charged handle had no committed arena slot"
        );
        let (index, generation) = match claimed {
            Ok(found) => found,
            Err(entry) => {
                drop(entry);
                return Err(HandleError::ArenaFull);
            }
        };

        let Some(named) = Slot::new(index as u32, generation) else {
            // Unreachable: `take` bounds the index by `SLOTS` and the
            // generation is masked on every advance. The recovery is to put
            // the slot back — an earlier version called `close(0)`, which can
            // only ever return `Malformed`, so the slot it claimed to free
            // stayed occupied for ever (D-95).
            let released = ARENA
                .lock_leaf()
                .slots
                .get_mut(index)
                .and_then(|slot| slot.entry.take());
            drop(released);
            return Err(HandleError::ArenaFull);
        };
        Ok(named.encode(self.mixer))
    }

    /// Install a handle into a slot promised earlier, attenuating on arrival.
    ///
    /// The receiving half of D-110's rule, applied to the ring: `offered` is
    /// what the operation produced, `declared` is what the program said it was
    /// willing to hold, and what lands is the **narrower of the two**. Without
    /// it the ring would be the widening path around the discipline the
    /// channels keep, which is the one thing that would make it worse than
    /// them (D-151).
    ///
    /// # Errors
    ///
    /// [`HandleError::Denied`] if `declared` asks for more than was offered,
    /// [`HandleError::Meaningless`] for a right the object makes no distinction
    /// about, and [`HandleError::ArenaFull`] only if the backing invariant is
    /// broken. On every refusal the promise goes back, because the reservation
    /// is taken by value and its destructor runs.
    pub(crate) fn install(
        &self,
        reservation: SlotReservation,
        object: ObjectRef,
        offered: Rights,
        declared: Rights,
    ) -> Result<u32, HandleError> {
        let granted = offered.attenuate(declared).ok_or(HandleError::Denied)?;
        if !granted.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        let handle = self.fill(object, granted)?;
        reservation.keep();
        Ok(handle)
    }

    /// What `handle` names here, if it names anything of this table's.
    ///
    /// The whole of a lookup: decode, bounds, generation, and **one ownership
    /// comparison**. That last is what makes a global arena safe to share —
    /// a well-formed handle from another table decodes to a slot that is not
    /// this table's, and is refused rather than honoured.
    ///
    /// # Errors
    ///
    /// [`HandleError::Malformed`] if the encoding is not a handle, or
    /// [`HandleError::Stale`] if it names no live entry belonging to this
    /// table. Empty, reused and foreign slots intentionally share the latter
    /// result so this global arena reveals no occupancy bit (D-222).
    pub(crate) fn resolve(&self, handle: u32) -> Result<(ObjectRef, Rights), HandleError> {
        let named = Slot::decode(handle, self.mixer).ok_or(HandleError::Malformed)?;
        let arena = ARENA.lock_leaf();
        let slot = arena
            .slots
            .get(named.index() as usize)
            .ok_or(HandleError::Stale)?;
        if slot.generation != named.generation() {
            return Err(HandleError::Stale);
        }
        let entry = slot.entry.as_ref().ok_or(HandleError::Stale)?;
        if entry.owner != self.id {
            return Err(HandleError::Stale);
        }
        Ok((entry.object.clone(), entry.rights))
    }

    /// Resolve, and require `wanted` as well.
    ///
    /// # Errors
    ///
    /// As [`resolve`](Self::resolve), plus [`HandleError::Denied`].
    pub(crate) fn resolve_with(
        &self,
        handle: u32,
        wanted: Rights,
    ) -> Result<ObjectRef, HandleError> {
        let (object, rights) = self.resolve(handle)?;
        if !rights.contains(wanted) {
            return Err(HandleError::Denied);
        }
        Ok(object)
    }

    /// What kind of thing `handle` names, without taking a reference to it.
    ///
    /// Needs `INSPECT`, because "what is this" is a distinction the kernel
    /// makes about the object and therefore a right (D-78's rule).
    ///
    /// # Errors
    ///
    /// As [`resolve_with`](Self::resolve_with).
    pub(crate) fn kind_of(&self, handle: u32) -> Result<ObjectType, HandleError> {
        self.resolve_with(handle, Rights::INSPECT)
            .map(|object| object.kind())
    }
}

/// The delegation methods do not have one reachability state: Duplicate is a
/// shipped ring opcode, while user close and channel transfer still have no
/// ring-3 operations (D-181).
#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
impl HandleTable {
    /// Copy `handle` into a second handle carrying `wanted` or fewer rights.
    ///
    /// Needs `DUPLICATE`, and **attenuation only narrows**: asking for a right
    /// the original does not hold is refused rather than clamped, because a
    /// clamp turns "give me more than I have" into a silent success.
    ///
    /// # Errors
    ///
    /// As [`resolve`](Self::resolve), plus [`HandleError::Denied`] if
    /// `DUPLICATE` is absent or `wanted` exceeds what is held, and
    /// [`HandleError::QuotaExhausted`].
    pub(crate) fn duplicate(&self, handle: u32, wanted: Rights) -> Result<u32, HandleError> {
        let (object, rights) = self.resolve(handle)?;
        if !rights.contains(Rights::DUPLICATE) {
            return Err(HandleError::Denied);
        }
        let granted = rights.attenuate(wanted).ok_or(HandleError::Denied)?;
        self.insert(object, granted)
    }

    /// Take `handle` out of this table to be sent, with `declared` rights.
    ///
    /// A **move**, not a copy: the entry is gone from this table when this
    /// returns, and if the send that follows fails the object is dropped
    /// rather than silently staying behind. A handle that survived a send
    /// would make delegation into duplication, which is the property `002`'s
    /// pillar 1 rests on.
    ///
    /// Three refusals, and each is a separate claim (D-110):
    ///
    /// - **`TRANSFER` is required.** Stripping it is how a holder is given
    ///   authority it cannot pass on, which costs one bit and is the cheapest
    ///   confinement this model has.
    /// - **`declared` must be a subset of what is held.** Attenuation only
    ///   narrows; a send cannot mint authority the sender never had.
    /// - **`declared` must mean something for this object type.** D-78's rule
    ///   checked at the boundary where it is easiest to violate.
    ///
    /// Refused at the **sender**, not at the receiver. A receiver that had to
    /// police this would need to know what the sender held, which it cannot.
    ///
    /// # Errors
    ///
    /// [`HandleError::Denied`] for any of the first two, and
    /// [`HandleError::Meaningless`] for the third.
    pub(crate) fn send(
        &self,
        handle: u32,
        declared: Rights,
    ) -> Result<(ObjectRef, Rights), HandleError> {
        let (object, held) = self.resolve(handle)?;
        if !held.contains(Rights::TRANSFER) {
            return Err(HandleError::Denied);
        }
        let granted = held.attenuate(declared).ok_or(HandleError::Denied)?;
        if !granted.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        self.close(handle)?;
        Ok((object, granted))
    }

    /// Take a handle that arrived in a message, with `declared` rights.
    ///
    /// The receiving half of the same rule, and the reason both ends declare
    /// (managarm's `PushDescriptor`/`PullDescriptor`): a receiver states what
    /// it is willing to hold, and gets the narrower of the two. Neither end
    /// can be surprised by the other.
    ///
    /// # Errors
    ///
    /// [`HandleError::Denied`] if `declared` asks for more than was offered,
    /// plus whatever [`insert`](Self::insert) refuses.
    pub(crate) fn accept(
        &self,
        object: ObjectRef,
        offered: Rights,
        declared: Rights,
    ) -> Result<u32, HandleError> {
        let granted = offered.attenuate(declared).ok_or(HandleError::Denied)?;
        self.insert(object, granted)
    }

    /// Close `handle`, dropping this table's reference to what it names.
    ///
    /// The generation advances here, so the value is dead the instant it is
    /// closed rather than when the slot is next handed out. A slot freed and
    /// never reused still refuses its old handle.
    ///
    /// The object is released **after** the arena's guard is gone: dropping
    /// the last reference to one may reach the frame allocator, and a guard
    /// held across that is the nesting this kernel does not have (D-85).
    ///
    /// # Errors
    ///
    /// As [`resolve`](Self::resolve).
    pub(crate) fn close(&self, handle: u32) -> Result<(), HandleError> {
        let named = Slot::decode(handle, self.mixer).ok_or(HandleError::Malformed)?;
        let released = {
            let mut arena = ARENA.lock_leaf();
            let slot = arena
                .slots
                .get_mut(named.index() as usize)
                .ok_or(HandleError::Stale)?;
            if slot.generation != named.generation() {
                return Err(HandleError::Stale);
            }
            match slot.entry.as_ref() {
                None => return Err(HandleError::Stale),
                Some(entry) if entry.owner != self.id => return Err(HandleError::Stale),
                Some(_) => {}
            }
            slot.generation = named.reused().generation();
            slot.entry.take()
        };
        self.step(Quota::close);
        drop(released);
        Ok(())
    }
}

/// A slot promised to an operation that has been admitted and has not finished.
///
/// Exists so that "the reservation is returned on every terminal path" is a
/// property of the type rather than a rule the paths have to remember. There
/// is no way to lose one silently: [`keep`](Self::keep) spends it, and every
/// other end — a refusal, a cancellation, a `?` on the way through, the death
/// of the program holding it — runs the destructor, which gives it back.
///
/// It holds an `Arc` to the table rather than a borrow, because the operation
/// outlives the call that admitted it and may outlive the thread that made it
/// (D-153 is slice 3, and this is the shape that makes it possible).
#[derive(Debug)]
pub(crate) struct SlotReservation {
    table: Arc<HandleTable>,
    spent: bool,
}

impl SlotReservation {
    /// Turn the promise into a held handle.
    ///
    /// By value, so a reservation cannot be kept twice.
    fn keep(mut self) {
        self.table.step(Quota::commit);
        self.spent = true;
    }
}

impl Drop for SlotReservation {
    fn drop(&mut self) {
        if !self.spent {
            self.table.step(Quota::release);
        }
    }
}

#[cfg(feature = "selftest")]
impl HandleTable {
    /// Arena commitment and capacity, for the backing invariant check.
    pub(crate) fn arena_backing_for_test() -> (usize, usize) {
        let arena = ARENA.lock_leaf();
        (arena.committed, arena.slots.capacity())
    }

    /// Committed quotas and occupied slots in the shared arena.
    pub(crate) fn arena_usage_for_test() -> (usize, usize) {
        let arena = ARENA.lock_leaf();
        (
            arena.committed,
            arena
                .slots
                .iter()
                .filter(|slot| slot.entry.is_some())
                .count(),
        )
    }

    /// The slot `handle` names here, whether or not it is live.
    pub(crate) fn slot_of(&self, handle: u32) -> Option<Slot> {
        Slot::decode(handle, self.mixer)
    }

    /// The handle this table *would* use for `slot`.
    ///
    /// Test support, and it exists because without it the cross-table check
    /// could not reach the ownership comparison at all: two tables' mixers
    /// differ, so one's handle decodes in the other to an index that is almost
    /// never in the arena, and the bounds check refuses it first (D-95).
    pub(crate) fn handle_for(&self, slot: Slot) -> u32 {
        slot.encode(self.mixer)
    }

    /// This table's non-repeating ownership identity.
    pub(crate) fn identity_for_test(&self) -> u64 {
        self.id.0
    }

    /// Run one check with the identity sequence at a controlled boundary.
    ///
    /// The check must drop every table it constructs before returning. Machine
    /// self-tests are single-driver, so no other constructor races this swap.
    pub(crate) fn with_next_identity_for_test<T>(next: u64, check: impl FnOnce() -> T) -> T {
        let previous = {
            let mut arena = ARENA.lock_leaf();
            core::mem::replace(&mut arena.next_table, next)
        };
        let result = check();
        ARENA.lock_leaf().next_table = previous;
        result
    }
}

impl Drop for HandleTable {
    /// A table's death closes everything it held.
    ///
    /// Which is the whole of a process's teardown, once there are processes:
    /// its authority is its handles, so releasing them releases everything it
    /// could reach.
    fn drop(&mut self) {
        // One at a time avoids a temporary allocation, and each object's
        // destructor runs after the arena leaf has gone (D-182).
        let mut next: usize = 0;
        while next < SLOTS as usize {
            let released = {
                let mut arena = ARENA.lock_leaf();
                let mut released = None;
                for (index, slot) in arena.slots.iter_mut().enumerate().skip(next) {
                    next = index + 1;
                    if slot
                        .entry
                        .as_ref()
                        .is_some_and(|entry| entry.owner == self.id)
                    {
                        slot.generation = handle_space::next_generation(slot.generation);
                        released = slot.entry.take().map(|entry| entry.object);
                        break;
                    }
                }
                released
            };
            if let Some(object) = released {
                drop(object);
            } else {
                break;
            }
        }

        {
            let mut arena = ARENA.lock_leaf();
            debug_assert!(
                arena.committed >= self.quota,
                "a table released quota the arena had not committed"
            );
            if arena.committed >= self.quota {
                arena.committed -= self.quota;
            }
        }
        self.counts.store(Quota::EMPTY, Ordering::Release);
    }
}
