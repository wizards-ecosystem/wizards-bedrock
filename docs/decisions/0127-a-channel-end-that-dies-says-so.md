# D-127 A channel end that dies says so

Status: Accepted design, 2026-08-25. Speculative implementation retired by D-248.

**Context.** `Channel::receive_blocking` documents two ways to return: with a
message, or `None` when "the far end closed with nothing left to read". The
second was unreachable for the only caller that could need it.

The loop is receive, test `is_orphaned`, wait. `WaitQueue::wait_unless`
registers, re-reads its condition once, and blocks — and a blocked thread
re-reads only when something wakes it. So `is_orphaned` is evaluated on the way
*into* the wait and never again. A receiver that blocks while its peer is
alive, and whose peer then dies, is waiting on a condition that has become true
and on a queue nothing will ever wake. It sleeps for the rest of the boot,
holding its slot.

Nothing woke it because nothing could. `Channel` had no `Drop`. An end is an
`Arc<Object>`; when the last reference goes, the `VecDeque` and the `Weak` are
released and that is all that happens. The peer's `Weak` upgrade starts failing
at that instant, and no code runs to tell anyone.

This is not a shape waiting for a future caller. `user::reclaim` drops a dead
program's handle table in a stated order, and step 2's stated reason is "the
bootstrap channel among them, **so the parent's end learns its peer is gone**".
A parent blocked on that channel learned nothing.

The check that was supposed to cover this — `an orphaned channel end refuses` —
polls `is_orphaned` from the checking thread and never blocks. It is a check
about the *predicate*, not about the wake, and it passes on a kernel with no
wake at all. D-95 found the same distinction on `Event` and stated it: the
state can only be re-read once a block returns.

**Options.**

1. **Poll.** Give a blocked receiver a deadline and let it re-test. Turns a
   tickless wait into a timer, for a condition that changes once.
2. **Signal the peer from every path that can drop an end.** `HandleTable`'s
   `Drop`, `close`, `send`, `reclaim`. Every one of them, for ever, and the
   next path to be written is the one that forgets.
3. **`Drop for Channel`.** The end that dies wakes the other, wherever it dies.

**Decision.** Option 3. `Drop` reaches the peer through `SpinLock::get_mut`
rather than a guard and calls `wake_all` on it.

`get_mut` is load-bearing, not tidiness. A `Drop` runs wherever the last
reference dies — inside `reclaim`, inside a handle table's teardown, at the end
of a `send` that failed — so a lock taken here is a lock taken under whatever
that caller happened to be holding, which is what D-121 forbids and what
D-126's check now reports. A `&mut self` needs no lock, so there is none to
nest.

**Rationale.** Option 2 is where this kind of bug is manufactured: a property
that must be restated at every site is a property that holds until the next
site. D-94 named that pattern and D-126 named it again about D-121 — a rule
applied once, at the moment the code is written, decays silently. `Drop` is the
one place the language guarantees runs on every path, including the ones not
written yet.

Option 1 is worse than it looks. `002` wants a machine whose only outstanding
business is a wait to take no interrupts at all, and a receiver polling for a
peer that will die once is exactly the arrangement D-57 removed.

The recursion terminates at depth two: reaching the peer takes a strong
reference, releasing it can destroy the peer in turn, and that peer's own drop
finds this end's strong count already at zero and upgrades nothing.

**Consequences.**

- `Channel` has a `Drop`, so its fields can no longer be moved out. Nothing
  moves them.
- One machine check, `a blocked receiver learns its peer is gone`: a spawned
  thread blocks in `receive_blocking`, the check waits until it is genuinely
  off the run queue, then drops the last strong reference to the other end.
  **Nothing is ever sent**, so the wake under test is the only thing that can
  end the wait — which is what makes it red rather than slow when the wake is
  absent. Broken and watched go red (D-28): without the `Drop` the thread never
  returns and holds its slot for the rest of the boot.
- `object::channel` refuses rather than returning an unlinked pair. The `if let`
  around the peer linking had no `else`, so a failure would have handed back two
  ends that satisfy the type and violate the invariant — every `send` reporting
  `PeerClosed` for a peer that was never opened, arbitrarily far from the cause.
  Unreachable, and written as a refusal anyway: this kernel's shape is that a
  constructor which cannot establish its invariant returns `None`.

**Left owed.** `Event` has the same shape and does not need it yet: an event is
signalled rather than closed, and nothing waits on one whose last reference can
go while a waiter holds a reference to it. The day an object's death is a
meaningful answer to a wait, it needs this treatment too, and the reasoning
should be here rather than rediscovered.
