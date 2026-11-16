use vstd::prelude::verus;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
use crate::{Action, Extent, SLOTS, Slot};
use crate::{Cpu, CpuSet, Load, Shootdown, Slots};

verus! {

pub open spec fn bit_set(word: u64, bit: u64) -> bool {
    (word >> bit) & 1 == 1
}

impl Cpu {
    #[verifier::type_invariant]
    pub(crate) open spec fn below_max(self) -> bool {
        self.0 < 64
    }

    pub open(crate) spec fn spec_index(self) -> int {
        self.0 as int
    }
}

impl CpuSet {
    pub open(crate) spec fn spec_bits(self) -> u64 {
        self.0
    }

    pub open(crate) spec fn spec_contains(self, cpu: int) -> bool {
        0 <= cpu < 64 && bit_set(self.0, cpu as u64)
    }
}

impl Load {
    #[verifier::type_invariant]
    pub(crate) open spec fn names_a_slot(self) -> bool {
        self.slot < SLOTS
    }

    pub open(crate) spec fn spec_slot(self) -> usize {
        self.slot
    }

    pub open(crate) spec fn full_pcid_spec(self) -> u16 {
        (self.slot + 1) as u16
    }

    pub open(crate) spec fn spec_flushes(self) -> bool {
        self.flushes
    }

    pub open(crate) spec fn spec_evicted(self) -> Option<u64> {
        self.evicted
    }
}

impl Shootdown {
    pub open(crate) spec fn spec_generation(self) -> u64 {
        self.generation
    }

    pub open(crate) spec fn spec_remaining(self) -> CpuSet {
        self.remaining
    }
}

// The flush a CPU performs covers the request, never moves its generation back, and is a range
// flush only for the one generation after the one it had seen.
pub open spec fn catches_up(
    seen: u64,
    request: u64,
    latest: u64,
    extent: Extent,
    action: Action,
    now: u64,
) -> bool {
    &&& now >= seen
    &&& now >= request
    &&& match action {
        Action::Skip => request <= seen && now == seen,
        Action::Range => extent == Extent::Range && now == seen + 1 && now >= latest,
        Action::Whole => now >= latest,
    }
}

// A slot that has not seen the latest generation is loaded only with a flush.
pub open spec fn stale_only_with_flush(seen: Option<u64>, latest: u64, flushes: bool) -> bool {
    flushes == !(seen matches Some(seen) && seen >= latest)
}

// An acknowledgement is taken once, from a CPU still owing one, and removes only that CPU.
pub open(crate) spec fn acknowledged_once(before: Shootdown, cpu: Cpu, after: Option<Shootdown>) -> bool {
    &&& after.is_some() == before.spec_remaining().spec_contains(cpu.spec_index())
    &&& after matches Some(next) ==> next.spec_generation() == before.spec_generation() && forall|other: int|
        #[trigger] next.spec_remaining().spec_contains(other) == (before.spec_remaining().spec_contains(other)
            && other != cpu.spec_index())
}

pub open(crate) spec fn complete_only_when_acknowledged(shootdown: Shootdown, complete: bool) -> bool {
    complete == forall|cpu: int| !#[trigger] shootdown.remaining.spec_contains(cpu)
}

// `states[k + 1]` is `states[k]` acknowledged by `cpus[k]`.
pub open(crate) spec fn acknowledged_in_turn(states: Seq<Shootdown>, cpus: Seq<Cpu>) -> bool {
    &&& states.len() == cpus.len() + 1
    &&& forall|step: int|
        0 <= step < cpus.len() ==> #[trigger] acknowledged_once(
            states[step],
            cpus[step],
            Some(states[step + 1]),
        )
}

// The flush completes only once every target but the requester has acknowledged it.
pub proof fn lemma_flush_completes_only_after_every_target(
    targets: CpuSet,
    requester: Cpu,
    states: Seq<Shootdown>,
    cpus: Seq<Cpu>,
)
    requires
        acknowledged_in_turn(states, cpus),
        forall|cpu: int| #[trigger] states[0].spec_remaining().spec_contains(cpu) == (
            targets.spec_contains(cpu) && cpu != requester.spec_index()),
        complete_only_when_acknowledged(states[cpus.len() as int], true),
    ensures
        forall|cpu: int|
            targets.spec_contains(cpu) && cpu != requester.spec_index() ==> exists|step: int|
                0 <= step < cpus.len() && #[trigger] cpus[step].spec_index() == cpu,
{
    lemma_remaining_after(states, cpus, cpus.len() as int);
    assert forall|cpu: int|
        targets.spec_contains(cpu) && cpu != requester.spec_index() implies exists|step: int|
        0 <= step < cpus.len() && #[trigger] cpus[step].spec_index() == cpu by {
        assert(!states[cpus.len() as int].spec_remaining().spec_contains(cpu));
    }
}

