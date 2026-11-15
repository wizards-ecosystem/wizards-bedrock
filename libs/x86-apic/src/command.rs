use crate::local_vector::DeliverableVector;

const DELIVERY_FIXED: u32 = 0;

const ASSERT: u32 = 1 << 14;

const SHORTHAND_SELF: u32 = 1 << 18;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Command(u32);

impl Command {
    pub const fn self_interrupt(vector: DeliverableVector) -> Self {
        Self(vector.get().get() as u32 | DELIVERY_FIXED | ASSERT | SHORTHAND_SELF)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }
}
