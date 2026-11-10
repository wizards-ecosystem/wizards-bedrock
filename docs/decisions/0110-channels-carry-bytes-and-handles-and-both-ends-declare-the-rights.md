# D-110 Channels carry bytes and handles, and both ends declare the rights

Status: Accepted design, 2026-08-24. Speculative implementation retired by D-248.

**Context.** `002` named `Channel` as the IPC primitive. Pillar 1 then said a
process received one bootstrap handle and every later handle through a channel.
Slice 6 built the handle table and object header; nothing could move a handle
between tables. D-151 later amended the provenance clause when completions
became a handle-producing path.

**Decision: the object.** `object::channel()` makes two ends together, each
holding a **weak** reference to the other. A send appends to the *peer's* inbox;
a receive takes from its own. Two strong references would form a cycle `Arc`
will not collect. The price of weak is that an orphaned end must say so, with
`PeerClosed` instead of a message the caller believes was delivered. Queued
messages survive the far end's death, because closing a peer does not reach into
this end's inbox.

The inbox is bounded at 16, and a full one returns `Full`. Nothing is silently
dropped: this is the backpressure io_uring's `CQ_NODROP` provides, and the
answer D-79 gave the completion queue.

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

**Consequences.** Six checks, at 118: the crossing and the bound, the orphaned
end, the two-sided attenuation, the `TRANSFER` refusal, object identity across
the boundary, and the blocking receive. Four breaks went red: `TRANSFER` dropped
from the send, the close removed so a send became a copy, `accept` clamping
instead of refusing, and the wait's re-read removed.

**The fourth break.** Replacing `wait_unless` with a plain `wait` inside
`receive_blocking` left the gate *green*. `receive_blocking` takes the message
before it waits, so it never reaches the one state where the two differ. This is
D-105's second rule again, and the fix follows the event's check:
`wait_for_message` waits with the message already queued, so a registration that
does not re-read blocks for good. It runs on a spawned thread, so the failure is
an observer timeout, not a hang.

Left owed: nothing yet gives a process its bootstrap channel. That is the
handle-at-spawn half of slice 7, and it needs a `Process`. The per-message
allocation is charged to no quota (D-44). A channel end has no `Drop` that wakes
its peer's waiters, so an end dropped while a thread blocks on the other side is
noticed only at the next wake. Its check is owed with reclamation.
