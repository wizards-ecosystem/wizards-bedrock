use wost::sync::{Level, impl_lock_after};

// Thread locks rank below every spin lock, because none is taken while a spin lock is held.
pub(crate) enum Storage {}
pub(crate) enum Iommu {}
pub(crate) enum Console {}
pub(crate) enum Serial {}
// One lock per kernel object, and at most one of them held at a time.
pub(crate) enum Object {}
pub(crate) enum Space {}
// The scheduler's: the reaper's list, then wait lists, then run queues, at most one of each.
pub(crate) enum Reap {}
pub(crate) enum Wait {}
pub(crate) enum RunQueue {}

impl Level for Storage {
    const RANK: u8 = 1;
}

impl Level for Iommu {
    const RANK: u8 = 2;
}

impl Level for Console {
    const RANK: u8 = 3;
}

impl Level for Serial {
    const RANK: u8 = 4;
}

impl Level for Object {
    const RANK: u8 = 6;
}

impl Level for Space {
    const RANK: u8 = 8;
}

impl Level for Reap {
    const RANK: u8 = 9;
}

impl Level for Wait {
    const RANK: u8 = 10;
}

impl Level for RunQueue {
    const RANK: u8 = 11;
}

impl_lock_after!(Unlocked => Storage);
impl_lock_after!(Storage => Iommu);
impl_lock_after!(Iommu => Console);
impl_lock_after!(Console => Serial);
impl_lock_after!(Serial => Object);
impl_lock_after!(Object => Space);
impl_lock_after!(Space => Reap);
impl_lock_after!(Reap => Wait);
impl_lock_after!(Wait => RunQueue);
