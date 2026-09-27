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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackLayout {
    base: u64,
    stack_len: u64,
    guard_len: u64,
    slots: usize,
}

/// One slot of a [`StackLayout`]: a guard range and the stack above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    /// First address of the guard gap. Never mapped.
    guard_base: u64,
    /// First address of the stack. Mapped writable, never executable.
    stack_base: u64,
    /// Bytes of stack.
    stack_len: u64,
}

/// Stack capacity split at one observed descending stack pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
mod tests {
    use super::*;

    fn layout() -> StackLayout {
        StackLayout::new(0xffff_fe00_0000_0000, 16 * 1024, 4096, 16).expect("a valid layout")
    }

    /// The addresses the layout actually produces, transcribed.
    ///
    /// Absolute addresses are independent of `stride()`, unlike a relational
    /// `base + stride·i <= base + stride·i` tautology that also admits
    /// overlapping slots (D-105).
    #[test]
    fn slots_land_where_the_arithmetic_says() {
        let layout = layout();
        // Stride is 4096 guard + 16384 stack = 20480 = 0x5000.
        assert_eq!(layout.stride(), 0x5000);
        assert_eq!(layout.base(), 0xffff_fe00_0000_0000);

        let first = layout.slot(0).expect("in range");
        assert_eq!(first.guard_base(), 0xffff_fe00_0000_0000);
        assert_eq!(first.stack_base(), 0xffff_fe00_0000_1000);
        assert_eq!(first.stack_top(), 0xffff_fe00_0000_5000);

        let second = layout.slot(1).expect("in range");
        assert_eq!(second.guard_base(), 0xffff_fe00_0000_5000);
        assert_eq!(second.stack_base(), 0xffff_fe00_0000_6000);
        assert_eq!(second.stack_top(), 0xffff_fe00_0000_a000);

        let last = layout.slot(15).expect("in range");
        assert_eq!(last.guard_base(), 0xffff_fe00_0004_b000);
        assert_eq!(last.stack_top(), 0xffff_fe00_0005_0000);
    }

    /// No stack overlaps a neighbour's, and every gap between them is a guard.
    ///
    /// Stated against the *stacks* rather than against `stride`, so a stride
    /// that swallowed the guard would be visible here: a guard page is a page
    /// nothing maps, and it is only a guard if it is not another slot's stack.
    #[test]
    fn no_two_stacks_touch_and_a_guard_lies_between_them() {
        let layout = layout();
        for index in 1..layout.slots() {
            let below = layout.slot(index - 1).expect("in range");
            let slot = layout.slot(index).expect("in range");
            assert_eq!(
                slot.stack_base() - below.stack_top(),
                4096,
                "slot {index} is not one guard page above its neighbour"
            );
            assert!(
                below.stack_top() <= slot.guard_base(),
                "slot {index} begins before its neighbour ends"
            );
        }
    }

    #[test]
    fn every_stack_has_its_guard_immediately_below() {
        let layout = layout();
        for index in 0..layout.slots() {
            let slot = layout.slot(index).expect("in range");
            assert_eq!(slot.guard_base() + 4096, slot.stack_base());
            assert_eq!(slot.stack_top() - slot.stack_base(), 16 * 1024);
            assert_eq!(slot.stack_len(), 16 * 1024);
        }
    }

    #[test]
    fn a_live_pointer_splits_stack_capacity_exactly() {
        let slot = layout().slot(0).expect("in range");
        let at_top = slot.usage_at(slot.stack_top()).expect("top is valid");
        assert_eq!(at_top.capacity(), 16 * 1024);
        assert_eq!(at_top.used(), 0);
        assert_eq!(at_top.headroom(), 16 * 1024);

        let interior = slot
            .usage_at(slot.stack_top() - 0x1234)
            .expect("interior is valid");
        assert_eq!(interior.used(), 0x1234);
        assert_eq!(interior.headroom(), 16 * 1024 - 0x1234);

        let at_base = slot.usage_at(slot.stack_base()).expect("base is valid");
        assert_eq!(at_base.used(), 16 * 1024);
        assert_eq!(at_base.headroom(), 0);
    }

    #[test]
    fn a_pointer_outside_the_stack_has_no_usage() {
        let slot = layout().slot(0).expect("in range");
        assert_eq!(slot.usage_at(slot.stack_base() - 1), None);
        assert_eq!(slot.usage_at(slot.stack_top() + 1), None);
    }

    /// `end()` and `last.stack_top()` are the same expression written twice,
    /// so the equality between them is `X == X`; the window's length against a
    /// transcribed number is what says the layout is the size it claims
    /// (D-105).
    #[test]
    fn the_window_ends_where_the_last_slot_does() {
        let layout = layout();
        let last = layout.slot(layout.slots() - 1).expect("in range");
        assert_eq!(layout.end(), 0xffff_fe00_0005_0000);
        assert_eq!(layout.end() - layout.base(), 16 * 0x5000);
        assert_eq!(layout.end(), last.stack_top());
        assert_eq!(layout.slot(layout.slots()), None);
    }

    #[test]
    fn misalignment_is_refused() {
        assert_eq!(StackLayout::new(0x1000 + 8, 0x4000, 0x1000, 4), None);
        assert_eq!(StackLayout::new(0x1000, 0x4000 + 8, 0x1000, 4), None);
        assert_eq!(StackLayout::new(0x1000, 0x4000, 0x1000 + 8, 4), None);
    }

    #[test]
    fn nothing_may_be_zero() {
        assert_eq!(StackLayout::new(0x1000, 0, 0x1000, 4), None);
        assert_eq!(StackLayout::new(0x1000, 0x4000, 0, 4), None);
        assert_eq!(StackLayout::new(0x1000, 0x4000, 0x1000, 0), None);
    }

    #[test]
    fn a_window_that_would_wrap_is_refused() {
        let top = !(PAGE_SIZE - 1);
        assert_eq!(StackLayout::new(top, 0x4000, 0x1000, 1), None);
        assert_eq!(StackLayout::new(0, !(PAGE_SIZE - 1), 0x1000, 2), None);
    }

    /// A page is four kilobytes, and a guard smaller than one is refused.
    ///
    /// Anchor `PAGE_SIZE` and include a 2048-byte rejection because positive
    /// values aligned to 4096 cannot distinguish a halved modulus. `wost` maps
    /// in 4096-byte units (D-124).
    #[test]
    fn a_guard_smaller_than_a_page_is_refused() {
        assert_eq!(PAGE_SIZE, 4096);
        assert!(
            StackLayout::new(0xffff_fe00_0000_0000, 4096, 2048, 2).is_none(),
            "a half-page guard was accepted"
        );
        assert!(
            StackLayout::new(0xffff_fe00_0000_0000, 2048, 4096, 2).is_none(),
            "a half-page stack was accepted"
        );
    }
}
