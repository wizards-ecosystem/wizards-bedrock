use core::sync::atomic::{AtomicU64, Ordering};

use handle_space::{Entry as Generation, Mixer, Quota, SLOTS, Slot};
use native_abi::Rights;
use world_tree::NodeRef;
use wost::mm::pool::Earmark;
use wost::mm::slab::{Blank, Hold, Refusal, Removal};
use wost::sync::SpinLock;

use crate::arena::{Local, Pages};
use crate::object::ObjectRef;
use crate::rank;
use crate::thread::ThreadContext;
use crate::world::{World, registry};

#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
pub(crate) use selftest::TestTable;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HandleError {
    Malformed,
    Stale,
    Denied,
    Meaningless,
    QuotaExhausted,
    Gone,
}

impl core::fmt::Display for HandleError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match *self {
            Self::Malformed => "not a handle",
            Self::Stale => "handle names nothing live in this table",
            Self::Denied => "handle does not carry the right",
            Self::Meaningless => "right means nothing for this object type",
            Self::QuotaExhausted => "handle quota exhausted",
            Self::Gone => "the object the handle named is gone",
        })
    }
}

impl core::error::Error for HandleError {}

const PER_PAGE: u32 = 64;

#[derive(Debug)]
struct Entry {
    generation: Generation,
    held: Option<(ObjectRef, Rights)>,
}

#[derive(Debug)]
struct Block {
    entries: [Entry; PER_PAGE as usize],
}

impl Blank for Block {
    const BLANK: Self = Self {
        entries: [const {
            Entry {
                generation: Generation::Free(0),
                held: None,
            }
        }; PER_PAGE as usize],
    };
}

// One table per process in its world's arena; a retired slot stays charged, so the quota never
// promises a slot that no longer exists.
#[derive(Debug)]
pub(crate) struct Table {
    mixer: Mixer,
    capacity: u32,
    counts: AtomicU64,
    entries: SpinLock<rank::Object, Pages<Block>>,
}

impl Table {
    pub(crate) fn frames_for(capacity: u32) -> u32 {
        Pages::<Block>::frames_for(capacity.div_ceil(PER_PAGE))
    }

