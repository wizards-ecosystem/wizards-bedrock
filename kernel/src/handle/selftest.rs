use handle_space::Slot;

use super::HandleTable;

impl HandleTable {
    pub(crate) fn slot_of(&self, handle: u32) -> Option<Slot> {
        Slot::decode(handle, self.mixer)
    }

    // Another table's handle decodes here to an index almost never in the arena, so a forged one
    // must be encoded with this table's mixer to reach the ownership comparison.
    pub(crate) fn handle_for(&self, slot: Slot) -> u32 {
        slot.encode(self.mixer)
    }

    pub(crate) fn identity_for_test(&self) -> u64 {
        self.id.0
    }
}
