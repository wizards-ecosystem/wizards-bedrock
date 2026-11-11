# D-110 Channels carry bytes and handles, and both ends declare the rights

Status: Accepted design, 2026-08-24. Speculative implementation retired by D-248.

**Decision: the object.** `object::channel()` makes two ends together, each
holding a **weak** reference to the other. A send appends to the *peer's* inbox;
a receive takes from its own. Two strong references would form a cycle `Arc`
will not collect. The price of weak is that an orphaned end must say so, with
`PeerClosed` instead of a message the caller believes was delivered. Queued
messages survive the far end's death, because closing a peer does not reach into
this end's inbox.

The inbox is bounded at 16, and a full one returns `Full`. That is backpressure,
not a promise: nothing is silently dropped, which is what io_uring's `CQ_NODROP`
exists to avoid, and it is the same answer D-79 gave the completion queue.

**Decision: the rights.** Handles travel as `(object, rights)`, never as table
values, because a handle value means nothing outside the table that minted it
(D-78). The discipline lives at the table boundary, in two symmetric calls.

`HandleTable::send(handle, declared)` is a **move** with three refusals:

- **`TRANSFER` is required**, so stripping that one bit yields a handle its
  holder may use and may not pass on. It is the cheapest confinement this model
  has, and it costs a bit.
- **`declared` must be a subset of what is held**, so attenuation only narrows.
- **`declared` must mean something for the object type**, checking D-78's rule
  where it is easiest to violate.

The refusal is at the **sender**. A receiver would need to know what the sender
held, and the message carries an object and rights, not a history.

`HandleTable::accept(object, offered, declared)` is the other half, and it is
why **both ends declare** (managarm's `PushDescriptor`/`PullDescriptor`). The
receiver states what it will hold and gets the narrower of the two, so neither
end can surprise the other. A receiver asking for more than was offered is
**refused, not clamped**. A clamp is a default by another name, and Fuchsia
RFC-0028 found such defaults unclear and inconsistent across object types. There
is no `send_with_all_rights`.

D-151 amended the provenance clause when completions became a handle-producing
path.

**Rationale.** `002` named `Channel` as the IPC primitive, and Pillar 1 then
said a process received one bootstrap handle and every later handle through a
channel, so the move and both declarations were where authority crossed.
D-151 later amended that provenance clause.

**Consequences.** `wait_for_message` waits with the message already queued, so
a registration that does not re-read blocks for good and the check fails as an
observer timeout, by D-105's second rule.

Left owed: the per-message allocation is charged to no quota (D-44). A channel
end has no `Drop` that wakes its peer's waiters, so an end dropped while a
thread blocks on the other side is noticed only at the next wake. Its check is
owed with reclamation.
