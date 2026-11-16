use vstd::prelude::verus;
#[cfg(verus_only)]
use vstd::prelude::*;

verus! {

pub open spec fn state_moves_only_to_sealed(
    before: crate::ObjectState,
    after: crate::ObjectState,
) -> bool {
    before == after || (before == crate::ObjectState::Writable && after
        != crate::ObjectState::Writable)
}

pub open spec fn state_code(state: crate::ObjectState) -> u64 {
    match state {
        crate::ObjectState::Writable => 0,
        crate::ObjectState::SealedRead => 1,
        crate::ObjectState::SealedExec => 2,
    }
}

pub open spec fn sealed_as(seal: crate::Seal) -> crate::ObjectState {
    match seal {
        crate::Seal::Read => crate::ObjectState::SealedRead,
        crate::Seal::Execute => crate::ObjectState::SealedExec,
    }
}

impl crate::StateWord {
    pub open(crate) spec fn spec_state(self) -> crate::ObjectState {
        self.state
    }

    pub open(crate) spec fn spec_writers(self) -> u32 {
        self.writers
    }

    pub open(crate) spec fn spec_bits(self) -> u64 {
        ((self.writers as u64) << 2u32) | state_code(self.state)
    }
}

pub open spec fn seal_admitted(word: crate::StateWord) -> bool {
    word.spec_state() == crate::ObjectState::Writable && word.spec_writers() == 0
}

pub(crate) proof fn lemma_state_word_packs(writers: u64, code: u64)
    requires
        writers <= 0xffff_ffff,
        code <= 2,
    ensures
        ((writers << 2u32) | code) >> 34u32 == 0,
        ((writers << 2u32) | code) & 3 == code,
        ((writers << 2u32) | code) >> 2u32 == writers,
{
    assert(((writers << 2u32) | code) >> 34u32 == 0 && ((writers << 2u32) | code) & 3 == code
        && ((writers << 2u32) | code) >> 2u32 == writers) by (bit_vector)
        requires
            writers <= 0xffff_ffff,
            code <= 2,
    ;
}

pub(crate) proof fn lemma_state_word_unpacks(bits: u64)
    requires
        bits >> 34u32 == 0,
    ensures
        bits >> 2u32 <= 0xffff_ffff,
        ((bits >> 2u32) << 2u32) | (bits & 3) == bits,
{
    assert(bits >> 2u32 <= 0xffff_ffff && ((bits >> 2u32) << 2u32) | (bits & 3) == bits)
        by (bit_vector)
        requires
            bits >> 34u32 == 0,
    ;
}

impl crate::Quota {
    #[verifier::type_invariant]
    pub(crate) open spec fn charged_within_limit(self) -> bool {
        self.used <= self.limit
    }

    pub open(crate) spec fn spec_limit(self) -> u32 {
        self.limit
    }

    pub open(crate) spec fn spec_used(self) -> u32 {
        self.used
    }

    pub open(crate) spec fn spec_bits(self) -> u64 {
        ((self.limit as u64) << 32u32) | (self.used as u64)
    }
}

pub open spec fn quota_covers(quota: crate::Quota, pages: u32) -> bool {
    quota.spec_used() + pages <= quota.spec_limit()
}

pub(crate) proof fn lemma_quota_packs(limit: u64, used: u64)
    requires
        limit <= 0xffff_ffff,
        used <= 0xffff_ffff,
    ensures
        ((limit << 32u32) | used) >> 32u32 == limit,
        ((limit << 32u32) | used) & 0xffff_ffff == used,
{
    assert(((limit << 32u32) | used) >> 32u32 == limit && ((limit << 32u32) | used) & 0xffff_ffff
        == used) by (bit_vector)
        requires
            limit <= 0xffff_ffff,
            used <= 0xffff_ffff,
    ;
}

pub(crate) proof fn lemma_quota_unpacks(bits: u64)
    ensures
        bits >> 32u32 <= 0xffff_ffff,
        bits & 0xffff_ffff <= 0xffff_ffff,
        ((bits >> 32u32) << 32u32) | (bits & 0xffff_ffff) == bits,
{
    assert(bits >> 32u32 <= 0xffff_ffff && bits & 0xffff_ffff <= 0xffff_ffff && ((bits >> 32u32)
        << 32u32) | (bits & 0xffff_ffff) == bits) by (bit_vector);
}

impl crate::Extent {
    #[verifier::type_invariant]
    pub(crate) open spec fn nonempty_in_range(self) -> bool {
        extent_valid(self)
    }

    pub open(crate) spec fn spec_first(self) -> u64 {
        self.first
    }

    pub open(crate) spec fn spec_pages(self) -> u64 {
        self.pages
    }

    pub open(crate) spec fn spec_end(self) -> int {
        self.first + self.pages
    }
}

pub open spec fn extent_valid(extent: crate::Extent) -> bool {
    extent.spec_pages() > 0 && extent.spec_end() <= u64::MAX
}

pub open spec fn extents_overlap(a: crate::Extent, b: crate::Extent) -> bool {
    a.spec_first() < b.spec_end() && b.spec_first() < a.spec_end()
}

pub open spec fn extent_contains(outer: crate::Extent, inner: crate::Extent) -> bool {
    outer.spec_first() <= inner.spec_first() && inner.spec_end() <= outer.spec_end()
}

impl<T> crate::Records<T> {
    #[verifier::type_invariant]
    pub(crate) open spec fn held_below_count(self) -> bool {
        &&& self.count <= crate::RECORDS
        &&& forall|i: int| 0 <= i < self.count ==> #[trigger] self.entries@[i] is Some
        &&& forall|i: int| self.count <= i < crate::RECORDS ==> #[trigger] self.entries@[i] is None
        &&& records_sorted_and_disjoint(self.spec_records())
    }

    pub open(crate) spec fn spec_records(self) -> Seq<crate::Record<T>> {
        Seq::new(self.count as nat, |i: int| self.entries@[i]->Some_0)
    }
}

pub open spec fn records_sorted_and_disjoint<T>(records: Seq<crate::Record<T>>) -> bool {
    &&& forall|i: int| 0 <= i < records.len() ==> extent_valid(#[trigger] records[i].extent)
    &&& forall|i: int, j: int|
        0 <= i < j < records.len() ==> (#[trigger] records[i]).extent.spec_end() <= (
        #[trigger] records[j]).extent.spec_first()
}

pub open spec fn free_in<T>(records: Seq<crate::Record<T>>, extent: crate::Extent) -> bool {
    forall|i: int| 0 <= i < records.len() ==> !extents_overlap(#[trigger] records[i].extent, extent)
}

pub open spec fn found_covers<T>(
    records: Seq<crate::Record<T>>,
    record: crate::Record<T>,
    extent: crate::Extent,
) -> bool {
    extent_contains(record.extent, extent) && exists|i: int|
        0 <= i < records.len() && #[trigger] records[i] == record
}

pub open spec fn none_covers<T>(records: Seq<crate::Record<T>>, extent: crate::Extent) -> bool {
    forall|i: int|
        0 <= i < records.len() ==> !extent_contains(#[trigger] records[i].extent, extent)
}

pub open spec fn placed_free<T>(
    records: Seq<crate::Record<T>>,
    range: crate::Extent,
    pages: u64,
    align: u64,
    start: u64,
) -> bool {
    &&& align > 0
    &&& start % align == 0
    &&& range.spec_first() <= start
    &&& start + pages <= range.spec_end()
    &&& forall|i: int|
        0 <= i < records.len() ==> (#[trigger] records[i]).extent.spec_end() <= start || start
            + pages <= records[i].extent.spec_first()
}

pub open(crate) spec fn carved<T>(
    records: Seq<crate::Record<T>>,
    at: int,
    extent: crate::Extent,
    value: T,
) -> Seq<crate::Record<T>> {
    records.subrange(0, at) + carved_pieces(records[at], extent, value) + records.subrange(
        at + 1,
        records.len() as int,
    )
}

pub open(crate) spec fn carved_pieces<T>(
    outer: crate::Record<T>,
    extent: crate::Extent,
    value: T,
) -> Seq<crate::Record<T>> {
    let left = crate::Record {
        extent: crate::Extent {
            first: outer.extent.first,
            pages: (extent.first - outer.extent.first) as u64,
        },
        value: outer.value,
    };
    let middle = crate::Record { extent, value };
    let right = crate::Record {
        extent: crate::Extent {
            first: extent.spec_end() as u64,
            pages: (outer.extent.spec_end() - extent.spec_end()) as u64,
        },
        value: outer.value,
    };
    let has_left = outer.extent.spec_first() < extent.spec_first();
    let has_right = extent.spec_end() < outer.extent.spec_end();
    if has_left && has_right {
        seq![left, middle, right]
    } else if has_left {
        seq![left, middle]
    } else if has_right {
        seq![middle, right]
    } else {
        seq![middle]
    }
}

pub(crate) proof fn lemma_sorted_after<T>(records: Seq<crate::Record<T>>, at: int)
    requires
        records_sorted_and_disjoint(records),
        0 <= at < records.len(),
    ensures
        forall|j: int|
            at < j < records.len() ==> records[at].extent.spec_end() <= (
            #[trigger] records[j]).extent.spec_first() && records[at].extent.spec_first()
                < records[j].extent.spec_first(),
{
    assert(extent_valid(records[at].extent));
}

pub(crate) proof fn lemma_inserted_sorted<T>(
    records: Seq<crate::Record<T>>,
    at: int,
    record: crate::Record<T>,
)
    requires
        records_sorted_and_disjoint(records),
        0 <= at <= records.len(),
        extent_valid(record.extent),
        free_in(records, record.extent),
        forall|i: int|
            0 <= i < at ==> (#[trigger] records[i]).extent.spec_first()
                <= record.extent.spec_first(),
        forall|i: int|
            at <= i < records.len() ==> (#[trigger] records[i]).extent.spec_first()
                > record.extent.spec_first(),
    ensures
        records_sorted_and_disjoint(records.insert(at, record)),
{
    assert forall|i: int| 0 <= i < at implies (#[trigger] records[i]).extent.spec_end()
        <= record.extent.spec_first() by {
        assert(!extents_overlap(records[i].extent, record.extent));
    }
    assert forall|i: int| at <= i < records.len() implies record.extent.spec_end() <= (
    #[trigger] records[i]).extent.spec_first() by {
        assert(!extents_overlap(records[i].extent, record.extent));
        assert(extent_valid(records[i].extent));
    }
    let inserted = records.insert(at, record);
    assert forall|i: int, j: int| 0 <= i < j < inserted.len() implies (
    #[trigger] inserted[i]).extent.spec_end() <= (#[trigger] inserted[j]).extent.spec_first() by {
        if j < at {
            assert(records[i].extent.spec_end() <= records[j].extent.spec_first());
        } else if j == at {
            assert(records[i].extent.spec_end() <= record.extent.spec_first());
        } else if i < at {
            assert(records[i].extent.spec_end() <= records[j - 1].extent.spec_first());
        } else if i == at {
            assert(record.extent.spec_end() <= records[j - 1].extent.spec_first());
        } else {
            assert(records[i - 1].extent.spec_end() <= records[j - 1].extent.spec_first());
        }
    }
    assert forall|i: int| 0 <= i < inserted.len() implies extent_valid(
        #[trigger] inserted[i].extent,
    ) by {
        if i < at {
            assert(extent_valid(records[i].extent));
        } else if i > at {
            assert(extent_valid(records[i - 1].extent));
        }
    }
}

pub(crate) proof fn lemma_removed_sorted<T>(records: Seq<crate::Record<T>>, at: int)
    requires
        records_sorted_and_disjoint(records),
        0 <= at < records.len(),
    ensures
        records_sorted_and_disjoint(records.remove(at)),
{
    let removed = records.remove(at);
    assert forall|i: int, j: int| 0 <= i < j < removed.len() implies (
    #[trigger] removed[i]).extent.spec_end() <= (#[trigger] removed[j]).extent.spec_first() by {
        let si = if i < at { i } else { i + 1 };
        let sj = if j < at { j } else { j + 1 };
        assert(records[si].extent.spec_end() <= records[sj].extent.spec_first());
    }
    assert forall|i: int| 0 <= i < removed.len() implies extent_valid(
        #[trigger] removed[i].extent,
    ) by {
        let si = if i < at { i } else { i + 1 };
        assert(extent_valid(records[si].extent));
    }
}

pub(crate) proof fn lemma_carved_sorted<T>(
    records: Seq<crate::Record<T>>,
    at: int,
    extent: crate::Extent,
    value: T,
)
    requires
        records_sorted_and_disjoint(records),
        0 <= at < records.len(),
        extent_valid(extent),
        extent_contains(records[at].extent, extent),
    ensures
        records_sorted_and_disjoint(carved(records, at, extent, value)),
{
    let outer = records[at];
    assert(extent_valid(outer.extent));
    let pieces = carved_pieces(outer, extent, value);
    lemma_carved_pieces(outer, extent, value);
    lemma_carved_index(records, at, extent, value);
    let carved = carved(records, at, extent, value);
    let shift = pieces.len() - 1;
    assert forall|i: int, j: int| 0 <= i < j < carved.len() implies (
    #[trigger] carved[i]).extent.spec_end() <= (#[trigger] carved[j]).extent.spec_first() by {
        if j < at {
            assert(records[i].extent.spec_end() <= records[j].extent.spec_first());
        } else if i < at && j < at + pieces.len() {
            assert(records[i].extent.spec_end() <= outer.extent.spec_first());
            assert(outer.extent.spec_first() <= pieces[j - at].extent.spec_first());
        } else if i < at {
            assert(records[i].extent.spec_end() <= records[j - shift].extent.spec_first());
        } else if i < at + pieces.len() && j < at + pieces.len() {
            assert(pieces[i - at].extent.spec_end() <= pieces[j - at].extent.spec_first());
        } else if i < at + pieces.len() {
            assert(pieces[i - at].extent.spec_end() <= outer.extent.spec_end());
            assert(outer.extent.spec_end() <= records[j - shift].extent.spec_first());
        } else {
            assert(records[i - shift].extent.spec_end() <= records[j
                - shift].extent.spec_first());
        }
    }
    assert forall|i: int| 0 <= i < carved.len() implies extent_valid(
        #[trigger] carved[i].extent,
    ) by {
        if i < at {
            assert(extent_valid(records[i].extent));
        } else if i < at + pieces.len() {
            assert(extent_valid(pieces[i - at].extent));
        } else {
            assert(extent_valid(records[i - shift].extent));
        }
    }
}

pub(crate) proof fn lemma_carved_index<T>(
    records: Seq<crate::Record<T>>,
    at: int,
    extent: crate::Extent,
    value: T,
)
    requires
        0 <= at < records.len(),
    ensures
        carved(records, at, extent, value).len() == records.len() - 1 + carved_pieces(
            records[at],
            extent,
            value,
        ).len(),
        forall|k: int|
            0 <= k < carved(records, at, extent, value).len() ==> #[trigger] carved(
                records,
                at,
                extent,
                value,
            )[k] == if k < at {
                records[k]
            } else if k < at + carved_pieces(records[at], extent, value).len() {
                carved_pieces(records[at], extent, value)[k - at]
            } else {
                records[k - carved_pieces(records[at], extent, value).len() + 1]
            },
{
}

pub(crate) proof fn lemma_carved_pieces<T>(outer: crate::Record<T>, extent: crate::Extent, value: T)
    requires
        extent_valid(outer.extent),
        extent_valid(extent),
        extent_contains(outer.extent, extent),
    ensures
        records_sorted_and_disjoint(carved_pieces(outer, extent, value)),
        1 <= carved_pieces(outer, extent, value).len() <= 3,
        forall|k: int|
            0 <= k < carved_pieces(outer, extent, value).len() ==> outer.extent.spec_first() <= (
            #[trigger] carved_pieces(outer, extent, value)[k]).extent.spec_first()
                && carved_pieces(outer, extent, value)[k].extent.spec_end()
                <= outer.extent.spec_end(),
{
    let pieces = carved_pieces(outer, extent, value);
    assert forall|i: int, j: int| 0 <= i < j < pieces.len() implies (
    #[trigger] pieces[i]).extent.spec_end() <= (#[trigger] pieces[j]).extent.spec_first() by {}
}

impl crate::MapperSlot {
    pub open(crate) spec fn spec_state(self) -> crate::MapperState {
        self.state
    }

    pub open(crate) spec fn spec_generation(self) -> u32 {
        self.generation
    }
}

pub open spec fn mapper_names(slot: crate::MapperSlot, event: crate::MapperEvent) -> bool {
    match event {
        crate::MapperEvent::Claim => true,
        crate::MapperEvent::Commit(generation) => generation == slot.spec_generation(),
        crate::MapperEvent::Abort(generation) => generation == slot.spec_generation(),
        crate::MapperEvent::Unmap(generation) => generation == slot.spec_generation(),
        crate::MapperEvent::Revoke(generation) => generation == slot.spec_generation(),
        crate::MapperEvent::Finish(generation) => generation == slot.spec_generation(),
    }
}

pub open(crate) spec fn mapper_with(
    slot: crate::MapperSlot,
    state: crate::MapperState,
) -> crate::MapperSlot {
    crate::MapperSlot { state, generation: slot.generation }
}

pub open(crate) spec fn mapper_freed(slot: crate::MapperSlot) -> crate::MapperSlot {
    if slot.generation < u32::MAX {
        crate::MapperSlot {
            state: crate::MapperState::Free,
            generation: (slot.generation + 1) as u32,
        }
    } else {
        mapper_with(slot, crate::MapperState::Retired)
    }
}

pub open(crate) spec fn mapper_listed(
    slot: crate::MapperSlot,
    event: crate::MapperEvent,
) -> Option<crate::MapperSlot> {
    if !mapper_names(slot, event) {
        None
    } else {
        match (slot.state, event) {
            (crate::MapperState::Free, crate::MapperEvent::Claim) => Some(
                mapper_with(slot, crate::MapperState::Reserved),
            ),
            (crate::MapperState::Reserved, crate::MapperEvent::Commit(_)) => Some(
                mapper_with(slot, crate::MapperState::Occupied),
            ),
            (crate::MapperState::Reserved, crate::MapperEvent::Revoke(_)) => Some(
                mapper_with(slot, crate::MapperState::Revoking),
            ),
            (crate::MapperState::Occupied, crate::MapperEvent::Revoke(_)) => Some(
                mapper_with(slot, crate::MapperState::Revoking),
            ),
            (crate::MapperState::Reserved, crate::MapperEvent::Abort(_)) => Some(
                mapper_freed(slot),
            ),
            (crate::MapperState::Occupied, crate::MapperEvent::Unmap(_)) => Some(
                mapper_freed(slot),
            ),
            (crate::MapperState::Revoking, crate::MapperEvent::Finish(_)) => Some(
                mapper_freed(slot),
            ),
            _ => None,
        }
    }
}

pub open spec fn mapper_revoking_refuses(
    slot: crate::MapperSlot,
    event: crate::MapperEvent,
) -> bool {
    slot.spec_state() == crate::MapperState::Revoking && (event is Commit || event is Abort
        || event is Unmap)
}

} // verus!
