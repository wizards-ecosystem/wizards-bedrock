use handle_space::Slot;

use super::HandleTable;

impl HandleTable {
    pub(crate) fn slot_of(&self, handle: u32) -> Option<Slot> {
        Slot::decode(handle, self.mixer)
    }

    pub(crate) fn handle_for(&self, slot: Slot) -> u32 {
        slot.encode(self.mixer)
    }

    pub(crate) fn identity_for_test(&self) -> u64 {
        self.id.0
    }
}
