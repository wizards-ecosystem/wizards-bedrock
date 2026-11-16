use crate::{Refusal, SLOTS_PER_PAGE};

const END: u16 = u16::MAX;
const LIVE: u16 = u16::MAX - 1;
const RETIRED: u16 = u16::MAX - 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Issued {
    pub slot: u16,
    pub generation: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Released {
    Held,
    NowZero,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Removed {
    Freed,
    Retired,
}

// FIFO free list: reuse spreads over every free slot, so each slot retires as late as possible.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Slots<const N: usize> {
    generation: [u32; N],
    strong: [u32; N],
    next_free: [u16; N],
    head: u16,
    tail: u16,
    free: u16,
    live: u16,
}

impl<const N: usize> Slots<N> {
    pub fn new(floor: u32) -> Option<Self> {
        if N == 0 || N > usize::from(SLOTS_PER_PAGE) {
            return None;
        }
        let mut next_free = [END; N];
        let mut slot: u16 = 0;
        while usize::from(slot) < N {
            if usize::from(slot) + 1 < N {
                next_free[usize::from(slot)] = slot + 1;
            }
            slot += 1;
        }
        Some(Self {
            generation: [floor; N],
            strong: [0; N],
            next_free,
            head: 0,
            tail: slot - 1,
            free: slot,
            live: 0,
        })
    }

    pub fn free(&self) -> u16 {
        self.free
    }

    pub fn live(&self) -> u16 {
        self.live
    }

    pub fn insert(&mut self) -> Result<Issued, Refusal> {
        let slot = self.head;
        let at = usize::from(slot);
        if at >= N {
            return Err(Refusal::Full);
        }
        self.head = self.next_free[at];
        if self.head == END {
            self.tail = END;
        }
        self.next_free[at] = LIVE;
        self.strong[at] = 1;
        self.free -= 1;
        self.live += 1;
        Ok(Issued {
            slot,
            generation: self.generation[at],
        })
    }

    pub fn resolves(&self, slot: u16, generation: u32) -> bool {
        self.live_at(slot, generation).is_some()
    }

    pub fn acquire(&mut self, slot: u16, generation: u32) -> Result<(), Refusal> {
        let Some(at) = self.live_at(slot, generation) else {
            return Err(Refusal::Stale);
        };
        if self.strong[at] == 0 {
            return Err(Refusal::Condemned);
        }
        if self.strong[at] == u32::MAX {
            return Err(Refusal::Saturated);
        }
        self.strong[at] += 1;
        Ok(())
    }

    pub fn release(&mut self, slot: u16, generation: u32) -> Result<Released, Refusal> {
        let Some(at) = self.live_at(slot, generation) else {
            return Err(Refusal::Stale);
        };
        if self.strong[at] == 0 {
            return Err(Refusal::Condemned);
        }
        self.strong[at] -= 1;
        if self.strong[at] == 0 {
            Ok(Released::NowZero)
        } else {
            Ok(Released::Held)
        }
    }

    pub fn remove(&mut self, slot: u16, generation: u32) -> Result<Removed, Refusal> {
        let Some(at) = self.live_at(slot, generation) else {
            return Err(Refusal::Stale);
        };
        if self.strong[at] != 0 {
            return Err(Refusal::Held);
        }
        self.live -= 1;
        if self.generation[at] == u32::MAX {
            self.next_free[at] = RETIRED;
            return Ok(Removed::Retired);
        }
        self.generation[at] += 1;
        self.next_free[at] = END;
        let tail = usize::from(self.tail);
        if tail < N {
            self.next_free[tail] = slot;
        } else {
            self.head = slot;
        }
        self.tail = slot;
        self.free += 1;
        Ok(Removed::Freed)
    }

    // A page that later takes this page's index starts at this floor; `None` once a slot retired.
    pub fn floor_after(&self, floor: u32) -> Option<u32> {
        let mut bound = floor;
        let mut at = 0;
        while at < N {
            let above = match self.next_free[at] {
                RETIRED => return None,
                LIVE => self.generation[at].checked_add(1)?,
                _ => self.generation[at],
            };
            if above > bound {
                bound = above;
            }
            at += 1;
        }
        Some(bound)
    }

    fn live_at(&self, slot: u16, generation: u32) -> Option<usize> {
        let at = usize::from(slot);
        if at < N && self.next_free[at] == LIVE && self.generation[at] == generation {
            Some(at)
        } else {
            None
        }
    }
}
