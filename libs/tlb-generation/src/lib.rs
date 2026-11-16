#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub const MAX_CPUS: u32 = 64;

pub const SLOTS: usize = 6;

const STRIPPED_PCID: u16 = 0x800;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cpu(u8);

impl Cpu {
    pub const fn new(index: u32) -> Option<Self> {
        if index < MAX_CPUS {
            Some(Self(index as u8))
        } else {
            None
        }
    }

    pub const fn index(self) -> u8 {
        self.0
    }

    const fn bit(self) -> u64 {
        1 << self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CpuSet(u64);

impl CpuSet {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn contains(self, cpu: Cpu) -> bool {
        self.0 & cpu.bit() != 0
    }

    #[must_use]
    pub const fn with(self, cpu: Cpu) -> Self {
        Self(self.0 | cpu.bit())
    }

    #[must_use]
    pub const fn without(self, cpu: Cpu) -> Self {
        Self(self.0 & !cpu.bit())
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

// A generation never repeats, so a space that would pass the last one is refused.
#[must_use]
pub const fn advance(generation: u64) -> Option<u64> {
    generation.checked_add(1)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Extent {
    Range,
    Whole,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Skip,
    Range,
    Whole,
}

// Returns the flush to perform and the generation this CPU has then seen.
#[must_use]
pub const fn catch_up(seen: u64, request: u64, latest: u64, extent: Extent) -> (Action, u64) {
    let newest = if request > latest { request } else { latest };
    if seen >= newest || request <= seen {
        return (Action::Skip, seen);
    }
    if matches!(extent, Extent::Range) && request == seen + 1 && request == newest {
        return (Action::Range, request);
    }
    (Action::Whole, newest)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Slot {
    space: u64,
    seen: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Load {
    slot: usize,
    flushes: bool,
    evicted: Option<u64>,
}

impl Load {
    pub const fn slot(self) -> usize {
        self.slot
    }

    pub const fn full_pcid(self) -> u16 {
        self.slot as u16 + 1
    }

    pub const fn stripped_pcid(self) -> u16 {
        self.full_pcid() | STRIPPED_PCID
    }

    pub const fn flushes(self) -> bool {
        self.flushes
    }

    pub const fn evicted(self) -> Option<u64> {
        self.evicted
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slots {
    entries: [Option<Slot>; SLOTS],
    victim: usize,
}

impl Slots {
    pub const fn new() -> Self {
        Self {
            entries: [None; SLOTS],
            victim: 0,
        }
    }

    pub fn load(&mut self, space: u64, latest: u64) -> Load {
        let mut index = 0;
        while index < SLOTS {
            if let Some(slot) = self.entries[index]
                && slot.space == space
            {
                let flushes = slot.seen < latest;
                if flushes {
                    self.entries[index] = Some(Slot {
                        space,
                        seen: latest,
                    });
                }
                return Load {
                    slot: index,
                    flushes,
                    evicted: None,
                };
            }
            index += 1;
        }
        let fresh = Some(Slot {
            space,
            seen: latest,
        });
        index = 0;
        while index < SLOTS {
            if self.entries[index].is_none() {
                self.entries[index] = fresh;
                return Load {
                    slot: index,
                    flushes: true,
                    evicted: None,
                };
            }
            index += 1;
        }
        let index = self.victim;
        let mut evicted = None;
        if let Some(slot) = self.entries[index] {
            evicted = Some(slot.space);
        }
        self.entries[index] = fresh;
        self.victim = if index + 1 == SLOTS { 0 } else { index + 1 };
        Load {
            slot: index,
            flushes: true,
            evicted,
        }
    }

    // Only for the space loaded on this CPU; a space not loaded flushes when it is next loaded.
    pub fn catch_up(&mut self, space: u64, request: u64, latest: u64, extent: Extent) -> Action {
        let mut index = 0;
        while index < SLOTS {
            if let Some(slot) = self.entries[index]
                && slot.space == space
            {
                let (action, seen) = catch_up(slot.seen, request, latest, extent);
                self.entries[index] = Some(Slot { space, seen });
                return action;
            }
            index += 1;
        }
        Action::Skip
    }
}

impl Default for Slots {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shootdown {
    generation: u64,
    remaining: CpuSet,
}

impl Shootdown {
    pub const fn new(targets: CpuSet, requester: Cpu, generation: u64) -> Self {
        Self {
            generation,
            remaining: targets.without(requester),
        }
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }

    pub const fn remaining(self) -> CpuSet {
        self.remaining
    }

    #[must_use]
    pub const fn acknowledge(self, cpu: Cpu) -> Option<Self> {
        if !self.remaining.contains(cpu) {
            return None;
        }
        Some(Self {
            generation: self.generation,
            remaining: self.remaining.without(cpu),
        })
    }

    pub const fn is_complete(self) -> bool {
        self.remaining.is_empty()
    }
}

#[cfg(test)]
mod tests;