pub proof fn lemma_remaining_after(states: Seq<Shootdown>, cpus: Seq<Cpu>, steps: int)
    requires
        acknowledged_in_turn(states, cpus),
        0 <= steps <= cpus.len(),
    ensures
        forall|cpu: int|
            #[trigger] states[steps].spec_remaining().spec_contains(cpu) == (
            states[0].spec_remaining().spec_contains(cpu) && forall|step: int|
                0 <= step < steps ==> #[trigger] cpus[step].spec_index() != cpu),
    decreases steps,
{
    if steps > 0 {
        lemma_remaining_after(states, cpus, steps - 1);
        let step = steps - 1;
        assert(acknowledged_once(states[step], cpus[step], Some(states[step + 1])));
        assert forall|cpu: int|
            #[trigger] states[steps].spec_remaining().spec_contains(cpu) == (
            states[0].spec_remaining().spec_contains(cpu) && forall|step: int|
                0 <= step < steps ==> #[trigger] cpus[step].spec_index() != cpu) by {
            if states[steps].spec_remaining().spec_contains(cpu) {
                assert forall|step: int| 0 <= step < steps implies #[trigger] cpus[step].spec_index()
                    != cpu by {
                    if step < steps - 1 {
                        assert(states[steps - 1].spec_remaining().spec_contains(cpu));
                    }
                }
            }
        }
    }
}

impl Slots {
    #[verifier::type_invariant]
    pub(crate) open spec fn distinct(self) -> bool {
        self.victim < SLOTS && distinct_entries(self.entries@)
    }

    pub(crate) open spec fn holds(self, index: int, space: u64) -> bool {
        holds_at(self.entries@, index, space)
    }

    pub open(crate) spec fn slot_of(self, space: u64) -> Option<int> {
        slot_in(self.entries@, space)
    }

    // The generation the slot holding `space` has seen, if a slot holds it.
    pub open(crate) spec fn seen_of(self, space: u64) -> Option<u64> {
        seen_in(self.entries@, space)
    }

    pub(crate) proof fn lemma_slot_of(self, index: int, space: u64)
        requires
            self.distinct(),
            self.holds(index, space),
        ensures
            self.slot_of(space) == Some(index),
            self.seen_of(space) == Some(self.entries@[index].unwrap().seen),
    {
        let chosen = choose|other: int| holds_at(self.entries@, other, space);
        assert(chosen == index);
    }

    pub(crate) proof fn lemma_absent(self, space: u64)
        requires
            forall|index: int| 0 <= index < SLOTS ==> !#[trigger] self.holds(index, space),
        ensures
            self.slot_of(space).is_none(),
            self.seen_of(space).is_none(),
    {
        assert forall|index: int| !#[trigger] holds_at(self.entries@, index, space) by {
            if 0 <= index < SLOTS {
                assert(!self.holds(index, space));
            }
        }
    }

    pub(crate) proof fn lemma_replace_keeps_distinct(self, index: int, space: u64)
        requires
            self.distinct(),
            0 <= index < SLOTS,
            forall|other: int| other != index ==> !#[trigger] self.holds(other, space),
        ensures
            forall|seen: u64| #[trigger]
                distinct_entries(self.entries@.update(index, Some(Slot { space, seen }))),
    {
        assert forall|seen: u64| #[trigger]
            distinct_entries(self.entries@.update(index, Some(Slot { space, seen }))) by {
            let after = self.entries@.update(index, Some(Slot { space, seen }));
            assert forall|left: int, right: int, held: u64|
                holds_at(after, left, held) && holds_at(after, right, held) implies left == right by {
                if left != index && right != index {
                    assert(holds_at(self.entries@, left, held) && holds_at(self.entries@, right, held));
                } else if left == index && right != index {
                    assert(self.holds(right, space));
                } else if right == index && left != index {
                    assert(self.holds(left, space));
                }
            }
        }
    }

    pub(crate) proof fn lemma_replaced(self, before: Self, index: int, space: u64)
        requires
            self.distinct(),
            0 <= index < SLOTS,
            before.distinct(),
            self.entries@ == before.entries@.update(index, self.entries@[index]),
            self.entries@[index] matches Some(slot) && slot.space == space,
        ensures
            self.slot_of(space) == Some(index),
            self.seen_of(space) == Some(self.entries@[index].unwrap().seen),
            forall|other: u64|
                other != space && !before.holds(index, other) ==> #[trigger] self.seen_of(other)
                    == before.seen_of(other),
    {
        self.lemma_slot_of(index, space);
        assert forall|other: u64|
            other != space && !before.holds(index, other) implies #[trigger] self.seen_of(other)
            == before.seen_of(other) by {
            assert forall|at: int| #[trigger] holds_at(self.entries@, at, other) == holds_at(
                before.entries@,
                at,
                other,
            ) by {}
            if exists|at: int| holds_at(before.entries@, at, other) {
                let at = choose|at: int| holds_at(before.entries@, at, other);
                before.lemma_slot_of(at, other);
                self.lemma_slot_of(at, other);
            }
        }
    }

    pub(crate) proof fn lemma_evicted(self, after: Self, index: int, space: u64)
        requires
            self.distinct(),
            after.distinct(),
            0 <= index < SLOTS,
            after.entries@ == self.entries@.update(index, after.entries@[index]),
            after.entries@[index] matches Some(slot) && slot.space == space,
            self.entries@[index] matches Some(slot) ==> slot.space != space,
        ensures
            self.entries@[index] matches Some(slot) ==> after.seen_of(slot.space).is_none(),
    {
        if let Some(slot) = self.entries@[index] {
            assert forall|at: int| 0 <= at < SLOTS implies !#[trigger] after.holds(at, slot.space) by {
                if at != index {
                    if after.holds(at, slot.space) {
                        assert(self.holds(at, slot.space) && self.holds(index, slot.space));
                    }
                }
            }
            after.lemma_absent(slot.space);
        }
    }
}

