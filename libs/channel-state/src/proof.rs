use vstd::prelude::verus;
#[cfg(verus_only)]
use vstd::prelude::*;

verus! {

pub open spec fn call_step(state: crate::CallState, event: crate::CallEvent) -> Option<
    crate::CallState,
> {
    match (state, event) {
        (crate::CallState::Idle, crate::CallEvent::Commit) => Some(crate::CallState::Queued),
        (crate::CallState::Queued, crate::CallEvent::Deliver) => Some(crate::CallState::Delivered),
        (crate::CallState::Queued, crate::CallEvent::Close) => Some(crate::CallState::PeerClosed),
        (crate::CallState::Queued, crate::CallEvent::Cancel) => Some(crate::CallState::Cancelled),
        (crate::CallState::Delivered, crate::CallEvent::Cancel) => Some(
            crate::CallState::Cancelled,
        ),
        (crate::CallState::Queued, crate::CallEvent::Timeout) => Some(crate::CallState::TimedOut),
        (crate::CallState::Delivered, crate::CallEvent::Timeout) => Some(
            crate::CallState::TimedOut,
        ),
        (crate::CallState::Delivered, crate::CallEvent::Claim) => Some(crate::CallState::Replying),
        (crate::CallState::Delivered, crate::CallEvent::Abandon) => Some(
            crate::CallState::Abandoned,
        ),
        (crate::CallState::Replying, crate::CallEvent::Complete) => Some(
            crate::CallState::Replied,
        ),
        (_, crate::CallEvent::Consume) => if is_outcome(state) {
            Some(crate::CallState::Idle)
        } else {
            None
        },
        _ => None,
    }
}

pub open spec fn is_outcome(state: crate::CallState) -> bool {
    match state {
        crate::CallState::Replied => true,
        crate::CallState::Abandoned => true,
        crate::CallState::Cancelled => true,
        crate::CallState::TimedOut => true,
        crate::CallState::PeerClosed => true,
        _ => false,
    }
}

pub open spec fn state_code(state: crate::CallState) -> u64 {
    match state {
        crate::CallState::Idle => 0,
        crate::CallState::Queued => 1,
        crate::CallState::Delivered => 2,
        crate::CallState::Replying => 3,
        crate::CallState::Replied => 4,
        crate::CallState::Abandoned => 5,
        crate::CallState::Cancelled => 6,
        crate::CallState::TimedOut => 7,
        crate::CallState::PeerClosed => 8,
    }
}

impl crate::Cell {
    #[verifier::type_invariant]
    pub(crate) open spec fn generation_in_word(self) -> bool {
        self.generation < crate::GENERATION_LIMIT
    }

    pub open(crate) spec fn spec_state(self) -> crate::CallState {
        self.state
    }

    pub open(crate) spec fn spec_generation(self) -> u64 {
        self.generation
    }

    pub open(crate) spec fn spec_bits(self) -> u64 {
        (self.generation << 4u32) | state_code(self.state)
    }
}

pub(crate) proof fn lemma_cell_packs(generation: u64, code: u64)
    requires
        generation < 0x1000_0000_0000_0000,
        code <= 8,
    ensures
        ((generation << 4u32) | code) & 15 == code,
        ((generation << 4u32) | code) >> 4u32 == generation,
{
    assert(((generation << 4u32) | code) & 15 == code && ((generation << 4u32) | code) >> 4u32
        == generation) by (bit_vector)
        requires
            generation < 0x1000_0000_0000_0000,
            code <= 8,
    ;
}

pub(crate) proof fn lemma_cell_unpacks(bits: u64)
    ensures
        bits >> 4u32 < 0x1000_0000_0000_0000,
        ((bits >> 4u32) << 4u32) | (bits & 15) == bits,
{
    assert(bits >> 4u32 < 0x1000_0000_0000_0000 && ((bits >> 4u32) << 4u32) | (bits & 15) == bits)
        by (bit_vector);
}

pub open(crate) spec fn cell_accepts(
    cell: crate::Cell,
    event: crate::CallEvent,
    generation: u64,
) -> bool {
    &&& generation == cell.generation
    &&& call_step(cell.state, event) is Some
    &&& !(event == crate::CallEvent::Consume && cell.generation + 1 >= crate::GENERATION_LIMIT)
}

// A refused event leaves the cell as it was.
pub open(crate) spec fn cell_after(
    cell: crate::Cell,
    event: crate::CallEvent,
    generation: u64,
) -> crate::Cell {
    if cell_accepts(cell, event, generation) {
        crate::Cell {
            state: call_step(cell.state, event)->Some_0,
            generation: if event == crate::CallEvent::Consume {
                (cell.generation + 1) as u64
            } else {
                cell.generation
            },
        }
    } else {
        cell
    }
}

pub open(crate) spec fn run(cell: crate::Cell, events: Seq<(crate::CallEvent, u64)>) -> crate::Cell
    decreases events.len(),
{
    if events.len() == 0 {
        cell
    } else {
        run(cell_after(cell, events[0].0, events[0].1), events.drop_first())
    }
}

pub open(crate) spec fn outcomes_reached(
    cell: crate::Cell,
    events: Seq<(crate::CallEvent, u64)>,
) -> nat
    decreases events.len(),
{
    if events.len() == 0 {
        0
    } else {
        let after = cell_after(cell, events[0].0, events[0].1);
        (if !is_outcome(cell.state) && is_outcome(after.state) {
            1nat
        } else {
            0nat
        }) + outcomes_reached(after, events.drop_first())
    }
}

pub open(crate) spec fn replies_succeeded(
    cell: crate::Cell,
    events: Seq<(crate::CallEvent, u64)>,
) -> nat
    decreases events.len(),
{
    if events.len() == 0 {
        0
    } else {
        let after = cell_after(cell, events[0].0, events[0].1);
        (if events[0].0 == crate::CallEvent::Complete && cell_accepts(
            cell,
            events[0].0,
            events[0].1,
        ) {
            1nat
        } else {
            0nat
        }) + replies_succeeded(after, events.drop_first())
    }
}

pub open(crate) spec fn steps_taken(cell: crate::Cell, events: Seq<(crate::CallEvent, u64)>) -> nat
    decreases events.len(),
{
    if events.len() == 0 {
        0
    } else {
        let after = cell_after(cell, events[0].0, events[0].1);
        (if cell_accepts(cell, events[0].0, events[0].1) {
            1nat
        } else {
            0nat
        }) + steps_taken(after, events.drop_first())
    }
}

pub open(crate) spec fn delivered_on(
    cell: crate::Cell,
    events: Seq<(crate::CallEvent, u64)>,
) -> bool
    decreases events.len(),
{
    cell.state == crate::CallState::Delivered || (events.len() > 0 && delivered_on(
        cell_after(cell, events[0].0, events[0].1),
        events.drop_first(),
    ))
}

// The server has received the call, or the call ended some way other than peer closed.
pub open spec fn received(state: crate::CallState) -> bool {
    state == crate::CallState::Delivered || state == crate::CallState::Replying || (is_outcome(
        state,
    ) && state != crate::CallState::PeerClosed)
}

pub open spec fn rank(state: crate::CallState) -> nat {
    match state {
        crate::CallState::Queued => 3,
        crate::CallState::Delivered => 2,
        crate::CallState::Replying => 1,
        _ => 0,
    }
}

pub open spec fn consumes(events: Seq<(crate::CallEvent, u64)>) -> bool {
    exists|k: int| 0 <= k < events.len() && (#[trigger] events[k]).0 == crate::CallEvent::Consume
}

// Each call has exactly one outcome: from a committed cell, any events short of the caller's
// consume reach at most one outcome, the call keeps its generation, a reply succeeds at most
// once, at most three events take effect, and a call the server received never ends peer closed.
pub(crate) proof fn lemma_call_has_one_outcome(
    cell: crate::Cell,
    events: Seq<(crate::CallEvent, u64)>,
)
    requires
        cell.state != crate::CallState::Idle,
        !consumes(events),
    ensures
        run(cell, events).state != crate::CallState::Idle,
        run(cell, events).generation == cell.generation,
        is_outcome(cell.state) ==> run(cell, events) == cell,
        outcomes_reached(cell, events) == if !is_outcome(cell.state) && is_outcome(
            run(cell, events).state,
        ) {
            1nat
        } else {
            0nat
        },
        replies_succeeded(cell, events) <= outcomes_reached(cell, events),
        steps_taken(cell, events) <= rank(cell.state),
        received(cell.state) ==> run(cell, events).state != crate::CallState::PeerClosed,
        delivered_on(cell, events) ==> run(cell, events).state != crate::CallState::PeerClosed,
    decreases events.len(),
{
    if events.len() > 0 {
        let after = cell_after(cell, events[0].0, events[0].1);
        let rest = events.drop_first();
        assert(events[0].0 != crate::CallEvent::Consume);
        assert(!consumes(rest)) by {
            if consumes(rest) {
                let k = choose|k: int|
                    0 <= k < rest.len() && (#[trigger] rest[k]).0 == crate::CallEvent::Consume;
                assert(events[k + 1] == rest[k]);
            }
        }
        lemma_call_has_one_outcome(after, rest);
    }
}

// From every committed state short of an outcome, some event the caller or reply right sends
// takes the call to an outcome.
pub(crate) proof fn lemma_outcome_reachable(cell: crate::Cell)
    requires
        cell.state != crate::CallState::Idle,
        !is_outcome(cell.state),
    ensures
        exists|event: crate::CallEvent|
            event != crate::CallEvent::Consume && cell_accepts(cell, event, cell.generation)
                && is_outcome(#[trigger] cell_after(cell, event, cell.generation).state),
{
    let event = if cell.state == crate::CallState::Replying {
        crate::CallEvent::Complete
    } else {
        crate::CallEvent::Cancel
    };
    assert(is_outcome(cell_after(cell, event, cell.generation).state));
}

// A reply right names one call: once the cell has moved past a generation, nothing naming it
// takes effect again.
pub(crate) proof fn lemma_old_generation_refused(
    cell: crate::Cell,
    events: Seq<(crate::CallEvent, u64)>,
    generation: u64,
)
    requires
        generation < cell.generation,
    ensures
        run(cell, events).generation >= cell.generation,
        forall|event: crate::CallEvent| !cell_accepts(run(cell, events), event, generation),
    decreases events.len(),
{
    if events.len() > 0 {
        let after = cell_after(cell, events[0].0, events[0].1);
        lemma_old_generation_refused(after, events.drop_first(), generation);
    }
}

impl crate::Descriptor {
    #[verifier::type_invariant]
    pub(crate) open spec fn within_rings(self) -> bool {
        descriptor_fits(self)
    }

    pub open(crate) spec fn spec_bytes_at(self) -> u16 {
        self.bytes_at
    }

    pub open(crate) spec fn spec_bytes(self) -> u16 {
        self.bytes
    }

    pub open(crate) spec fn spec_handles_at(self) -> u16 {
        self.handles_at
    }

    pub open(crate) spec fn spec_handles(self) -> u16 {
        self.handles
    }
}

pub open spec fn descriptor_fits(descriptor: crate::Descriptor) -> bool {
    &&& descriptor.spec_bytes_at() < crate::BYTES
    &&& descriptor.spec_bytes() <= crate::MESSAGE_BYTES
    &&& descriptor.spec_handles_at() < crate::HANDLES
    &&& descriptor.spec_handles() <= crate::MESSAGE_HANDLES
}

pub open spec fn spans_cover(spans: [crate::Span; 2], at: u16, len: u16, capacity: u16) -> bool {
    &&& spans[0].at == at
    &&& spans[1].at == 0
    &&& spans[0].len + spans[1].len == len
    &&& at + spans[0].len <= capacity
    &&& spans[1].len > 0 ==> at + spans[0].len == capacity
}

pub open spec fn bytes_before(messages: Seq<crate::Descriptor>, k: int) -> int
    decreases k,
{
    if k <= 0 {
        0
    } else {
        bytes_before(messages, k - 1) + messages[k - 1].spec_bytes()
    }
}

pub open spec fn handles_before(messages: Seq<crate::Descriptor>, k: int) -> int
    decreases k,
{
    if k <= 0 {
        0
    } else {
        handles_before(messages, k - 1) + messages[k - 1].spec_handles()
    }
}

impl crate::Inbox {
    #[verifier::type_invariant]
    pub(crate) open spec fn rings_tile(self) -> bool {
        &&& self.count <= 16
        &&& self.head < 16
        &&& self.bytes_head < crate::BYTES
        &&& self.handles_head < crate::HANDLES
        &&& self.bytes_used <= crate::BYTES
        &&& self.handles_used <= crate::HANDLES
        &&& self.bytes_used == bytes_before(self.spec_messages(), self.count as int)
        &&& self.handles_used == handles_before(self.spec_messages(), self.count as int)
        &&& forall|k: int| 0 <= k < self.count ==> tiled(self, k)
    }

    pub open(crate) spec fn spec_messages(self) -> Seq<crate::Descriptor> {
        Seq::new(self.count as nat, |k: int| self.descriptors@[(self.head + k) % 16])
    }

    pub open(crate) spec fn get_spec(self, position: int) -> Option<crate::Entry> {
        if 0 <= position < self.count {
            Some(
                crate::Entry {
                    index: slot_of(self, position),
                    descriptor: self.spec_messages()[position],
                },
            )
        } else {
            None
        }
    }
}

pub open(crate) spec fn slot_of(inbox: crate::Inbox, position: int) -> u8 {
    ((inbox.head + position) % 16) as u8
}

pub open(crate) spec fn tiled(inbox: crate::Inbox, k: int) -> bool {
    let messages = inbox.spec_messages();
    &&& descriptor_fits(messages[k])
    &&& messages[k].bytes_at == (inbox.bytes_head + bytes_before(messages, k)) % 2304
    &&& messages[k].handles_at == (inbox.handles_head + handles_before(messages, k)) % 32
}

pub open(crate) spec fn inbox_admits(inbox: crate::Inbox, bytes: u16, handles: u16) -> bool {
    &&& bytes <= crate::MESSAGE_BYTES
    &&& handles <= crate::MESSAGE_HANDLES
    &&& inbox.count < 16
    &&& inbox.bytes_used + bytes <= crate::BYTES
    &&& inbox.handles_used + handles <= crate::HANDLES
}

pub open(crate) spec fn handles_in_ring(inbox: crate::Inbox, position: int) -> bool {
    handles_before(inbox.spec_messages(), position) + inbox.spec_messages()[position].spec_handles()
        <= inbox.handles_used
}

pub open spec fn holds_handle(messages: Seq<crate::Descriptor>, k: int, offset: int) -> bool {
    handles_before(messages, k) <= offset < handles_before(messages, k) + messages[k].spec_handles()
}

// A handle reference in the inbox ring is in exactly one slot: the handle ring's used run,
// `offset` slots past its head, belongs to exactly one queued message.
pub proof fn lemma_handle_in_one_message(messages: Seq<crate::Descriptor>, offset: int)
    requires
        0 <= offset < handles_before(messages, messages.len() as int),
    ensures
        exists|k: int| 0 <= k < messages.len() && holds_handle(messages, k, offset),
        forall|k1: int, k2: int|
            0 <= k1 < messages.len() && 0 <= k2 < messages.len() && holds_handle(
                messages,
                k1,
                offset,
            ) && holds_handle(messages, k2, offset) ==> k1 == k2,
{
    lemma_handle_owner_exists(messages, messages.len() as int, offset);
    assert forall|k1: int, k2: int|
        0 <= k1 < messages.len() && 0 <= k2 < messages.len() && holds_handle(messages, k1, offset)
            && holds_handle(messages, k2, offset) implies k1 == k2 by {
        if k1 < k2 {
            lemma_handles_before_monotone(messages, k1 + 1, k2);
        } else if k2 < k1 {
            lemma_handles_before_monotone(messages, k2 + 1, k1);
        }
    }
}

proof fn lemma_handle_owner_exists(messages: Seq<crate::Descriptor>, k: int, offset: int)
    requires
        0 <= k <= messages.len(),
        0 <= offset < handles_before(messages, k),
    ensures
        exists|j: int| 0 <= j < k && holds_handle(messages, j, offset),
    decreases k,
{
    if offset < handles_before(messages, k - 1) {
        lemma_handle_owner_exists(messages, k - 1, offset);
    } else {
        assert(holds_handle(messages, k - 1, offset));
    }
}

pub(crate) proof fn lemma_handles_before_monotone(messages: Seq<crate::Descriptor>, a: int, b: int)
    requires
        0 <= a <= b,
    ensures
        handles_before(messages, a) <= handles_before(messages, b),
        bytes_before(messages, a) <= bytes_before(messages, b),
    decreases b - a,
{
    if a < b {
        lemma_handles_before_monotone(messages, a, b - 1);
    }
}

pub(crate) proof fn lemma_handles_before_bounded(messages: Seq<crate::Descriptor>, position: int)
    requires
        0 <= position < messages.len(),
    ensures
        handles_before(messages, position) + messages[position].spec_handles() <= handles_before(
            messages,
            messages.len() as int,
        ),
{
    lemma_handles_before_monotone(messages, position + 1, messages.len() as int);
}

proof fn lemma_before_prefix(
    messages: Seq<crate::Descriptor>,
    descriptor: crate::Descriptor,
    k: int,
)
    requires
        0 <= k <= messages.len(),
    ensures
        bytes_before(messages.push(descriptor), k) == bytes_before(messages, k),
        handles_before(messages.push(descriptor), k) == handles_before(messages, k),
    decreases k,
{
    if k > 0 {
        lemma_before_prefix(messages, descriptor, k - 1);
    }
}

pub(crate) proof fn lemma_used_after_push(
    messages: Seq<crate::Descriptor>,
    descriptor: crate::Descriptor,
)
    ensures
        forall|k: int|
            0 <= k <= messages.len() ==> #[trigger] bytes_before(messages.push(descriptor), k)
                == bytes_before(messages, k) && handles_before(messages.push(descriptor), k)
                == handles_before(messages, k),
        bytes_before(messages.push(descriptor), messages.len() + 1int) == bytes_before(
            messages,
            messages.len() as int,
        ) + descriptor.spec_bytes(),
        handles_before(messages.push(descriptor), messages.len() + 1int) == handles_before(
            messages,
            messages.len() as int,
        ) + descriptor.spec_handles(),
{
    assert forall|k: int| 0 <= k <= messages.len() implies #[trigger] bytes_before(
        messages.push(descriptor),
        k,
    ) == bytes_before(messages, k) && handles_before(messages.push(descriptor), k)
        == handles_before(messages, k) by {
        lemma_before_prefix(messages, descriptor, k);
    }
    lemma_before_prefix(messages, descriptor, messages.len() as int);
}

proof fn lemma_before_drop_first(messages: Seq<crate::Descriptor>, k: int)
    requires
        0 <= k < messages.len(),
    ensures
        bytes_before(messages, k + 1) == messages[0].spec_bytes() + bytes_before(
            messages.drop_first(),
            k,
        ),
        handles_before(messages, k + 1) == messages[0].spec_handles() + handles_before(
            messages.drop_first(),
            k,
        ),
    decreases k,
{
    assert(bytes_before(messages, 0) == 0 && handles_before(messages, 0) == 0);
    assert(bytes_before(messages.drop_first(), 0) == 0);
    assert(handles_before(messages.drop_first(), 0) == 0);
    if k > 0 {
        lemma_before_drop_first(messages, k - 1);
        assert(messages.drop_first()[k - 1] == messages[k]);
    }
}

pub(crate) proof fn lemma_used_after_pop(messages: Seq<crate::Descriptor>)
    requires
        messages.len() > 0,
    ensures
        forall|k: int|
            1 <= k <= messages.len() ==> #[trigger] bytes_before(messages, k)
                == messages[0].spec_bytes() + bytes_before(messages.drop_first(), k - 1)
                && handles_before(messages, k) == messages[0].spec_handles() + handles_before(
                messages.drop_first(),
                k - 1,
            ),
        bytes_before(messages, messages.len() as int) >= messages[0].spec_bytes(),
        handles_before(messages, messages.len() as int) >= messages[0].spec_handles(),
{
    assert forall|k: int| 1 <= k <= messages.len() implies #[trigger] bytes_before(messages, k)
        == messages[0].spec_bytes() + bytes_before(messages.drop_first(), k - 1) && handles_before(
        messages,
        k,
    ) == messages[0].spec_handles() + handles_before(messages.drop_first(), k - 1) by {
        lemma_before_drop_first(messages, k - 1);
    }
    lemma_before_drop_first(messages, messages.len() - 1);
    lemma_handles_before_monotone(messages.drop_first(), 0, messages.len() - 1);
}

pub(crate) proof fn lemma_mod_shift(x: int, y: int, m: int)
    requires
        m > 0,
    ensures
        ((x % m) + y) % m == (x + y) % m,
{
    vstd::arithmetic::div_mod::lemma_add_mod_noop(x, y, m);
    vstd::arithmetic::div_mod::lemma_add_mod_noop(x % m, y, m);
    vstd::arithmetic::div_mod::lemma_mod_twice(x, m);
}

} // verus!