    pub(crate) fn new(earmark: &mut Earmark, mixer: Mixer, capacity: u32) -> Self {
        assert!(
            capacity <= SLOTS,
            "a table of {capacity} entries exceeds the handle index range"
        );
        Self {
            mixer,
            capacity,
            counts: AtomicU64::new(Quota::EMPTY),
            entries: SpinLock::new(Pages::build(earmark, capacity.div_ceil(PER_PAGE))),
        }
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

    fn limit(&self) -> usize {
        self.capacity as usize
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn live(&self) -> usize {
        Quota::live(self.counts.load(Ordering::Acquire)) as usize
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn promised(&self) -> usize {
        Quota::reserved(self.counts.load(Ordering::Acquire)) as usize
    }

    pub(crate) fn insert(
        &self,
        context: &mut ThreadContext,
        object: ObjectRef,
        rights: Rights,
    ) -> Result<u64, HandleError> {
        if !rights.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        // The charge comes first, so the quota invariant guarantees a free entry.
        if !self.step(|bits| Quota::take(bits, self.limit())) {
            return Err(HandleError::QuotaExhausted);
        }
        Ok(self.fill(context, object, rights))
    }

    fn fill(&self, context: &mut ThreadContext, object: ObjectRef, rights: Rights) -> u64 {
        let mut pages = self.entries.lock(context);
        let mut index = 0;
        while index < self.capacity {
            let Some(entry) = entry_mut(&mut pages, index) else {
                break;
            };
            if let Some((live, generation)) = entry.generation.fill() {
                entry.generation = live;
                entry.held = Some((object, rights));
                let Some(named) = Slot::new(index, generation) else {
                    panic!("a table issued a slot outside the handle range");
                };
                return named.encode(self.mixer);
            }
            index += 1;
        }
        panic!("a charged handle found no free entry");
    }

    pub(crate) fn resolve(
        &self,
        context: &mut ThreadContext,
        handle: u64,
    ) -> Result<(ObjectRef, Rights), HandleError> {
        let named = Slot::decode(handle, self.mixer).ok_or(HandleError::Malformed)?;
        let mut pages = self.entries.lock(context);
        let entry = entry_mut(&mut pages, named.index()).ok_or(HandleError::Stale)?;
        if !entry.generation.is_live(named.generation()) {
            return Err(HandleError::Stale);
        }
        match entry.held.ok_or(HandleError::Stale)? {
            (ObjectRef::Dead(_), _) => Err(HandleError::Gone),
            held => Ok(held),
        }
    }

    // `false` when the slot is at its last generation, so a seal refuses before it changes anything.
    pub(crate) fn renewable(&self, handle: u64) -> bool {
        Slot::decode(handle, self.mixer).is_some_and(|named| {
            Slot::new(named.index(), named.generation().saturating_add(1)).is_some()
        })
    }

    // The same slot and object under the next generation with new rights; the old value is gone.
    pub(crate) fn replace(
        &self,
        context: &mut ThreadContext,
        handle: u64,
        object: ObjectRef,
        rights: Rights,
    ) -> Result<u64, HandleError> {
        if !rights.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        let named = Slot::decode(handle, self.mixer).ok_or(HandleError::Malformed)?;
        let mut pages = self.entries.lock(context);
        let entry = entry_mut(&mut pages, named.index()).ok_or(HandleError::Stale)?;
        if entry.held.is_none_or(|(held, _)| held != object) {
            return Err(HandleError::Stale);
        }
        let (renewed, generation) = entry
            .generation
            .renew(named.generation())
            .ok_or(HandleError::Stale)?;
        entry.generation = renewed;
        entry.held = Some((object, rights));
        let Some(issued) = Slot::new(named.index(), generation) else {
            panic!("a table renewed a slot outside the handle range");
        };
        Ok(issued.encode(self.mixer))
    }

    // The entry goes first and the object's reference after, outside the table lock.
    pub(crate) fn close(
        &self,
        context: &mut ThreadContext,
        world: &World,
        handle: u64,
    ) -> Result<(), HandleError> {
        let named = Slot::decode(handle, self.mixer).ok_or(HandleError::Malformed)?;
        let (object, retired) = {
            let mut pages = self.entries.lock(context);
            let entry = entry_mut(&mut pages, named.index()).ok_or(HandleError::Stale)?;
            let emptied = entry
                .generation
                .empty(named.generation())
                .ok_or(HandleError::Stale)?;
            let Some((object, _)) = entry.held.take() else {
                panic!("a live handle entry held no object");
            };
            entry.generation = emptied;
            (object, emptied == Generation::Retired)
        };
        if !retired {
            let dropped = self.step(Quota::drop_live);
            assert!(dropped, "a closed handle was not counted live");
        }
        object.release(world);
        Ok(())
    }

    // Every handle of a message leaves the table together or none does; each keeps its value
    // while it moves, so a restore gives back exactly what was taken.
    pub(crate) fn take(
        &self,
        context: &mut ThreadContext,
        moved: &[(u64, Rights)],
        into: &mut [Option<(ObjectRef, Rights)>],
    ) -> Result<(), HandleError> {
        let mut pages = self.entries.lock(context);
        let mut taken = 0;
        let refusal = loop {
            let Some(&(handle, declared)) = moved.get(taken) else {
                return Ok(());
            };
            match self.take_one(&mut pages, handle, declared) {
                Ok(object) => into[taken] = Some(object),
                Err(refusal) => break refusal,
            }
            taken += 1;
        };
        for &(handle, _) in &moved[..taken] {
            self.settle_move(&mut pages, handle, Generation::restore);
        }
        into.fill(None);
        Err(refusal)
    }

    fn take_one(
        &self,
        pages: &mut Pages<Block>,
        handle: u64,
        declared: Rights,
    ) -> Result<(ObjectRef, Rights), HandleError> {
        let named = Slot::decode(handle, self.mixer).ok_or(HandleError::Malformed)?;
        let entry = entry_mut(pages, named.index()).ok_or(HandleError::Stale)?;
        let moving = entry
            .generation
            .take(named.generation())
            .ok_or(HandleError::Stale)?;
        let (object, held) = entry.held.ok_or(HandleError::Stale)?;
        if let ObjectRef::Dead(_) = object {
            return Err(HandleError::Gone);
        }
        if !held.contains(Rights::TRANSFER) {
            return Err(HandleError::Denied);
        }
        let granted = held.attenuate(declared).ok_or(HandleError::Denied)?;
        entry.generation = moving;
        Ok((object, granted))
    }

    pub(crate) fn restore(&self, context: &mut ThreadContext, moved: &[(u64, Rights)]) {
        let mut pages = self.entries.lock(context);
        for &(handle, _) in moved {
            self.settle_move(&mut pages, handle, Generation::restore);
        }
    }

    // The objects now belong to the message, so no reference is released here.
    pub(crate) fn erase(&self, context: &mut ThreadContext, moved: &[(u64, Rights)]) {
        let mut retired = 0;
        {
            let mut pages = self.entries.lock(context);
            for &(handle, _) in moved {
                if self.settle_move(&mut pages, handle, Generation::erase) == Generation::Retired {
                    retired += 1;
                }
            }
        }
        for _ in retired..moved.len() {
            let dropped = self.step(Quota::drop_live);
            assert!(dropped, "an erased handle was not counted live");
        }
    }

    fn settle_move(
        &self,
        pages: &mut Pages<Block>,
        handle: u64,
        settle: fn(Generation, u64) -> Option<Generation>,
    ) -> Generation {
        let settled = Slot::decode(handle, self.mixer).and_then(|named| {
            let entry = entry_mut(pages, named.index())?;
            let next = settle(entry.generation, named.generation())?;
            entry.generation = next;
            if !matches!(next, Generation::Live(_)) {
                entry.held = None;
            }
            Some(next)
        });
        settled.unwrap_or_else(|| panic!("a moving handle left its state"))
    }

    // Teardown of a table no one else can reach: every live entry's reference goes back.
    fn drain(&mut self, world: &World) {
        let pages = self.entries.get_mut();
        let mut index = 0;
        while index < self.capacity {
            let Some(entry) = entry_mut(pages, index) else {
                break;
            };
            if let Some((object, _)) = entry.held.take() {
                object.release(world);
            }
            index += 1;
        }
    }
}

fn entry_mut(pages: &mut Pages<Block>, index: u32) -> Option<&mut Entry> {
    pages
        .get_mut(index / PER_PAGE)?
        .entries
        .get_mut((index % PER_PAGE) as usize)
}

// Names a table by its world and its place in that world's arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TableRef {
    world: NodeRef,
    table: Local<Table>,
}

impl TableRef {
    pub(crate) fn create(earmark: &mut Earmark, world_ref: NodeRef, world: &World) -> Self {
        let table = Table::new(earmark, world.mixer(), world.handles());
        Self {
            world: world_ref,
            table: world.tables.insert(earmark, || table),
        }
    }

    pub(crate) fn hold(self) -> Option<(Hold<World>, Hold<Table>)> {
        let world = registry::hold(self.world)?;
        let table = world.tables.hold(self.table)?;
        Some((world, table))
    }

    pub(crate) fn reserve_slot(self) -> Result<SlotReservation, HandleError> {
        let (_, table) = self.hold().ok_or(HandleError::Gone)?;
        if !table.step(|bits| Quota::reserve(bits, table.limit())) {
            return Err(HandleError::QuotaExhausted);
        }
        Ok(SlotReservation {
            table: self,
            spent: false,
        })
    }

    // The process's reference goes; the table and every reference it holds go with the last one.
    pub(crate) fn destroy(self) -> bool {
        let Some(world) = registry::hold(self.world) else {
            return true;
        };
        match world.tables.destroy(self.table) {
            Ok(Some(mut table)) => {
                table.drain(&world);
                true
            }
            Ok(None) | Err(Removal::Refused(Refusal::Stale)) => true,
            Err(_) => false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct SlotReservation {
    table: TableRef,
    spent: bool,
}

impl SlotReservation {
    pub(crate) fn install(
        mut self,
        context: &mut ThreadContext,
        object: ObjectRef,
        offered: Rights,
        declared: Rights,
    ) -> Result<u64, HandleError> {
        let granted = offered.attenuate(declared).ok_or(HandleError::Denied)?;
        if !granted.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        let (_, table) = self.table.hold().ok_or(HandleError::Gone)?;
        let committed = table.step(Quota::commit);
        assert!(committed, "a live reservation refused quota commit");
        self.spent = true;
        Ok(table.fill(context, object, granted))
    }
}

impl Drop for SlotReservation {
    fn drop(&mut self) {
        if self.spent {
            return;
        }
        if let Some((_, table)) = self.table.hold() {
            let released = table.step(Quota::release);
            assert!(released, "a live reservation refused quota release");
        }
    }
}
