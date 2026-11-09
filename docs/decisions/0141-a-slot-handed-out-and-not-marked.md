# D-141 A slot handed out and not marked

Status: Accepted, 2026-08-25.

**Context.** `HandleTable::insert` took the arena's leaf guard **twice**:

```rust
let index = loop {
    reserve_a_slot()?;                      // allocates, so outside the lock
    if let Some(index) = ARENA.lock_leaf().take() { break index; }
};
let mut arena = ARENA.lock_leaf();          // ...and again
arena.slots[index].entry = Some(entry);
```

`take` scanned for `entry.is_none()`, advanced the hint, and returned the
index — **without marking the slot**. So between the two acquisitions the slot
was free by every test the arena can make, and the guard had been dropped, which
restores the interrupt flag.

Two callers interleaving there both receive the same index. The second write
**overwrites a live entry**, and the displaced `Entry` is dropped in place,
under the leaf guard. An `ObjectRef` dropped there can reach `Channel::drop` →
`WaitQueue::wake_all` → `THREADS.lock_leaf()`: the lock-under-a-leaf class
D-121, D-126 and D-128 exist for, and one D-126's runtime detector sees only on
a boot where the race actually fires.

The loser also loses its authority — its handle resolves `NotYours`. That half
fails closed, which is why it would have been a mystery rather than a breach.

**Reachable today: no.** Spawn is single-threaded and no syscall mints a
handle. Live the moment either changes.

**Decision.** `take` becomes `claim(entry)`: it finds the slot **and installs
the entry**, under the one guard that found it. `find_free` is the private
scan. `insert` builds the `Entry` inside the retry loop — `ObjectRef` is an
`Arc`, so the clone is a refcount — and the second acquisition is gone.

The reservation stays outside the lock, because that is the other half of the
same rule: growing allocates, and allocating under this guard is the
`ARENA → HEAP` order D-128 removed.

**Rationale.** The window existed because two things had to be true at once —
"the room is reserved" and "the slot is mine" — and they were established under
separate guards with nothing carrying the first claim across. That is the same
shape as D-138's port stranding: a resource taken in one step and committed in
another, with no way to hold the gap. Where D-138 could not close it at the
call site and had to move the question into the registry, here the fix is
simply to do both under one guard, because nothing between them allocates.

**What the check does and does not establish, said plainly.** It fills a table
past a grow, and requires every handle to be distinct, to still resolve to what
it named, and the live count to match. That is the invariant an overwrite
breaks. It **does not reproduce the interleaving**, and nothing here can: it
needs two threads inside a window between two lock acquisitions, and a check
that spawned racers to try would be flaky — which is worse than a narrower
check honestly described. Breaking `claim` so it hands out a slot it never
fills turns it red, along with two other handle checks.

**Consequences.**

- `Arena::claim` replaces `Arena::take`; `insert` takes the guard once. 143
  checks.
- The retry loop clones an `Arc` per attempt. Attempts past the first are the
  case where another table consumed the reserved room, which D-105 already
  records as expected and rare.

**Left owed.** The interleaving is unreproduced. If a handle-minting syscall or
a second CPU arrives, the honest way to test this class is a stress check with
a bounded flake budget, and that is a decision for whoever adds either.

`mem::with_active_space` still hands safe code `&mut` the kernel's own
`AddressSpace`, whose `Reach::Everywhere` disables both of D-105's guards — so
a caller could reclaim the kernel's tables or unmap the stack it is standing
on. No caller does. Same family as this entry, not closed.
