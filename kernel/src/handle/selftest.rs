use handle_space::Slot;

use super::{ARENA, HandleTable};

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
