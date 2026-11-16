use vstd::prelude::verus;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
use crate::slots::{END, LIVE, RETIRED};
use crate::{Index, Slots, slots::FreeList};
#[cfg(verus_only)]
use crate::{Refusal, Released};

verus! {

impl Index {
    #[verifier::type_invariant]
    pub(crate) open spec fn names_a_slot_of_a_page(self) -> bool {
        self.page < 0x40_0000 && self.slot < 1024
    }

    pub open(crate) spec fn spec_page(self) -> u32 {
        self.page
    }

    pub open(crate) spec fn spec_slot(self) -> u16 {
        self.slot
    }
}

// Arenas, Containment: objects name each other by index and generation. An index is its page
// above ten bits of slot, and the two decode back exactly.
pub open spec fn index_bits(page: u32, slot: u16) -> u32 {
    (page << 10u32) | slot as u32
}

pub proof fn lemma_page_constants()
    ensures
        crate::PAGES == 0x40_0000,
        crate::SLOTS_PER_PAGE == 1024,
{
    assert(1u32 << 22u32 == 0x40_0000u32) by (bit_vector);
    assert(1u16 << 10u32 == 1024u16) by (bit_vector);
}

pub proof fn lemma_bits_name_an_index(bits: u32)
    ensures
        bits >> 10u32 < 0x40_0000,
        bits & 1023u32 < 1024,
        index_bits(bits >> 10u32, (bits & 1023u32) as u16) == bits,
{
    assert(bits >> 10u32 < 0x40_0000u32 && bits & 1023u32 < 1024u32) by (bit_vector);
    assert(((bits >> 10u32) << 10u32) | (bits & 1023u32) == bits) by (bit_vector);
}

pub proof fn lemma_index_names_its_bits(page: u32, slot: u16)
    requires
        page < 0x40_0000,
        slot < 1024,
    ensures
        index_bits(page, slot) >> 10u32 == page,
        index_bits(page, slot) & 1023u32 == slot as u32,
{
    let low = slot as u32;
    assert(((page << 10u32) | low) >> 10u32 == page && ((page << 10u32) | low) & 1023u32 == low)
        by (bit_vector)
        requires
            page < 0x40_0000u32,
            low < 1024u32,
    ;
}

// Arenas, Refusal: the earmark covers the batch. The frames hold every object the free slots do
// not, never fewer, and one frame less would not.
pub open spec fn frames_cover_the_batch(
    objects: u32,
    free_slots: u32,
    per_page: u16,
    frames: u32,
) -> bool {
    let short = if objects <= free_slots {
        0
    } else {
        objects - free_slots
    };
    frames * per_page >= short && frames * per_page < short + per_page
}

pub proof fn lemma_whole_pages(short: u32, per_page: u32, whole: u32)
    requires
        0 < per_page,
        whole == short / per_page,
    ensures
        whole * per_page <= short,
        short < whole * per_page + per_page,
        (short % per_page == 0) == (whole * per_page == short),
        short % per_page != 0 ==> whole < u32::MAX,
        (whole + 1) * per_page == whole * per_page + per_page,
{
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(short as int, per_page as int);
    vstd::arithmetic::div_mod::lemma_mod_bound(short as int, per_page as int);
    assert((whole + 1) * per_page == whole * per_page + per_page) by (nonlinear_arith);
    assert(per_page * whole == whole * per_page) by (nonlinear_arith);
    if short % per_page != 0 {
        assert(per_page >= 2);
        assert(whole < u32::MAX) by (nonlinear_arith)
            requires
                whole * per_page <= short,
                short <= u32::MAX,
                per_page >= 2,
        ;
    }
}

// The free slots in list order: `steps` slots from `from`, each found through the one before.
pub(crate) open spec fn chain(links: Seq<u16>, from: u16, steps: nat) -> Seq<u16>
    decreases steps,
{
    if steps == 0 {
        Seq::empty()
    } else {
        seq![from] + chain(links, links[from as int], (steps - 1) as nat)
    }
}

pub(crate) open spec fn count(links: Seq<u16>, value: u16) -> nat
    decreases links.len(),
{
    if links.len() == 0 {
        0
    } else {
        count(links.drop_last(), value) + if links.last() == value {
            1nat
        } else {
            0nat
        }
    }
}

pub(crate) open spec fn free_link(link: u16, n: int) -> bool {
    link < n || link == END
}

pub(crate) open spec fn first_link(at: int, n: int) -> u16 {
    if at + 1 < n {
        (at + 1) as u16
    } else {
        END
    }
}

// The list from `head` holds `free` distinct free slots ending at `tail`, and the counts add up.
pub(crate) open spec fn list_holds(
    links: Seq<u16>,
    head: u16,
    tail: u16,
    free: u16,
    live: u16,
) -> bool {
    let n = links.len() as int;
    let list = chain(links, head, free as nat);
    &&& 1 <= n <= 1024
    &&& list.len() == free
    &&& list.no_duplicates()
    &&& forall|i: int|
        #![trigger list[i]]
        0 <= i < free ==> list[i] < n && free_link(links[list[i] as int], n)
    &&& free == 0 ==> head == END && tail == END
    &&& free > 0 ==> tail == list[free - 1] && links[tail as int] == END
    &&& live == count(links, LIVE)
    &&& free + live + count(links, RETIRED) == n
}

impl<const N: usize> FreeList<N> {
    pub(crate) open spec fn holds(self) -> bool {
        list_holds(self.next_free@, self.head, self.tail, self.free, self.live)
    }

    pub(crate) open spec fn links(self) -> Seq<u16> {
        self.next_free@
    }
}

impl<const N: usize> Slots<N> {
    #[verifier::type_invariant]
    pub(crate) open spec fn free_list_holds(self) -> bool {
        self.list.holds()
    }

    pub open(crate) spec fn is_live(self, slot: int) -> bool {
        0 <= slot < N && self.list.next_free@[slot] == LIVE
    }

    pub open(crate) spec fn is_retired(self, slot: int) -> bool {
        0 <= slot < N && self.list.next_free@[slot] == RETIRED
    }

    pub open(crate) spec fn generation_of(self, slot: int) -> u32 {
        self.generation@[slot]
    }

    pub open(crate) spec fn strong_of(self, slot: int) -> u32 {
        self.strong@[slot]
    }

    pub open(crate) spec fn spec_free(self) -> u16 {
        self.list.free
    }

    pub open(crate) spec fn spec_live(self) -> u16 {
        self.list.live
    }

    pub open(crate) spec fn insert_refuses(self) -> bool {
        self.list.free == 0
    }

    pub open(crate) spec fn resolves_to(self, slot: int, generation: u32) -> bool {
        self.is_live(slot) && self.generation_of(slot) == generation
    }

    pub open(crate) spec fn closes_its_index(self, slot: int) -> bool {
        self.is_retired(slot) || (self.is_live(slot) && self.generation_of(slot) == u32::MAX)
    }
}

// Arenas, Containment: a slot never issues the same generation twice. It issues from a free slot
// at that slot's generation, keeps it while live, and leaves live only by raising it or retiring.
pub open(crate) spec fn slot_never_repeats_a_generation<const N: usize>(
    before: Slots<N>,
    after: Slots<N>,
) -> bool {
    forall|slot: int|
        0 <= slot < N ==> {
            &&& before.generation_of(slot) <= after.generation_of(slot)
            &&& before.is_retired(slot) ==> after.is_retired(slot)
            &&& !before.is_live(slot) && after.is_live(slot) ==> !before.is_retired(slot)
                && after.generation_of(slot) == before.generation_of(slot)
            &&& before.is_live(slot) && after.is_live(slot) ==> after.generation_of(slot)
                == before.generation_of(slot)
            &&& before.is_live(slot) && !after.is_live(slot) ==> after.is_retired(slot)
                || after.generation_of(slot) > before.generation_of(slot)
        }
}

// Arenas, Containment: an index issued before a remove never resolves afterwards.
pub open(crate) spec fn removed_index_never_resolves<const N: usize>(
    after: Slots<N>,
    slot: int,
    generation: u32,
) -> bool {
    !after.resolves_to(slot, generation) && (after.is_retired(slot) || after.generation_of(slot)
        > generation)
}

// Strong counts never underflow, and "now zero" is reported exactly at the fall from one.
pub open(crate) spec fn now_zero_once_per_fall<const N: usize>(
    before: Slots<N>,
    after: Slots<N>,
    slot: int,
    r: Result<Released, Refusal>,
) -> bool {
    match r {
        Ok(Released::NowZero) => before.strong_of(slot) == 1 && after.strong_of(slot) == 0,
        Ok(Released::Held) => before.strong_of(slot) > 1 && after.strong_of(slot) + 1
            == before.strong_of(slot),
        Err(Refusal::Condemned) => before.strong_of(slot) == 0,
        Err(_) => true,
    }
}

pub open(crate) spec fn only_touches<const N: usize>(
    before: Slots<N>,
    after: Slots<N>,
    slot: int,
) -> bool {
    forall|other: int|
        0 <= other < N && other != slot ==> {
            &&& after.is_live(other) == before.is_live(other)
            &&& after.is_retired(other) == before.is_retired(other)
            &&& after.generation_of(other) == before.generation_of(other)
            &&& after.strong_of(other) == before.strong_of(other)
        }
}

pub open(crate) spec fn counts_only<const N: usize>(
    before: Slots<N>,
    after: Slots<N>,
    slot: int,
) -> bool {
    &&& after.list == before.list
    &&& after.generation == before.generation
    &&& forall|other: int|
        0 <= other < N && other != slot ==> after.strong_of(other) == before.strong_of(other)
}

pub open(crate) spec fn below_the_floor<const N: usize>(
    slots: Slots<N>,
    slot: int,
    bound: u32,
) -> bool {
    !slots.is_retired(slot) && if slots.is_live(slot) {
        slots.generation_of(slot) < bound
    } else {
        slots.generation_of(slot) <= bound
    }
}

pub(crate) open spec fn same_state(before: u16, after: u16) -> bool {
    (before == LIVE) == (after == LIVE) && (before == RETIRED) == (after == RETIRED)
}

proof fn lemma_chain_len(links: Seq<u16>, from: u16, steps: nat)
    ensures
        chain(links, from, steps).len() == steps,
    decreases steps,
{
    if steps > 0 {
        lemma_chain_len(links, links[from as int], (steps - 1) as nat);
    }
}

// A chain reads only the links of its slots before the last, so other links may change.
proof fn lemma_chain_agree(a: Seq<u16>, b: Seq<u16>, from: u16, steps: nat)
    requires
        forall|i: int|
            #![trigger chain(a, from, steps)[i]]
            0 <= i < steps - 1 ==> a[chain(a, from, steps)[i] as int] == b[chain(
                a,
                from,
                steps,
            )[i] as int],
    ensures
        chain(b, from, steps) == chain(a, from, steps),
    decreases steps,
{
    reveal_with_fuel(chain, 2);
    if steps > 1 {
        let rest = chain(a, a[from as int], (steps - 1) as nat);
        lemma_chain_len(a, a[from as int], (steps - 1) as nat);
        assert(chain(a, from, steps)[0] == from);
        assert(a[from as int] == b[from as int]);
        assert forall|i: int|
            #![trigger rest[i]]
            0 <= i < steps - 2 implies a[rest[i] as int] == b[rest[i] as int] by {
            assert(rest[i] == chain(a, from, steps)[i + 1]);
        }
        lemma_chain_agree(a, b, a[from as int], (steps - 1) as nat);
    } else if steps == 1 {
        assert(chain(b, from, 1) =~= chain(a, from, 1));
    }
}

proof fn lemma_chain_push(links: Seq<u16>, from: u16, steps: nat)
    requires
        steps >= 1,
    ensures
        chain(links, from, steps + 1) == chain(links, from, steps).push(
            links[chain(links, from, steps)[steps - 1] as int],
        ),
    decreases steps,
{
    reveal_with_fuel(chain, 3);
    let next = links[from as int];
    lemma_chain_len(links, from, steps);
    lemma_chain_len(links, next, (steps - 1) as nat);
    if steps == 1 {
        assert(chain(links, next, 1) =~= seq![next]);
        assert(chain(links, from, 2) =~= chain(links, from, 1).push(next));
    } else {
        lemma_chain_push(links, next, (steps - 1) as nat);
        assert(chain(links, from, steps)[steps - 1] == chain(links, next, (steps - 1) as nat)[steps
            - 2]);
        assert(chain(links, from, steps + 1) =~= chain(links, from, steps).push(
            links[chain(links, from, steps)[steps - 1] as int],
        ));
    }
}

proof fn lemma_count_update(links: Seq<u16>, at: int, value: u16, counted: u16)
    requires
        0 <= at < links.len(),
    ensures
        count(links.update(at, value), counted) + (if links[at] == counted {
            1int
        } else {
            0int
        }) == count(links, counted) + (if value == counted {
            1int
        } else {
            0int
        }),
    decreases links.len(),
{
    let updated = links.update(at, value);
    if at == links.len() - 1 {
        assert(updated.drop_last() =~= links.drop_last());
    } else {
        assert(updated.drop_last() =~= links.drop_last().update(at, value));
        lemma_count_update(links.drop_last(), at, value, counted);
    }
}

proof fn lemma_count_absent(links: Seq<u16>, counted: u16)
    requires
        forall|at: int| 0 <= at < links.len() ==> links[at] != counted,
    ensures
        count(links, counted) == 0,
    decreases links.len(),
{
    if links.len() > 0 {
        lemma_count_absent(links.drop_last(), counted);
    }
}

proof fn lemma_count_present(links: Seq<u16>, at: int, counted: u16)
    requires
        0 <= at < links.len(),
        links[at] == counted,
    ensures
        count(links, counted) > 0,
    decreases links.len(),
{
    if at < links.len() - 1 {
        lemma_count_present(links.drop_last(), at, counted);
    }
}

pub(crate) proof fn lemma_none_live<const N: usize>(slots: Slots<N>)
    ensures
        (count(slots.list.next_free@, LIVE) == 0) == (forall|slot: int|
            0 <= slot < N ==> !slots.is_live(slot)),
{
    let links = slots.list.next_free@;
    if forall|slot: int| 0 <= slot < N ==> !slots.is_live(slot) {
        assert forall|at: int| 0 <= at < links.len() implies links[at] != LIVE by {
            assert(!slots.is_live(at));
        }
        lemma_count_absent(links, LIVE);
    } else {
        let slot = choose|slot: int| 0 <= slot < N && slots.is_live(slot);
        lemma_count_present(links, slot, LIVE);
    }
}

pub(crate) proof fn lemma_head_names_a_free_slot(
    links: Seq<u16>,
    head: u16,
    tail: u16,
    free: u16,
    live: u16,
)
    requires
        list_holds(links, head, tail, free, live),
    ensures
        (free > 0) == ((head as int) < links.len()),
        free > 0 ==> free_link(links[head as int], links.len() as int),
{
    if free > 0 {
        assert(chain(links, head, free as nat)[0] == head);
    }
}

proof fn lemma_counting_up(links: Seq<u16>, k: int)
    requires
        1 <= links.len() <= 1024,
        0 <= k <= links.len(),
        forall|at: int| 0 <= at < links.len() ==> links[at] == first_link(at, links.len() as int),
    ensures
        chain(links, k as u16, (links.len() - k) as nat) =~= Seq::new(
            (links.len() - k) as nat,
            |i: int| (k + i) as u16,
        ),
    decreases links.len() - k,
{
    reveal_with_fuel(chain, 2);
    let n = links.len() as int;
    if k < n {
        lemma_counting_up(links, k + 1);
        if k + 1 < n {
            assert(links[k] == (k + 1) as u16);
        }
    }
}

pub(crate) proof fn lemma_first_list(links: Seq<u16>)
    requires
        1 <= links.len() <= 1024,
        forall|at: int| 0 <= at < links.len() ==> links[at] == first_link(at, links.len() as int),
    ensures
        list_holds(links, 0, (links.len() - 1) as u16, links.len() as u16, 0),
{
    let n = links.len() as int;
    lemma_counting_up(links, 0);
    let list = chain(links, 0, n as nat);
    assert(forall|i: int| 0 <= i < n ==> list[i] == i as u16);
    lemma_count_absent(links, LIVE);
    lemma_count_absent(links, RETIRED);
}

pub(crate) proof fn lemma_take_head(links: Seq<u16>, head: u16, tail: u16, free: u16, live: u16)
    requires
        list_holds(links, head, tail, free, live),
        (head as int) < links.len(),
    ensures
        free >= 1,
        live < 1024,
        list_holds(
            links.update(head as int, LIVE),
            links[head as int],
            if links[head as int] == END {
                END
            } else {
                tail
            },
            (free - 1) as u16,
            (live + 1) as u16,
        ),
{
    let n = links.len() as int;
    let list = chain(links, head, free as nat);
    let next = links[head as int];
    let updated = links.update(head as int, LIVE);
    lemma_count_update(links, head as int, LIVE, LIVE);
    lemma_count_update(links, head as int, LIVE, RETIRED);
    let rest = chain(links, next, (free - 1) as nat);
    lemma_chain_len(links, next, (free - 1) as nat);
    assert(list[0] == head);
    assert forall|i: int| 0 <= i < free - 1 implies rest[i] == list[i + 1] && rest[i] != head by {
        assert(list[i + 1] != list[0]);
    }
    lemma_chain_agree(links, updated, next, (free - 1) as nat);
    assert(chain(updated, next, (free - 1) as nat) == rest);
    assert forall|i: int|
        #![trigger rest[i]]
        0 <= i < free - 1 implies rest[i] < n && free_link(updated[rest[i] as int], n) by {
        assert(list[i + 1] < n && free_link(links[list[i + 1] as int], n));
    }
    assert(rest.no_duplicates()) by {
        assert forall|i: int, j: int|
            0 <= i < free - 1 && 0 <= j < free - 1 && i != j implies rest[i] != rest[j] by {
            assert(list[i + 1] != list[j + 1]);
        }
    }
    if free > 1 {
        assert(rest[0] == next);
        assert(tail == rest[free - 2]);
        assert(tail != head);
    }
}

pub(crate) proof fn lemma_put(
    links: Seq<u16>,
    head: u16,
    tail: u16,
    free: u16,
    live: u16,
    slot: u16,
)
    requires
        list_holds(links, head, tail, free, live),
        (slot as int) < links.len(),
        links[slot as int] == LIVE,
    ensures
        live >= 1,
        free < 1024,
        ((tail as int) < links.len()) == (free > 0),
        list_holds(
            if free > 0 {
                links.update(slot as int, END).update(tail as int, slot)
            } else {
                links.update(slot as int, END)
            },
            if free > 0 {
                head
            } else {
                slot
            },
            slot,
            (free + 1) as u16,
            (live - 1) as u16,
        ),
{
    let n = links.len() as int;
    let list = chain(links, head, free as nat);
    let cleared = links.update(slot as int, END);
    lemma_count_update(links, slot as int, END, LIVE);
    lemma_count_update(links, slot as int, END, RETIRED);
    assert forall|i: int| 0 <= i < free implies list[i] != slot by {
        assert(free_link(links[list[i] as int], n));
    }
    if free == 0 {
        reveal_with_fuel(chain, 2);
        assert(chain(cleared, slot, 1) =~= seq![slot]);
    } else {
        let appended = cleared.update(tail as int, slot);
        lemma_count_update(cleared, tail as int, slot, LIVE);
        lemma_count_update(cleared, tail as int, slot, RETIRED);
        assert forall|i: int|
            #![trigger list[i]]
            0 <= i < free - 1 implies links[list[i] as int] == appended[list[i] as int] by {
            assert(list[i] != list[free - 1]);
        }
        lemma_chain_agree(links, appended, head, free as nat);
        lemma_chain_push(appended, head, free as nat);
        let grown = chain(appended, head, (free + 1) as nat);
        assert(grown =~= list.push(slot));
        assert forall|i: int|
            #![trigger grown[i]]
            0 <= i < free + 1 implies grown[i] < n && free_link(appended[grown[i] as int], n) by {
            if i < free {
                assert(grown[i] == list[i]);
                assert(free_link(links[list[i] as int], n));
            }
        }
        assert(grown.no_duplicates());
    }
}

pub(crate) proof fn lemma_retire(
    links: Seq<u16>,
    head: u16,
    tail: u16,
    free: u16,
    live: u16,
    slot: u16,
)
    requires
        list_holds(links, head, tail, free, live),
        (slot as int) < links.len(),
        links[slot as int] == LIVE,
    ensures
        live >= 1,
        list_holds(links.update(slot as int, RETIRED), head, tail, free, (live - 1) as u16),
{
    let n = links.len() as int;
    let list = chain(links, head, free as nat);
    let retired = links.update(slot as int, RETIRED);
    lemma_count_update(links, slot as int, RETIRED, LIVE);
    lemma_count_update(links, slot as int, RETIRED, RETIRED);
    assert forall|i: int| 0 <= i < free implies list[i] != slot by {
        assert(free_link(links[list[i] as int], n));
    }
    lemma_chain_agree(links, retired, head, free as nat);
}

} // verus!
