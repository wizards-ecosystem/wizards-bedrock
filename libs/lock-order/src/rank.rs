pub trait Level {
    const RANK: u8;
}

pub trait LockAfter<A> {}

pub trait LockBefore<B> {}

impl<B: LockAfter<A>, A> LockBefore<B> for A {}

#[derive(Debug)]
pub enum Unlocked {}

impl Level for Unlocked {
    const RANK: u8 = 0;
}
