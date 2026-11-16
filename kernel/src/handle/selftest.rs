use core::ops::Deref;

use handle_space::Slot;
use native_abi::Rights;
use wost::mm::pool::earmark;
use wost::mm::slab::Hold;

use super::{HandleError, Table, TableRef};
use crate::object;
use crate::thread::ThreadContext;
use crate::world::{self, ChannelPool, TablePool, World, registry};

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
            table: world.tables.insert(&mut earmark, table),
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

    // A fresh channel in the root world, so each handle names an object of its own.
    pub(crate) fn insert_channel(
        &self,
        context: &mut ThreadContext,
        rights: Rights,
    ) -> Result<u64, HandleError> {
        let world = self.world();
        let frames = ChannelPool::frames_for(1).ok_or(HandleError::QuotaExhausted)?;
        let mut earmark =
            earmark(context, world.arena(), frames).map_err(|_| HandleError::QuotaExhausted)?;
        let channel = object::bootstrap_identity(&mut earmark, world);
        self.insert(context, channel, rights).inspect_err(|_| {
            channel.release(world);
        })
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
