//! The handle table: one global arena, and who owns which slot.
//!
//! A process's power is exactly its handle table (`002` pillar 1), so this is
//! where authority is kept. D-78 settled the shape and `libs/handle-space` owns
//! the arithmetic; what is here is the arena, the lock, and the ownership
//! comparison that turns a decoded slot into a permission.
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
use core::sync::atomic::{AtomicU32, Ordering};

use handle_space::{Mixer, Rights, SLOTS, Slot};
use wost::sync::SpinLock;

use crate::object::{ObjectRef, ObjectType};

/// Which table a slot belongs to.
///
/// The single comparison a lookup makes beyond arithmetic. Tables are numbered
/// rather than addressed so that the arena holds no pointer back to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableId(u32);

/// Why a handle operation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum HandleError {
    /// The value is not a well-formed handle at all.
    Malformed,
    /// It names a slot that is empty, or one whose generation has moved on —
    /// which is what a handle kept across a close looks like.
    Stale,
    /// It names a live slot belonging to another table.
    NotYours,
    /// It does not carry the rights the operation needs.
    Denied,
    /// A right was asked for that means nothing about this kind of object.
    Meaningless,
    /// This table has as many handles as its quota allows.
    QuotaExhausted,
    /// The arena is full.
    ArenaFull,
}

impl core::fmt::Display for HandleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match *self {
            Self::Malformed => "not a handle",
            Self::Stale => "handle names nothing live",
            Self::NotYours => "handle belongs to another table",
            Self::Denied => "handle does not carry the right",
            Self::Meaningless => "right means nothing for this object type",
            Self::QuotaExhausted => "handle quota exhausted",
            Self::ArenaFull => "the handle arena is full",
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
}

impl Arena {
    const fn new() -> Self {
        Self {
            slots: Vec::new(),
            hint: 0,
        }
    }

    /// Take a free slot, growing the arena if every existing one is busy.
    fn take(&mut self) -> Option<usize> {
        let count = self.slots.len();
        for step in 0..count {
            let index = (self.hint + step) % count;
            if self.slots[index].entry.is_none() {
                self.hint = (index + 1) % count;
                return Some(index);
            }
        }
        if count >= SLOTS as usize || count == self.slots.capacity() {
            // Growing means allocating, and allocating under this guard would
            // nest `ARENA → HEAP → FRAMES` — the order D-85 deleted (D-95).
            // The caller reserves outside the lock and comes back.
            return None;
        }
        self.slots.push(ArenaSlot {
            generation: 0,
            entry: None,
        });
        Some(count)
    }
}

static ARENA: SpinLock<Arena> = SpinLock::new(Arena::new());

/// Make sure the arena has room for one more slot, allocating outside its lock.
///
/// The whole reason this is a separate step: `Vec` growth calls the allocator,
/// which takes `HEAP` and possibly `FRAMES`, and doing that under `ARENA`'s
/// guard would rebuild the very lock order D-85 deleted. Reserving first costs
/// a second acquisition and keeps every one of them a leaf (D-95).
///
/// **Growing is two allocator operations, not one**, and only the first was
/// moved out. The handover used to be `arena.slots = grown`, and that
/// assignment drops the vector it replaces — which after `append` still owns
/// its buffer, so the free was a `HEAP` acquisition under this guard. Latent
/// rather than harmless: the first growth hands back a vector with no
/// allocation behind it, so nothing was reached until a boot held sixteen
/// handles at once and none did (D-128).
///
/// # Errors
///
/// [`HandleError::ArenaFull`] when every slot the arena may ever hold exists
/// and is taken.
fn reserve_a_slot() -> Result<(), HandleError> {
    loop {
        let wanted = {
            let arena = ARENA.lock_leaf();
            if arena.slots.iter().any(|slot| slot.entry.is_none())
                || arena.slots.len() < arena.slots.capacity()
            {
                return Ok(());
            }
            if arena.slots.len() >= SLOTS as usize {
                return Err(HandleError::ArenaFull);
            }
            arena.slots.capacity().max(8) * 2
        };

        // Allocated with no lock held, then handed over. Another caller may
        // have grown it meanwhile, which the loop re-checks rather than
        // assuming.
        let room: Vec<ArenaSlot> = Vec::with_capacity(wanted);
        let mut arena = ARENA.lock_leaf();
        // Whichever vector this call is left holding: the arena's old buffer
        // where the handover happened, and the unused room where another
        // caller got there first. Named so it can be freed below rather than
        // where it stops being wanted.
        let discarded = if arena.slots.capacity() < wanted {
            let mut grown = room;
            grown.append(&mut arena.slots);
            core::mem::replace(&mut arena.slots, grown)
        } else {
            room
        };
        // The guard first, then the free. The other order is the one this
        // function exists to prevent.
        drop(arena);
        drop(discarded);
    }
}

/// Numbers tables, so no two share an identity.
static NEXT_TABLE: AtomicU32 = AtomicU32::new(0);

