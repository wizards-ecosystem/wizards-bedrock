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
        if count >= SLOTS as usize {
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
            mixer: Mixer::new(id.0.wrapping_mul(0x9E37_79B9)),
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
    /// [`HandleError::QuotaExhausted`] if the table is full, and
    /// [`HandleError::ArenaFull`] if the machine is.
    pub(crate) fn insert(&mut self, object: ObjectRef, rights: Rights) -> Result<u32, HandleError> {
        if self.live >= self.quota {
            return Err(HandleError::QuotaExhausted);
        }
        let mut arena = ARENA.lock_leaf();
        let index = arena.take().ok_or(HandleError::ArenaFull)?;
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
            // generation is masked on every advance. Closing rather than
            // panicking keeps the arena consistent if that ever changes.
            let _ = self.close(0);
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

impl Drop for HandleTable {
    /// A table's death closes everything it held.
    ///
    /// Which is the whole of a process's teardown, once there are processes:
    /// its authority is its handles, so releasing them releases everything it
    /// could reach.
    fn drop(&mut self) {
        let released = {
            let mut arena = ARENA.lock_leaf();
            let mut released = Vec::new();
            for slot in &mut arena.slots {
                if slot
                    .entry
                    .as_ref()
                    .is_some_and(|entry| entry.owner == self.id)
                {
                    slot.generation = handle_space::next_generation(slot.generation);
                    released.extend(slot.entry.take());
                }
            }
            released
        };
        self.live = 0;
        drop(released);
    }
}
