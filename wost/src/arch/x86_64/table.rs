//! The operand the descriptor table registers are loaded from.
//!
//! `lgdt`, `lidt` and their `s`-prefixed counterparts all read or write the same
//! ten bytes: a limit and a base, in that order and with no padding between
//! them.

use core::fmt;

/// The ten bytes a descriptor table register holds.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C, packed(2))]
pub(super) struct TablePointer {
    limit: u16,
    base: u64,
}

impl TablePointer {
    /// Describe a table of `limit + 1` bytes at `base`.
    pub(super) const fn new(base: u64, limit: u16) -> Self {
        Self { limit, base }
    }

    /// A pointer for the CPU to overwrite.
    pub(super) const fn empty() -> Self {
        Self { limit: 0, base: 0 }
    }

    /// Address of the first entry.
    pub(super) const fn base(self) -> u64 {
        self.base
    }

    /// One less than the table's size in bytes.
    pub(super) const fn limit(self) -> u16 {
        self.limit
    }
}

impl fmt::Debug for TablePointer {
    /// Hand-written because fields of a packed struct cannot be borrowed, which
    /// is what a derived implementation does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TablePointer")
            .field("base", &self.base())
            .field("limit", &self.limit())
            .finish()
    }
}