pub(crate) open spec fn slot_in(entries: Seq<Option<Slot>>, space: u64) -> Option<int> {
    if exists|index: int| holds_at(entries, index, space) {
        Some(choose|index: int| holds_at(entries, index, space))
    } else {
        None
    }
}

pub(crate) open spec fn seen_in(entries: Seq<Option<Slot>>, space: u64) -> Option<u64> {
    match slot_in(entries, space) {
        Some(index) => match entries[index] {
            Some(slot) => Some(slot.seen),
            None => None,
        },
        None => None,
    }
}

pub(crate) open spec fn holds_at(entries: Seq<Option<Slot>>, index: int, space: u64) -> bool {
    0 <= index < SLOTS && (entries[index] matches Some(slot) && slot.space == space)
}

// No two slots hold one address space.
pub(crate) open spec fn distinct_entries(entries: Seq<Option<Slot>>) -> bool {
    &&& entries.len() == SLOTS
    &&& forall|left: int, right: int, space: u64|
        #[trigger] holds_at(entries, left, space) && #[trigger] holds_at(entries, right, space)
            ==> left == right
}

pub proof fn lemma_test_bit(word: u64, bit: u64)
    requires
        bit < 64,
    ensures
        (word & (1u64 << bit) != 0) == bit_set(word, bit),
{
    assert((word & (1u64 << bit) != 0) == bit_set(word, bit)) by (bit_vector)
        requires
            bit < 64,
    ;
}

pub proof fn lemma_with_bit(word: u64, set: u64)
    requires
        set < 64,
    ensures
        forall|bit: int|
            0 <= bit < 64 ==> #[trigger] bit_set(word | (1u64 << set), bit as u64) == (bit_set(
                word,
                bit as u64,
            ) || bit == set),
{
    assert forall|bit: int| 0 <= bit < 64 implies #[trigger] bit_set(word | (1u64 << set), bit as u64)
        == (bit_set(word, bit as u64) || bit == set) by {
        let b = bit as u64;
        assert(bit_set(word | (1u64 << set), b) == (bit_set(word, b) || b == set)) by (bit_vector)
            requires
                set < 64,
                b < 64,
        ;
    }
}

pub proof fn lemma_without_bit(word: u64, clear: u64)
    requires
        clear < 64,
    ensures
        forall|bit: int|
            0 <= bit < 64 ==> #[trigger] bit_set(word & !(1u64 << clear), bit as u64) == (bit_set(
                word,
                bit as u64,
            ) && bit != clear),
{
    assert forall|bit: int| 0 <= bit < 64 implies #[trigger] bit_set(
        word & !(1u64 << clear),
        bit as u64,
    ) == (bit_set(word, bit as u64) && bit != clear) by {
        let b = bit as u64;
        assert(bit_set(word & !(1u64 << clear), b) == (bit_set(word, b) && b != clear))
            by (bit_vector)
            requires
                clear < 64,
                b < 64,
        ;
    }
}

pub proof fn lemma_no_bits(word: u64)
    requires
        word == 0,
    ensures
        forall|bit: int| 0 <= bit < 64 ==> !#[trigger] bit_set(word, bit as u64),
{
    assert forall|bit: int| 0 <= bit < 64 implies !#[trigger] bit_set(word, bit as u64) by {
        let b = bit as u64;
        assert(!bit_set(0, b)) by (bit_vector);
    }
}

pub proof fn lemma_some_bit(word: u64) -> (bit: int)
    requires
        word != 0,
    ensures
        0 <= bit < 64,
        bit_set(word, bit as u64),
{
    vstd::std_specs::bits::axiom_u64_trailing_zeros(word);
    word.trailing_zeros() as int
}

} // verus!
