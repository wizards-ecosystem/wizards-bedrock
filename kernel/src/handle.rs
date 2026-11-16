use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use handle_space::{Mixer, Quota, SLOTS, Slot};
use native_abi::Rights;
use wost::sync::{Leaf, SpinLock};

use crate::object::ObjectRef;

#[cfg(feature = "selftest")]
mod selftest;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TableId(u64);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HandleError {
    Malformed,
    Stale,
    Denied,
    Meaningless,
    QuotaExhausted,
    ArenaFull,
    OutOfMemory,
    IdentityExhausted,
}

impl core::fmt::Display for HandleError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match *self {
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

#[derive(Debug)]
struct Entry {
    owner: TableId,
    object: ObjectRef,
    rights: Rights,
}

#[derive(Debug)]
struct ArenaSlot {
    generation: u32,
    entry: Option<Entry>,
}

#[derive(Debug)]
struct Arena {
    slots: Vec<ArenaSlot>,
    hint: usize,
    committed: usize,
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

    // Never allocates: claims run in trap context and under the `ARENA` leaf.
    fn claim(&mut self, entry: Entry) -> Result<(usize, u32), Entry> {
        let Some(index) = self.find_free() else {
            return Err(entry);
        };
        let slot = &mut self.slots[index];
        let generation = slot.generation;
        slot.entry = Some(entry);
        Ok((index, generation))
    }

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
            return None;
        }
        self.slots.push(ArenaSlot {
            generation: 0,
            entry: None,
        });
        Some(count)
    }

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

    fn commit_table(&mut self, required: usize, successor: u64) -> TableId {
        let id = TableId(self.next_table);
        self.next_table = successor;
        self.committed = required;
        id
    }
}

// A leaf: objects released in table teardown drop only after its guard is gone.
static ARENA: SpinLock<Leaf, Arena> = SpinLock::new(Arena::new());

// Backing for the whole quota is committed here, so a trap-context claim never allocates.
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
            // Another constructor may have installed enough backing meanwhile.
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

#[derive(Debug)]
pub(crate) struct HandleTable {
    id: TableId,
    mixer: Mixer,
    quota: usize,
    counts: AtomicU64,
}

impl HandleTable {
    pub(crate) fn new(quota: usize) -> Result<Self, HandleError> {
        let id = commit_table(quota)?;
        Ok(Self {
            id,
            mixer: Mixer::for_table(id.0 as u32),
            quota,
            counts: AtomicU64::new(Quota::EMPTY),
        })
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn live(&self) -> usize {
        Quota::live(self.counts.load(Ordering::Acquire)) as usize
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn promised(&self) -> usize {
        Quota::reserved(self.counts.load(Ordering::Acquire)) as usize
    }

    #[must_use = "a refused quota transition changed no count"]
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

    pub(crate) fn reserve_slot(self: &Arc<Self>) -> Result<SlotReservation, HandleError> {
        if !self.step(|bits| Quota::reserve(bits, self.quota)) {
            return Err(HandleError::QuotaExhausted);
        }
        Ok(SlotReservation {
            table: Arc::clone(self),
            spent: false,
        })
    }

    pub(crate) fn insert(&self, object: ObjectRef, rights: Rights) -> Result<u32, HandleError> {
        if !rights.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        // The charge comes first, so the commitment invariant guarantees the arena claim.
        if !self.step(|bits| Quota::take(bits, self.quota)) {
            return Err(HandleError::QuotaExhausted);
        }
        Ok(self.fill(object, rights))
    }

    fn fill(&self, object: ObjectRef, rights: Rights) -> u32 {
        let entry = Entry {
            owner: self.id,
            object,
            rights,
        };
        let claimed = {
            let mut arena = ARENA.lock_leaf();
            arena.claim(entry)
        };
        let (index, generation) = match claimed {
            Ok(found) => found,
            Err(entry) => {
                drop(entry);
                panic!("a charged handle had no committed arena slot");
            }
        };

        let named = Slot::new(index as u32, generation)
            .unwrap_or_else(|| panic!("a charged handle produced an invalid slot name"));
        named.encode(self.mixer)
    }

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
}

#[derive(Debug)]
pub(crate) struct SlotReservation {
    table: Arc<HandleTable>,
    spent: bool,
}

impl SlotReservation {
    pub(crate) fn install(
        self,
        object: ObjectRef,
        offered: Rights,
        declared: Rights,
    ) -> Result<u32, HandleError> {
        let granted = offered.attenuate(declared).ok_or(HandleError::Denied)?;
        if !granted.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        let handle = self.table.fill(object, granted);
        self.keep();
        Ok(handle)
    }

    fn keep(mut self) {
        let committed = self.table.step(Quota::commit);
        assert!(committed, "a live reservation refused quota commit");
        self.spent = true;
    }
}

impl Drop for SlotReservation {
    fn drop(&mut self) {
        if !self.spent {
            let released = self.table.step(Quota::release);
            assert!(released, "a live reservation refused quota release");
        }
    }
}

impl Drop for HandleTable {
    fn drop(&mut self) {
        // One at a time, so teardown needs no temporary allocation.
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

        let commitment_held = {
            let mut arena = ARENA.lock_leaf();
            let held = arena.committed >= self.quota;
            if held {
                arena.committed -= self.quota;
            }
            held
        };
        assert!(
            commitment_held,
            "a table released quota the arena had not committed"
        );
        self.counts.store(Quota::EMPTY, Ordering::Release);
    }
}
