#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus, verus_spec, verus_verify};

#[verus_verify]
pub const RECORDS: usize = 64;

// Page numbers, not addresses: `first` and `pages` count 4 KiB pages.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Extent {
    pub(crate) first: u64,
    pub(crate) pages: u64,
}

#[verus_verify]
impl Extent {
    #[verus_spec(r =>
        ensures
            match r {
                Some(extent) => extent.spec_first() == first && extent.spec_pages() == pages
                    && crate::proof::extent_valid(extent),
                None => pages == 0 || first + pages > u64::MAX,
            },
    )]
    pub fn new(first: u64, pages: u64) -> Option<Self> {
        if pages == 0 || first.checked_add(pages).is_none() {
            return None;
        }
        Some(Self { first, pages })
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_first(),
    )]
    pub fn first(self) -> u64 {
        self.first
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_pages(),
            r > 0,
    )]
    pub fn pages(self) -> u64 {
        proof! {
            use_type_invariant(&self);
        }
        self.pages
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_end(),
    )]
    pub fn end(self) -> u64 {
        proof! {
            use_type_invariant(&self);
        }
        self.first + self.pages
    }

    #[verus_spec(r =>
        ensures
            r == crate::proof::extents_overlap(self, other),
    )]
    pub fn overlaps(self, other: Self) -> bool {
        self.first < other.end() && other.first < self.end()
    }

    #[verus_spec(r =>
        ensures
            r == crate::proof::extent_contains(self, other),
    )]
    pub fn contains(self, other: Self) -> bool {
        self.first <= other.first && other.end() <= self.end()
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Record<T> {
    pub extent: Extent,
    pub value: T,
}

#[verus_verify]
#[derive(PartialEq, Eq, Debug)]
pub struct Records<T> {
    pub(crate) entries: [Option<Record<T>>; RECORDS],
    pub(crate) count: usize,
}

verus! {

impl<T: Copy> Records<T> {
    pub exec const EMPTY: Self = Self {
        entries: [None; RECORDS],
        count: 0,
    };
}

} // verus!

// Written out because a derived clone has no specification to carry the type invariant.
#[verus_verify]
impl<T: Copy> Clone for Records<T> {
    fn clone(&self) -> Self {
        proof! {
            use_type_invariant(self);
        }
        Self {
            entries: self.entries,
            count: self.count,
        }
    }
}

#[verus_verify]
impl<T: Copy> Records<T> {
    #[verus_spec(r =>
        ensures
            r == self.spec_records().len(),
            r <= RECORDS,
    )]
    pub fn count(&self) -> usize {
        proof! {
            use_type_invariant(self);
        }
        self.count
    }

    #[verus_spec(r =>
        ensures
            r == if position < self.spec_records().len() {
                Some(self.spec_records()[position as int])
            } else {
                None
            },
    )]
    pub fn get(&self, position: usize) -> Option<Record<T>> {
        proof! {
            use_type_invariant(self);
        }
        if position < self.count {
            self.entries[position]
        } else {
            None
        }
    }

    #[verus_spec(r =>
        ensures
            match r {
                Some(record) => crate::proof::found_covers(self.spec_records(), record, extent),
                None => crate::proof::none_covers(self.spec_records(), extent),
            },
    )]
    pub fn covering(&self, extent: Extent) -> Option<Record<T>> {
        let (_, record) = self.covering_at(extent)?;
        Some(record)
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(()) => crate::proof::free_in(old(self).spec_records(), extent)
                    && exists|at: int| 0 <= at <= old(self).spec_records().len()
                        && final(self).spec_records()
                            == old(self).spec_records().insert(at, Record { extent, value }),
                Err(RecordRefusal::Overlap) => !crate::proof::free_in(
                    old(self).spec_records(),
                    extent,
                ) && *final(self) == *old(self),
                Err(RecordRefusal::Full) => old(self).spec_records().len() == RECORDS
                    && *final(self) == *old(self),
                Err(_) => false,
            },
    )]
    pub fn insert(&mut self, extent: Extent, value: T) -> Result<(), RecordRefusal> {
        proof! {
            use_type_invariant(&*self);
            use_type_invariant(&extent);
        }
        if !self.is_free(extent) {
            return Err(RecordRefusal::Overlap);
        }
        if self.count >= RECORDS {
            return Err(RecordRefusal::Full);
        }
        let at = self.position_for(extent);
        let record = Record { extent, value };
        proof! {
            crate::proof::lemma_inserted_sorted(self.spec_records(), at as int, record);
        }
        insert_at(&mut self.entries, &mut self.count, at, record);
        Ok(())
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(value) => exists|at: int| 0 <= at < old(self).spec_records().len()
                    && old(self).spec_records()[at] == (Record { extent, value })
                    && final(self).spec_records() == old(self).spec_records().remove(at),
                Err(RecordRefusal::NotFound) => (forall|at: int|
                    0 <= at < old(self).spec_records().len()
                        ==> (#[trigger] old(self).spec_records()[at]).extent != extent)
                    && *final(self) == *old(self),
                Err(_) => false,
            },
    )]
    pub fn remove(&mut self, extent: Extent) -> Result<T, RecordRefusal> {
        proof! {
            use_type_invariant(&*self);
        }
        let Some((at, record)) = self.covering_at(extent) else {
            return Err(RecordRefusal::NotFound);
        };
        if record.extent.first != extent.first || record.extent.pages != extent.pages {
            proof! {
                use_type_invariant(&extent);
                let records = self.spec_records();
                assert forall|i: int| 0 <= i < records.len() implies (#[trigger] records[i]).extent
                    != extent by {
                    if records[i].extent == extent && i != at {
                        let other = records[at as int].extent;
                        assert(other.spec_end() <= extent.spec_first() || extent.spec_end()
                            <= other.spec_first());
                    }
                }
            }
            return Err(RecordRefusal::NotFound);
        }
        proof! {
            crate::proof::lemma_removed_sorted(self.spec_records(), at as int);
        }
        remove_at(&mut self.entries, &mut self.count, at);
        Ok(record.value)
    }

    // Replaces the part of one record that `extent` covers; the remainders keep that record's value.
    #[verus_spec(r =>
        ensures
            match r {
                Ok(kept) => exists|at: int| 0 <= at < old(self).spec_records().len()
                    && crate::proof::extent_contains(old(self).spec_records()[at].extent, extent)
                    && (#[trigger] old(self).spec_records()[at]).value == kept
                    && final(self).spec_records()
                        == crate::proof::carved(old(self).spec_records(), at, extent, value),
                Err(RecordRefusal::NotCovered) => crate::proof::none_covers(
                    old(self).spec_records(),
                    extent,
                ) && *final(self) == *old(self),
                Err(RecordRefusal::Full) => !crate::proof::none_covers(
                    old(self).spec_records(),
                    extent,
                ) && *final(self) == *old(self),
                Err(_) => false,
            },
    )]
    pub fn carve(&mut self, extent: Extent, value: T) -> Result<T, RecordRefusal> {
        proof! {
            use_type_invariant(&*self);
            use_type_invariant(&extent);
        }
        let Some((at, record)) = self.covering_at(extent) else {
            return Err(RecordRefusal::NotCovered);
        };
        proof! {
            use_type_invariant(&record.extent);
        }
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
        proof! {
            crate::proof::lemma_carved_sorted(self.spec_records(), at as int, extent, value);
            assert(left is Some <==> record.extent.spec_first() < extent.spec_first());
            assert(right is Some <==> extent.spec_end() < record.extent.spec_end());
        }
        carve_at(
            &mut self.entries,
            &mut self.count,
            at,
            Record { extent, value },
            left,
            right,
            added,
        );
        Ok(record.value)
    }
}

