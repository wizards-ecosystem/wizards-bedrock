use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C, packed(2))]
pub(super) struct TablePointer {
    limit: u16,
    base: u64,
}

impl TablePointer {
    pub(super) const fn new(base: u64, limit: u16) -> Self {
        Self { limit, base }
    }

    pub(super) const fn empty() -> Self {
        Self { limit: 0, base: 0 }
    }

    pub(super) const fn base(self) -> u64 {
        self.base
    }

    pub(super) const fn limit(self) -> u16 {
        self.limit
    }
}

// Hand-written: a derived `Debug` borrows fields a packed struct cannot lend.
impl fmt::Debug for TablePointer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TablePointer")
            .field("base", &self.base())
            .field("limit", &self.limit())
            .finish()
    }
}
