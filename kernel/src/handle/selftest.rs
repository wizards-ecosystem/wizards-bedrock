use core::ops::Deref;

use handle_space::Slot;
use native_abi::Rights;
use wost::mm::pool::earmark;
use wost::mm::slab::Hold;

use super::{HandleError, Table, TableRef};
use crate::object::ObjectRef;
use crate::thread::ThreadContext;
use crate::world::{self, TablePool, World, registry};

impl Table {
    pub(crate) fn slot_of(&self, handle: u64) -> Option<Slot> {
        Slot::decode(handle, self.mixer)
    }

    pub(crate) fn handle_for(&self, slot: Slot) -> u64 {
        slot.encode(self.mixer)
    }
}

// A table of a chosen capacity in the root world, destroyed when the check ends.
#[derive(Debug)]
pub(crate) struct TestTable {
    pub(crate) table: TableRef,
    held: Option<(Hold<World>, Hold<Table>)>,
}

impl TestTable {
    pub(crate) fn new(context: &mut ThreadContext, capacity: u32) -> Option<Self> {
        let world_ref = world::root()?;
        let world = registry::hold(world_ref)?;
        let frames = TablePool::frames_for(1)?.checked_add(Table::frames_for(capacity))?;
        let mut earmark = earmark(context, world.arena(), frames).ok()?;
        let table = Table::new(&mut earmark, world.mixer(), capacity);
        let table = TableRef {
            world: world_ref,
            table: world.tables.insert(&mut earmark, || table),
        };
        Some(Self {
            table,
            held: table.hold(),
        })
    }

    fn world(&self) -> &World {
        match &self.held {
            Some((world, _)) => world,
            None => panic!("a self-test table vanished while the check held it"),
        }
    }

    // A handle to the root world, an object any number of handles may name and none counts.
    pub(crate) fn insert_world(
        &self,
        context: &mut ThreadContext,
        rights: Rights,
    ) -> Result<u64, HandleError> {
        let root = ObjectRef::World(self.world().node());
        self.insert(context, root, rights)
    }
}

impl Deref for TestTable {
    type Target = Table;

    fn deref(&self) -> &Table {
        match &self.held {
            Some((_, table)) => table,
            None => panic!("a self-test table vanished while the check held it"),
        }
    }
}

impl Drop for TestTable {
    fn drop(&mut self) {
        drop(self.held.take());
        assert!(self.table.destroy(), "a self-test table outlived its check");
    }
}