verus! {

impl<T: Copy> Records<T> {
    #[verifier::loop_isolation(false)]
    pub fn is_free(&self, extent: Extent) -> (r: bool)
        ensures
            r == crate::proof::free_in(self.spec_records(), extent),
    {
        proof {
            use_type_invariant(self);
        }
        let mut at = 0;
        while at < self.count
            invariant
                at <= self.count,
                forall|i: int|
                    0 <= i < at ==> !crate::proof::extents_overlap(
                        #[trigger] self.spec_records()[i].extent,
                        extent,
                    ),
            decreases self.count - at,
        {
            let Some(record) = self.entries[at] else {
                return true;
            };
            if record.extent.overlaps(extent) {
                proof {
                    assert(crate::proof::extents_overlap(
                        self.spec_records()[at as int].extent,
                        extent,
                    ));
                }
                return false;
            }
            at += 1;
        }
        true
    }

    #[verifier::loop_isolation(false)]
    fn covering_at(&self, extent: Extent) -> (r: Option<(usize, Record<T>)>)
        ensures
            match r {
                Some((at, record)) => at < self.spec_records().len()
                    && self.spec_records()[at as int] == record
                    && crate::proof::extent_contains(record.extent, extent),
                None => crate::proof::none_covers(self.spec_records(), extent),
            },
    {
        proof {
            use_type_invariant(self);
        }
        let mut at = 0;
        while at < self.count
            invariant
                at <= self.count,
                forall|i: int|
                    0 <= i < at ==> !crate::proof::extent_contains(
                        #[trigger] self.spec_records()[i].extent,
                        extent,
                    ),
            decreases self.count - at,
        {
            let record = self.entries[at]?;
            if record.extent.contains(extent) {
                return Some((at, record));
            }
            at += 1;
        }
        None
    }

    // The least aligned start inside `range` whose extent of `pages` meets no record.
    #[verifier::loop_isolation(false)]
    pub fn first_fit(&self, range: Extent, pages: u64, align: u64) -> (r: Option<u64>)
        ensures
            match r {
                Some(start) => crate::proof::placed_free(
                    self.spec_records(),
                    range,
                    pages,
                    align,
                    start,
                ),
                None => true,
            },
    {
        proof {
            use_type_invariant(self);
            use_type_invariant(&range);
        }
        let mut cursor = range.first;
        let mut at = 0;
        while at < self.count
            invariant
                at <= self.count,
                range.spec_first() <= cursor,
                forall|i: int|
                    0 <= i < at ==> (#[trigger] self.spec_records()[i]).extent.spec_end() <= cursor,
            decreases self.count - at,
        {
            let record = self.entries[at]?;
            if record.extent.end() > cursor {
                let limit = if record.extent.first < range.end() {
                    record.extent.first
                } else {
                    range.end()
                };
                if let Some(start) = fits(cursor, pages, align, limit) {
                    proof {
                        crate::proof::lemma_sorted_after(self.spec_records(), at as int);
                    }
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

    #[verifier::loop_isolation(false)]
    fn position_for(&self, extent: Extent) -> (at: usize)
        ensures
            at <= self.spec_records().len(),
            forall|i: int|
                0 <= i < at ==> (#[trigger] self.spec_records()[i]).extent.spec_first()
                    <= extent.spec_first(),
            forall|i: int|
                at <= i < self.spec_records().len() ==> (
                #[trigger] self.spec_records()[i]).extent.spec_first() > extent.spec_first(),
    {
        proof {
            use_type_invariant(self);
        }
        let mut at = 0;
        while at < self.count
            invariant
                at <= self.count,
                forall|i: int|
                    0 <= i < at ==> (#[trigger] self.spec_records()[i]).extent.spec_first()
                        <= extent.spec_first(),
            decreases self.count - at,
        {
            let Some(record) = self.entries[at] else {
                return at;
            };
            if record.extent.first > extent.first {
                proof {
                    crate::proof::lemma_sorted_after(self.spec_records(), at as int);
                }
                return at;
            }
            at += 1;
        }
        at
    }
}

fn insert_at<T: Copy>(
    entries: &mut [Option<Record<T>>; RECORDS],
    count: &mut usize,
    at: usize,
    record: Record<T>,
)
    requires
        at <= *old(count) < RECORDS,
        forall|i: int| 0 <= i < *old(count) ==> #[trigger] (*old(entries))@[i] is Some,
        forall|i: int| *old(count) <= i < RECORDS ==> #[trigger] (*old(entries))@[i] is None,
    ensures
        *final(count) == *old(count) + 1,
        forall|i: int| 0 <= i < *final(count) ==> #[trigger] (*final(entries))@[i] is Some,
        forall|i: int| *final(count) <= i < RECORDS ==> #[trigger] (*final(entries))@[i] is None,
        crate::proof::records_in(*final(entries), *final(count)) == crate::proof::records_in(
            *old(entries),
            *old(count),
        ).insert(at as int, record),
    no_unwind
{
    open(entries, *count, at, 1);
    entries[at] = Some(record);
    proof {
        assert(crate::proof::records_in(*entries, (*old(count) + 1) as usize)
            =~= crate::proof::records_in(*old(entries), *old(count)).insert(at as int, record));
    }
    *count += 1;
}

fn remove_at<T: Copy>(entries: &mut [Option<Record<T>>; RECORDS], count: &mut usize, at: usize)
    requires
        at < *old(count) <= RECORDS,
        forall|i: int| 0 <= i < *old(count) ==> #[trigger] (*old(entries))@[i] is Some,
        forall|i: int| *old(count) <= i < RECORDS ==> #[trigger] (*old(entries))@[i] is None,
    ensures
        *final(count) == *old(count) - 1,
        forall|i: int| 0 <= i < *final(count) ==> #[trigger] (*final(entries))@[i] is Some,
        forall|i: int| *final(count) <= i < RECORDS ==> #[trigger] (*final(entries))@[i] is None,
        crate::proof::records_in(*final(entries), *final(count)) == crate::proof::records_in(
            *old(entries),
            *old(count),
        ).remove(at as int),
    no_unwind
{
    close(entries, *count, at);
    proof {
        assert(crate::proof::records_in(*entries, (*old(count) - 1) as usize)
            =~= crate::proof::records_in(*old(entries), *old(count)).remove(at as int));
    }
    *count -= 1;
}

// Writes what `carve` leaves of the record at `at`: the left remainder, `middle`, the right one.
fn carve_at<T: Copy>(
    entries: &mut [Option<Record<T>>; RECORDS],
    count: &mut usize,
    at: usize,
    middle: Record<T>,
    left: Option<Extent>,
    right: Option<Extent>,
    added: usize,
)
    requires
        at < *old(count),
        *old(count) + added <= RECORDS,
        forall|i: int| 0 <= i < *old(count) ==> #[trigger] (*old(entries))@[i] is Some,
        forall|i: int| *old(count) <= i < RECORDS ==> #[trigger] (*old(entries))@[i] is None,
        left is Some <==> (*old(entries))@[at as int]->Some_0.extent.spec_first() < middle.extent.spec_first(),
        right is Some <==> middle.extent.spec_end() < (*old(entries))@[at as int]->Some_0.extent.spec_end(),
        left is Some ==> left->Some_0.spec_first() == (*old(entries))@[at as int]->Some_0.extent.spec_first()
            && left->Some_0.spec_pages() == middle.extent.spec_first()
            - (*old(entries))@[at as int]->Some_0.extent.spec_first(),
        right is Some ==> right->Some_0.spec_first() == middle.extent.spec_end()
            && right->Some_0.spec_pages() == (*old(entries))@[at as int]->Some_0.extent.spec_end() - middle.extent.spec_end(),
        added == (if left is Some {
            1int
        } else {
            0int
        }) + (if right is Some {
            1int
        } else {
            0int
        }),
    ensures
        *final(count) == *old(count) + added,
        forall|i: int| 0 <= i < *final(count) ==> #[trigger] (*final(entries))@[i] is Some,
        forall|i: int| *final(count) <= i < RECORDS ==> #[trigger] (*final(entries))@[i] is None,
        crate::proof::records_in(*final(entries), *final(count)) == crate::proof::carved(
            crate::proof::records_in(*old(entries), *old(count)),
            at as int,
            middle.extent,
            middle.value,
        ),
    no_unwind
{
    let Some(outer) = entries[at] else {
        return;
    };
    open(entries, *count, at + 1, added);
    let mut next = at;
    if let Some(left) = left {
        entries[next] = Some(Record { extent: left, value: outer.value });
        next += 1;
    }
    entries[next] = Some(middle);
    next += 1;
    if let Some(right) = right {
        entries[next] = Some(Record { extent: right, value: outer.value });
    }
    proof {
        let extent = middle.extent;
        let value = middle.value;
        let before = crate::proof::records_in(*old(entries), *old(count));
        let after = crate::proof::records_in(*entries, (*old(count) + added) as usize);
        let pieces = crate::proof::carved_pieces(outer, extent, value);
        let carved = crate::proof::carved(before, at as int, extent, value);
        crate::proof::lemma_carved_index(before, at as int, extent, value);
        assert(pieces.len() == added + 1);
        assert forall|k: int| 0 <= k <= added implies (*entries)@[at + k] == Some(
            #[trigger] pieces[k],
        ) by {
            if left is Some {
                assert(left->Some_0 == pieces[0].extent);
            }
            if right is Some {
                assert(right->Some_0 == pieces[added as int].extent);
            }
        }
        assert forall|k: int| 0 <= k < carved.len() implies #[trigger] after[k] == carved[k] by {
            if at <= k <= at + added {
                assert((*entries)@[k] == Some(pieces[k - at]));
            }
        }
        assert(after =~= carved);
    }
    *count += added;
}

fn open<T: Copy>(entries: &mut [Option<Record<T>>; RECORDS], count: usize, at: usize, by: usize)
    requires
        at <= count,
        count + by <= RECORDS,
    ensures
        forall|i: int| 0 <= i < at ==> (*final(entries))@[i] == (*old(entries))@[i],
        forall|i: int|
            at + by <= i < count + by ==> #[trigger] (*final(entries))@[i] == (*old(entries))@[i
                - by],
        forall|i: int|
            count + by <= i < RECORDS ==> #[trigger] (*final(entries))@[i] == (*old(entries))@[i],
    no_unwind
{
    if by == 0 {
        return;
    }
    let mut from = count;
    while from > at
        invariant
            at <= from <= count,
            count + by <= RECORDS,
            forall|i: int| 0 <= i < from ==> #[trigger] (*entries)@[i] == (*old(entries))@[i],
            forall|i: int|
                from + by <= i < count + by ==> #[trigger] (*entries)@[i] == (*old(entries))@[i
                    - by],
            forall|i: int|
                count + by <= i < RECORDS ==> #[trigger] (*entries)@[i] == (*old(entries))@[i],
        decreases from,
    {
        from -= 1;
        entries[from + by] = entries[from];
    }
}

fn close<T: Copy>(entries: &mut [Option<Record<T>>; RECORDS], count: usize, at: usize)
    requires
        at < count <= RECORDS,
    ensures
        forall|i: int| 0 <= i < at ==> #[trigger] (*final(entries))@[i] == (*old(entries))@[i],
        forall|i: int|
            at <= i < count - 1 ==> #[trigger] (*final(entries))@[i] == (*old(entries))@[i + 1],
        (*final(entries))@[count - 1] is None,
        forall|i: int|
            count <= i < RECORDS ==> #[trigger] (*final(entries))@[i] == (*old(entries))@[i],
    no_unwind
{
    let mut to = at;
    while to + 1 < count
        invariant
            at <= to < count <= RECORDS,
            forall|i: int| 0 <= i < at ==> #[trigger] (*entries)@[i] == (*old(entries))@[i],
            forall|i: int|
                at <= i < to ==> #[trigger] (*entries)@[i] == (*old(entries))@[i + 1],
            forall|i: int|
                to <= i < RECORDS ==> #[trigger] (*entries)@[i] == (*old(entries))@[i],
        decreases count - to,
    {
        entries[to] = entries[to + 1];
        to += 1;
    }
    entries[count - 1] = None;
}

} // verus!

#[verus_spec(r =>
    ensures
        match r {
            Some(up) => align > 0 && value <= up && up - value < align && up % align == 0,
            None => true,
        },
)]
pub(crate) fn align_up(value: u64, align: u64) -> Option<u64> {
    if !is_power_of_two(align) {
        return None;
    }
    let rem = value % align;
    if rem == 0 {
        return Some(value);
    }
    proof! {
        let quotient = value / align;
        vstd::arithmetic::div_mod::lemma_fundamental_div_mod(value as int, align as int);
        vstd::arithmetic::div_mod::lemma_mod_bound(value as int, align as int);
        assert(value + (align - rem) == (quotient + 1) * align) by (nonlinear_arith)
            requires
                value == align * quotient + rem,
        ;
        vstd::arithmetic::div_mod::lemma_mod_multiples_basic(quotient + 1, align as int);
    }
    value.checked_add(align - rem)
}

#[verus_spec(r =>
    ensures
        r ==> align > 0,
)]
fn is_power_of_two(align: u64) -> bool {
    align != 0 && align & (align - 1) == 0
}

#[verus_spec(r =>
    ensures
        match r {
            Some(start) => align > 0 && cursor <= start && start % align == 0
                && start + pages <= limit,
            None => true,
        },
)]
fn fits(cursor: u64, pages: u64, align: u64, limit: u64) -> Option<u64> {
    let start = align_up(cursor, align)?;
    if start.checked_add(pages)? <= limit {
        Some(start)
    } else {
        None
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordRefusal {
    Overlap,
    Full,
    NotFound,
    NotCovered,
}
