//! Where kernel stacks and the guard gaps below them sit inside a window.
//!
//! A stack overrun should fault, not corrupt, so every stack has a gap below
//! it that is simply never mapped — absence of mapping is the strongest guard
//! there is, and it costs nothing. This crate is the arithmetic that places
//! them: which addresses are a slot's guard, which are its stack, and the
//! proof (as host tests) that no two slots touch. The mapping itself is
//! `wost`'s; the policy of how many and where is the kernel's.
//!
//! One caveat travels with the design rather than the arithmetic: a stack
//! frame larger than the guard can step clean over it without faulting. A
//! guard of one page is adequate for a kernel built without `alloca` and
//! without SSE spills, and D-52 says so explicitly rather than silently.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Bytes in a page, the granularity mappings and guards come in.
pub const PAGE_SIZE: u64 = 4096;

/// Bytes in the word a stack paint and its scan step by.
const WORD: u64 = 8;

/// A window carved into equal slots of guard-then-stack.
///
/// Slots ascend from `base`, each `guard_len + stack_len` bytes: the guard
/// occupies the low addresses, the stack the high ones, so a stack growing
/// down runs into its own guard and never into a neighbour.
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

/// One slot of a [`StackLayout`]: a guard range and the stack above it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slot {
    /// First address of the guard gap. Never mapped.
    guard_base: u64,
    /// First address of the stack. Mapped writable, never executable.
    stack_base: u64,
    /// Bytes of stack.
    stack_len: u64,
}

/// Stack capacity split at one observed descending stack pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StackUsage {
    capacity: u64,
    used: u64,
}

impl StackUsage {
    /// Bytes available in the complete stack slot.
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

impl Slot {
    /// First address of the guard gap. Never mapped.
    pub const fn guard_base(&self) -> u64 {
        self.guard_base
    }

    /// First address of the stack. Mapped writable, never executable.
    pub const fn stack_base(&self) -> u64 {
        self.stack_base
    }

    /// Bytes of stack.
    pub const fn stack_len(&self) -> u64 {
        self.stack_len
    }

    /// One past the stack's last byte — where a descending stack begins.
    pub const fn stack_top(&self) -> u64 {
        self.stack_base + self.stack_len
    }

    /// Split this stack's capacity at an observed descending stack pointer.
    ///
    /// The top itself represents an unused stack and the base represents a
    /// completely used one. A pointer outside the checked slot has no usage
    /// meaning and is refused.
    pub const fn usage_at(&self, pointer: u64) -> Option<StackUsage> {
        if pointer < self.stack_base || pointer > self.stack_top() {
            return None;
        }
        Some(StackUsage {
            capacity: self.stack_len,
            used: self.stack_top() - pointer,
        })
    }

    /// The word-aligned span from the stack's base to `clearance` below `pointer`.
    ///
    /// A frame at `pointer` whose own calls stay within `clearance` holds
    /// nothing there. `None` when `pointer` is outside the stack or no whole
    /// word fits below the clearance.
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

/// Whether two half-open byte ranges overlap.
///
/// Empty ranges touch nothing. An overflowing non-empty range is invalid and
/// treated as overlapping so a caller protecting memory fails closed.
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

impl StackLayout {
    /// Lay out `slots` stacks of `stack_len` bytes, each above a
    /// `guard_len`-byte gap, ascending from `base`.
    ///
    /// `None` unless every value is page-aligned and non-zero and the whole
    /// window fits below `u64::MAX` — a layout that wraps would alias its own
    /// slots, which is the one property this constructor exists to rule out.
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

    /// How many slots the layout holds.
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

#[cfg(test)]
mod tests;
