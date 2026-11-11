# D-127 A channel end that dies says so

Status: Accepted design, 2026-08-25. Speculative implementation retired by D-248.

**Decision.** `Drop for Channel`: the end that dies wakes the other, wherever
it dies. `Drop` reaches the peer through `SpinLock::get_mut` rather than a guard
and calls `wake_all` on it.

`get_mut` is load-bearing, not tidiness. A `Drop` runs wherever the last
reference dies (inside `reclaim`, inside a handle table's teardown, at the end
of a `send` that failed), so a lock taken here is a lock taken under whatever
that caller happened to be holding, which is what D-121 forbids and what
D-126's check now reports. A `&mut self` needs no lock, so there is none to
nest.

`object::channel` refuses rather than returning an unlinked pair: a constructor
which cannot establish its invariant returns `None`.

**Rationale.** A receiver that blocked while its peer was alive, and whose peer
then died, waited on a queue nothing would ever wake: `wait_unless` re-reads
`is_orphaned` only when woken, and `Channel` had no `Drop`. Signalling the peer
from every path that can drop an end is where this kind of bug is
manufactured: a property that must be restated at every site holds until the
next site, and `Drop` is the one place the language guarantees runs on every
path. Polling would turn a tickless wait into a timer for a condition that
changes once, the arrangement D-57 removed.

The recursion terminates at depth two: reaching the peer takes a strong
reference, releasing it can destroy the peer in turn, and that peer's own drop
finds this end's strong count already at zero and upgrades nothing.

**Consequences.**

- `Channel` has a `Drop`, so its fields can no longer be moved out.
- `a blocked receiver learns its peer is gone` blocks a spawned thread in
  `receive_blocking`, waits until it is off the run queue, then drops the last
  strong reference to the other end. **Nothing is ever sent**, so the wake under
  test is the only thing that can end the wait, which makes it red rather than
  slow when the wake is absent. The old `an orphaned channel end refuses`
  polled the predicate, not the wake.

**Left owed.** `Event` has the same shape and does not need it yet: an event is
signalled rather than closed, and nothing waits on one whose last reference can
go while a waiter holds a reference to it. The day an object's death is a
meaningful answer to a wait, it needs this treatment too, and the reasoning
should be here rather than rediscovered.
