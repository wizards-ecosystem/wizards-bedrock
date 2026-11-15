#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub const PAGE_SIZE: u64 = 4096;

const WORD: u64 = 8;

// A one-page guard assumes no `alloca` or SSE spills.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StackLayout {
    base: u64,
    stack_len: u64,
    guard_len: u64,
    slots: usize,
}

impl StackLayout {
    pub const fn new(base: u64, stack_len: u64, guard_len: u64, slots: usize) -> Option<Self> {
        if !base.is_multiple_of(PAGE_SIZE)
            || !stack_len.is_multiple_of(PAGE_SIZE)
            || !guard_len.is_multiple_of(PAGE_SIZE)
        {
            return None;
        }
        if stack_len == 0 || guard_len == 0 || slots == 0 {
            return None;
        }

        let Some(stride) = stack_len.checked_add(guard_len) else {
            return None;
        };
        let slot_count = slots as u64;
        if slot_count as usize != slots {
            return None;
        }
        let Some(span) = stride.checked_mul(slot_count) else {
            return None;
        };
        if base.checked_add(span).is_none() {
            return None;
        }

        Some(Self {
            base,
            stack_len,
            guard_len,
            slots,
        })
    }

    pub const fn slot(&self, index: usize) -> Option<Slot> {
        if index >= self.slots {
            return None;
        }
        let guard_base = self.base + self.stride() * index as u64;
        Some(Slot {
            guard_base,
            stack_base: guard_base + self.guard_len,
            stack_len: self.stack_len,
        })
    }

    pub const fn slots(&self) -> usize {
        self.slots
    }

    const fn stride(&self) -> u64 {
        self.guard_len + self.stack_len
    }

    pub const fn base(&self) -> u64 {
        self.base
    }

    pub const fn end(&self) -> u64 {
        self.base + self.stride() * self.slots as u64
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slot {
    guard_base: u64,
    stack_base: u64,
    stack_len: u64,
}

impl Slot {
    pub const fn guard_base(&self) -> u64 {
        self.guard_base
    }

    pub const fn stack_base(&self) -> u64 {
        self.stack_base
    }

    pub const fn stack_len(&self) -> u64 {
        self.stack_len
    }

    pub const fn stack_top(&self) -> u64 {
        self.stack_base + self.stack_len
    }

    pub const fn usage_at(&self, pointer: u64) -> Option<StackUsage> {
        if pointer < self.stack_base || pointer > self.stack_top() {
            return None;
        }
        Some(StackUsage {
            capacity: self.stack_len,
            used: self.stack_top() - pointer,
        })
    }

    pub const fn span_below(&self, pointer: u64, clearance: u64) -> Option<core::ops::Range<u64>> {
        if self.usage_at(pointer).is_none() {
            return None;
        }
        let Some(limit) = pointer.checked_sub(clearance) else {
            return None;
        };
        let end = limit - limit % WORD;
        if end <= self.stack_base {
            return None;
        }
        Some(self.stack_base..end)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StackUsage {
    capacity: u64,
    used: u64,
}

impl StackUsage {
    pub const fn capacity(self) -> u64 {
        self.capacity
    }

    pub const fn used(self) -> u64 {
        self.used
    }

    pub const fn headroom(self) -> u64 {
        self.capacity - self.used
    }
}

pub const fn ranges_overlap(left: u64, left_length: u64, right: u64, right_length: u64) -> bool {
    if left_length == 0 || right_length == 0 {
        return false;
    }
    let Some(left_end) = left.checked_add(left_length) else {
        return true;
    };
    let Some(right_end) = right.checked_add(right_length) else {
        return true;
    };
    left < right_end && right < left_end
}
