# D-110 Channels carry bytes and handles, and both ends declare the rights

Status: Accepted design, 2026-08-24. Speculative implementation retired by D-248.

**Context.** `002` named `Channel` as the IPC primitive and, at this decision,
pillar 1 said a process received one bootstrap handle and every later handle
through a channel. Slice 6 built the handle table and the object header;
nothing could move a handle between tables. D-151 later amended the provenance
clause when completions became a handle-producing path.

**Decision — the object.** Two ends, made together by `object::channel()`, each
holding a **weak** reference to the other. A send appends to the *peer's*
inbox; a receive takes from its own. Weak rather than strong because two ends
holding each other strongly is a cycle `Arc` will not collect, and the price of
weak is that an orphaned end must say so — `PeerClosed` rather than a message a
caller believes was delivered. Messages already queued survive the far end's
death, because closing a peer does not reach into this end's inbox.

The inbox is bounded at 16 and a full one returns `Full`. Backpressure, not a
promise: nothing is silently dropped, which is what io_uring's `CQ_NODROP`
exists to avoid and the same answer D-79 gave the completion queue.

**Decision — the rights.** Handles travel as `(object, rights)`, never as table
values: a handle value means nothing outside the table that minted it (D-78).
The discipline lives at the table boundary, in two calls that are deliberately
symmetric.

`HandleTable::send(handle, declared)` is a **move**. Three refusals, each a
separate claim:

- **`TRANSFER` is required**, so stripping that one bit yields a handle its
  holder may use and may not pass on. That is the cheapest confinement this
  model has, and it costs a bit.
- **`declared` must be a subset of what is held** — attenuation only narrows.
- **`declared` must mean something for the object type**, which is D-78's rule
  checked where it is easiest to violate.

The refusal is at the **sender**. A receiver policing it would need to know
what the sender held, and the message carries an object and a set of rights,
not a history.

`HandleTable::accept(object, offered, declared)` is the other half, and it is
why **both ends declare** (managarm's `PushDescriptor`/`PullDescriptor`): the
receiver states what it is willing to hold and gets the narrower of the two, so
neither end can be surprised by the other. A receiver asking for more than was
offered is **refused, not clamped** — a clamp is a default by another name, and
Fuchsia RFC-0028's finding is that defaults here have unclear behaviour that is
inconsistent across object types. There is no `send_with_all_rights`.

**Consequences.** Six checks, at 118: the crossing and the bound, the orphaned
end, the two-sided attenuation, the `TRANSFER` refusal, the object identity
across the boundary, and the blocking receive. Four were broken and went red —
`TRANSFER` dropped from the send, the close removed so a send became a copy,
`accept` clamping instead of refusing, and the wait's re-read removed.

**The fourth break is the one worth recording.** Replacing `wait_unless` with a
plain `wait` inside `receive_blocking` left the gate *green*, because
`receive_blocking` takes the message before it ever waits — so it cannot reach
the one state where the two differ. This is D-105's second rule again, and the
fix is the same shape the event's check took: `wait_for_message` drives the wait
directly with the message already queued, so a registration that does not
re-read blocks on a channel nobody will send to again. Written on a spawned
thread, so the failure is a timeout in the observer and not a hang.

Left owed: nothing yet gives a process its bootstrap channel — that is the
handle-at-spawn half of slice 7, and it needs a `Process` to hand it to. The
per-message allocation is charged to no quota (D-44). And a channel end has no
`Drop` that wakes its peer's waiters, so an end dropped while a thread is
blocked on the other side is only noticed at the next wake; the check that
would catch it is the one owed with reclamation.
