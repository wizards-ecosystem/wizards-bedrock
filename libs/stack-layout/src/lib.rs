//! Where kernel stacks and the never-mapped guard gaps below them sit inside a window.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Bytes in a page, the granularity mappings and guards come in.
pub const PAGE_SIZE: u64 = 4096;

/// Bytes in the word a stack paint and its scan step by.
const WORD: u64 = 8;

/// A window of guard-then-stack slots; a one-page guard assumes no `alloca` or SSE spills.
///
/// # Examples
///
/// ```
/// use stack_layout::StackLayout;
///
/// let layout = StackLayout::new(0x1000_0000, 16 * 1024, 4096, 4).unwrap();
/// let slot = layout.slot(0).unwrap();
/// assert_eq!(slot.guard_base(), 0x1000_0000);
/// assert_eq!(slot.stack_base(), 0x1000_1000);
/// assert_eq!(slot.stack_top(), 0x1000_5000);
/// ```
///
/// A slot can only come from a checked layout; its bounds cannot be forged.
///
/// ```compile_fail
/// let layout = stack_layout::StackLayout::new(0, 4096, 4096, 1).unwrap();
/// let slot = layout.slot(0).unwrap();
/// let _ = slot.stack_base;
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StackLayout {
    base: u64,
    stack_len: u64,
    guard_len: u64,
    slots: usize,
}

impl StackLayout {
    /// Lay out page-aligned, non-zero slots, refusing a window that would wrap and alias.
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

    /// The ranges slot `index` occupies, or `None` past the last slot.
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

    /// Bytes from one slot's guard to the next slot's.
    pub const fn stride(&self) -> u64 {
        self.guard_len + self.stack_len
    }

    /// First address of the window.
    pub const fn base(&self) -> u64 {
        self.base
    }

    /// One past the window's last byte.
    pub const fn end(&self) -> u64 {
        self.base + self.stride() * self.slots as u64
    }
}

/// One slot of a [`StackLayout`]: a guard range and the stack above it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slot {
    guard_base: u64,
    stack_base: u64,
    stack_len: u64,
}

impl Slot {
    /// First address of the guard gap. Never mapped.
    pub const fn guard_base(&self) -> u64 {
        self.guard_base
    }

    /// First address of the stack. Mapped writable, never executable.
    pub const fn stack_base(&self) -> u64 {
        self.stack_base
    }

    pub const fn stack_len(&self) -> u64 {
        self.stack_len
    }

    /// One past the stack's last byte, where a descending stack begins.
    pub const fn stack_top(&self) -> u64 {
        self.stack_base + self.stack_len
    }

    /// Split capacity at a descending stack pointer from the base to the top inclusive.
    pub const fn usage_at(&self, pointer: u64) -> Option<StackUsage> {
        if pointer < self.stack_base || pointer > self.stack_top() {
            return None;
        }
        Some(StackUsage {
            capacity: self.stack_len,
            used: self.stack_top() - pointer,
        })
    }

    /// The word-aligned span from the base to `clearance` below `pointer`, unused by a frame there.
    ///
    /// # Examples
    ///
    /// ```
    /// use stack_layout::StackLayout;
    ///
    /// let slot = StackLayout::new(0x1000_0000, 16 * 1024, 4096, 1).unwrap().slot(0).unwrap();
    /// assert_eq!(slot.span_below(0x1000_4ff4, 1024), Some(0x1000_1000..0x1000_4bf0));
    /// assert_eq!(slot.span_below(0x1000_1404, 1024), None);
    /// assert_eq!(slot.span_below(0x1000_0ff8, 0), None);
    /// ```
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

/// Stack capacity split at one observed descending stack pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StackUsage {
    capacity: u64,
    used: u64,
}

impl StackUsage {
    pub const fn capacity(self) -> u64 {
        self.capacity
    }

    /// Bytes between the observed pointer and the top of the stack.
    pub const fn used(self) -> u64 {
        self.used
    }

    /// Bytes between the bottom of the stack and the observed pointer.
    pub const fn headroom(self) -> u64 {
        self.capacity - self.used
    }
}

/// Whether two half-open ranges overlap; empty touches nothing and an overflow fails closed.
pub const fn ranges_overlap(left: u64, left_len: u64, right: u64, right_len: u64) -> bool {
    if left_len == 0 || right_len == 0 {
        return false;
    }
    let Some(left_end) = left.checked_add(left_len) else {
        return true;
    };
    let Some(right_end) = right.checked_add(right_len) else {
        return true;
    };
    left < right_end && right < left_end
}

#[cfg(test)]
mod tests;
