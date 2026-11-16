use core::fmt;

pub const RECORDS: usize = 64;

// Page numbers, not addresses: `first` and `pages` count 4 KiB pages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Extent {
    first: u64,
    pages: u64,
}

impl Extent {
    pub fn new(first: u64, pages: u64) -> Option<Self> {
        if pages == 0 || first.checked_add(pages).is_none() {
            return None;
        }
        Some(Self { first, pages })
    }

    pub fn first(self) -> u64 {
        self.first
    }

    pub fn pages(self) -> u64 {
        self.pages
    }

    pub fn end(self) -> u64 {
        self.first + self.pages
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.first < other.end() && other.first < self.end()
    }

    pub fn contains(self, other: Self) -> bool {
        self.first <= other.first && other.end() <= self.end()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Record<T> {
    pub extent: Extent,
    pub value: T,
}

// Sorted by first page, pairwise disjoint, held in positions below `count`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Records<T> {
    entries: [Option<Record<T>>; RECORDS],
    count: usize,
}

impl<T: Copy> Records<T> {
    pub const EMPTY: Self = Self {
        entries: [None; RECORDS],
        count: 0,
    };

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn get(&self, position: usize) -> Option<Record<T>> {
        if position < self.count && position < RECORDS {
            self.entries[position]
        } else {
            None
        }
    }

    pub fn is_free(&self, extent: Extent) -> bool {
        let mut at = 0;
        while at < self.count && at < RECORDS {
            if let Some(record) = self.entries[at]
                && record.extent.overlaps(extent)
            {
                return false;
            }
            at += 1;
        }
        true
    }

    pub fn covering(&self, extent: Extent) -> Option<Record<T>> {
        let mut at = 0;
        while at < self.count && at < RECORDS {
            if let Some(record) = self.entries[at]
                && record.extent.contains(extent)
            {
                return Some(record);
            }
            at += 1;
        }
        None
    }

    pub fn insert(&mut self, extent: Extent, value: T) -> Result<(), RecordRefusal> {
        if !self.is_free(extent) {
            return Err(RecordRefusal::Overlap);
        }
        if self.count >= RECORDS {
            return Err(RecordRefusal::Full);
        }
        let at = self.position_for(extent);
        self.open(at, 1);
        self.entries[at] = Some(Record { extent, value });
        Ok(())
    }

    pub fn remove(&mut self, extent: Extent) -> Result<T, RecordRefusal> {
        let mut at = 0;
        while at < self.count && at < RECORDS {
            if let Some(record) = self.entries[at]
                && record.extent == extent
            {
                self.close(at);
                return Ok(record.value);
            }
            at += 1;
        }
        Err(RecordRefusal::NotFound)
    }

    // Replaces the part of one record that `extent` covers; the remainders keep that record's value.
    pub fn carve(&mut self, extent: Extent, value: T) -> Result<T, RecordRefusal> {
        let mut at = 0;
        while at < self.count && at < RECORDS {
            if let Some(record) = self.entries[at]
                && record.extent.contains(extent)
            {
                return self.split(at, record, extent, value);
            }
            at += 1;
        }
        Err(RecordRefusal::NotCovered)
    }

    fn split(
        &mut self,
        at: usize,
        record: Record<T>,
        extent: Extent,
        value: T,
    ) -> Result<T, RecordRefusal> {
        let left = Extent::new(record.extent.first, extent.first - record.extent.first);
        let right = Extent::new(extent.end(), record.extent.end() - extent.end());
        let mut added = 0;
        if left.is_some() {
            added += 1;
        }
        if right.is_some() {
            added += 1;
        }
        if self.count + added > RECORDS {
            return Err(RecordRefusal::Full);
        }
        self.open(at + 1, added);
        let mut next = at;
        if let Some(left) = left {
            self.entries[next] = Some(Record {
                extent: left,
                value: record.value,
            });
            next += 1;
        }
        self.entries[next] = Some(Record { extent, value });
        next += 1;
        if let Some(right) = right {
            self.entries[next] = Some(Record {
                extent: right,
                value: record.value,
            });
        }
        Ok(record.value)
    }

    // The least aligned start inside `range` whose extent of `pages` meets no record.
    pub fn first_fit(&self, range: Extent, pages: u64, align: u64) -> Option<u64> {
        let mut cursor = range.first;
        let mut at = 0;
        while at < self.count && at < RECORDS {
            if let Some(record) = self.entries[at]
                && record.extent.end() > cursor
            {
                let limit = if record.extent.first < range.end() {
                    record.extent.first
                } else {
                    range.end()
                };
                if let Some(start) = fits(cursor, pages, align, limit) {
                    return Some(start);
                }
                if record.extent.first >= range.end() {
                    return None;
                }
                cursor = record.extent.end();
            }
            at += 1;
        }
        fits(cursor, pages, align, range.end())
    }

    fn position_for(&self, extent: Extent) -> usize {
        let mut at = 0;
        while at < self.count && at < RECORDS {
            if let Some(record) = self.entries[at]
                && record.extent.first > extent.first
            {
                return at;
            }
            at += 1;
        }
        at
    }

    fn open(&mut self, at: usize, by: usize) {
        if by == 0 || self.count + by > RECORDS {
            return;
        }
        let mut from = self.count;
        while from > at {
            from -= 1;
            self.entries[from + by] = self.entries[from];
        }
        self.count += by;
    }

    fn close(&mut self, at: usize) {
        if self.count == 0 || self.count > RECORDS {
            return;
        }
        let mut to = at;
        while to + 1 < self.count {
            self.entries[to] = self.entries[to + 1];
            to += 1;
        }
        self.count -= 1;
        self.entries[self.count] = None;
    }
}

pub(crate) fn align_up(value: u64, align: u64) -> Option<u64> {
    if !align.is_power_of_two() {
        return None;
    }
    let mask = align - 1;
    Some(value.checked_add(mask)? & !mask)
}

fn fits(cursor: u64, pages: u64, align: u64, limit: u64) -> Option<u64> {
    let start = align_up(cursor, align)?;
    if start.checked_add(pages)? <= limit {
        Some(start)
    } else {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordRefusal {
    Overlap,
    Full,
    NotFound,
    NotCovered,
}

impl fmt::Display for RecordRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Overlap => "the extent overlaps a record",
            Self::Full => "the space holds its maximum number of records",
            Self::NotFound => "no record has exactly that extent",
            Self::NotCovered => "no single record covers the extent",
        })
    }
}

impl core::error::Error for RecordRefusal {}