/// A process's authority: the handles it holds, and how many it may hold.
///
/// Not `Clone`: two tables with one identity would be two processes with one
/// set of permissions.
#[derive(Debug)]
pub(crate) struct HandleTable {
    id: TableId,
    mixer: Mixer,
    quota: usize,
    live: usize,
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
    /// distinct-per-table constant is what the property needs. It removes the
    /// accidental case — a value that leaked from one table naming something
    /// in another — which is what it is for.
    pub(crate) fn new(quota: usize) -> Self {
        let id = TableId(NEXT_TABLE.fetch_add(1, Ordering::Relaxed));
        Self {
            id,
            mixer: Mixer::for_table(id.0),
            quota,
            live: 0,
        }
    }

    /// How many handles this table holds.
    pub(crate) const fn live(&self) -> usize {
        self.live
    }

    /// Put `object` in this table with `rights`, and return its handle.
    ///
    /// # Errors
    ///
    /// Rights that mean nothing for this kind of object are refused rather
    /// than granted and ignored — D-78's rule, enforced (D-95).
    ///
    /// # Errors
    ///
    /// [`HandleError::Meaningless`] for a right this object type makes no
    /// distinction about, [`HandleError::QuotaExhausted`] if the table is
    /// full, and [`HandleError::ArenaFull`] if the machine is.
    pub(crate) fn insert(&mut self, object: ObjectRef, rights: Rights) -> Result<u32, HandleError> {
        if !rights.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        if self.live >= self.quota {
            return Err(HandleError::QuotaExhausted);
        }
        // Reserved before the guard, never under it: `Vec` growth calls the
        // allocator, and this lock is a leaf (D-85, D-95). The reservation is
        // not a hold, so another table can consume the room in the window
        // between dropping that guard and taking this one — retried rather
        // than reported as a full machine, which is what it used to say
        // (D-105).
        let index = loop {
            reserve_a_slot()?;
            if let Some(index) = ARENA.lock_leaf().take() {
                break index;
            }
        };
        let mut arena = ARENA.lock_leaf();
        let slot = &mut arena.slots[index];
        let generation = slot.generation;
        slot.entry = Some(Entry {
            owner: self.id,
            object,
            rights,
        });
        drop(arena);

        let Some(named) = Slot::new(index as u32, generation) else {
            // Unreachable: `take` bounds the index by `SLOTS` and the
            // generation is masked on every advance. The recovery is to put
            // the slot back — an earlier version called `close(0)`, which can
            // only ever return `Malformed`, so the slot it claimed to free
            // stayed occupied for ever (D-95).
            let mut arena = ARENA.lock_leaf();
            if let Some(slot) = arena.slots.get_mut(index) {
                slot.entry = None;
            }
            return Err(HandleError::ArenaFull);
        };
        self.live += 1;
        Ok(named.encode(self.mixer))
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
    /// [`HandleError::Malformed`], [`HandleError::Stale`] or
    /// [`HandleError::NotYours`], each naming a different thing that went
    /// wrong, because a caller that cannot tell them apart cannot report
    /// usefully.
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
            return Err(HandleError::NotYours);
        }
        Ok((Arc::clone(&entry.object), entry.rights))
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

/// The delegation half, reached by the checks and by no shipped caller: a
/// program closes, duplicates or sends a handle through synchronous calls that
/// are on D-114's owed list (D-116).
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
    pub(crate) fn duplicate(&mut self, handle: u32, wanted: Rights) -> Result<u32, HandleError> {
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
        &mut self,
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
        &mut self,
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
    pub(crate) fn close(&mut self, handle: u32) -> Result<(), HandleError> {
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
                Some(entry) if entry.owner != self.id => return Err(HandleError::NotYours),
                Some(_) => {}
            }
            slot.generation = named.reused().generation();
            slot.entry.take()
        };
        self.live -= 1;
        drop(released);
        Ok(())
    }
}

#[cfg(feature = "selftest")]
impl HandleTable {
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
}

impl Drop for HandleTable {
    /// A table's death closes everything it held.
    ///
    /// Which is the whole of a process's teardown, once there are processes:
    /// its authority is its handles, so releasing them releases everything it
    /// could reach.
    fn drop(&mut self) {
        // Sized before the guard is taken, because `Vec` growth calls the
        // allocator and this lock is a leaf (D-95). `live` is exactly how many
        // entries will come out, so nothing grows under the lock.
        let mut released: Vec<ObjectRef> = Vec::with_capacity(self.live);
        {
            let mut arena = ARENA.lock_leaf();
            for slot in &mut arena.slots {
                if released.len() == released.capacity() {
                    break;
                }
                if slot
                    .entry
                    .as_ref()
                    .is_some_and(|entry| entry.owner == self.id)
                {
                    slot.generation = handle_space::next_generation(slot.generation);
                    if let Some(entry) = slot.entry.take() {
                        released.push(entry.object);
                    }
                }
            }
        }
        self.live = 0;
        // Released with the guard gone: dropping the last reference to an
        // object can reach the frame allocator.
        drop(released);
    }
}
