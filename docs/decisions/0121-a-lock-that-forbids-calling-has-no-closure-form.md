# D-121 A lock that forbids calling has no closure form

Status: Accepted, 2026-08-24.

**Decision.** **A `lock_leaf` static may not expose a closure form.** If a
caller needs to do work with the contents, the contents are copied out.
`lock_leaf()` buys its use from a trap and from `Drop` by moving the
obligation onto the caller: **nothing is called while the guard is live**, and
a closure *is* a call. A leaf lock is the last one in the order because nothing
under it can reach another.

`with_memory_map` became `map_covers_memory`, `first_region`,
`past_every_region` and `direct_mapped_extents`; `with_io_apic` became
`routing_of`; the `with_active_space` shape they were copied from is gone too.
Nothing in the tree hands a caller a reference under a leaf guard. Where a
predicate is genuinely needed the argument is a **`fn` pointer, not a
closure**: it cannot capture, so it cannot reach a lock the caller happened to
be holding, and the whole of what it can do is readable at its definition.

Where the answer is a structure rather than a number, it is **copied out and
the guard dropped** before the caller touches it. `direct_mapped_extents`
returns up to `MAX_EXTENTS = 64` extents by value and reports the true count
separately, so a map with more stretches than fit makes the caller refuse
rather than silently check part of memory.

**Rationale.** A query takes the lock and returns; there is no window in which
caller code runs with the guard live, so there is nothing for a caller to get
wrong. A documented obligation was already there and a caller walking page
tables under `MAP`'s guard violated it. A rank over leaf locks would not catch
that walk, which takes no lock and faults with the guard held. A caller-supplied
function under a lock is an unbounded call, and its bound cannot be read from
the site that takes the lock. The cost is four calls where there was one, at
boot, on an uncontended lock.

**Consequences.** `first_region` takes `fn(&MemoryRegion) -> bool`. The
non-capturing restriction is the point, not an inconvenience: a `move` closure
over a guard is precisely the thing being prevented, and the type refuses it.

**Left owed.** The rule is not mechanically enforced: a fourth static could grow
a `with_` method tomorrow and nothing would say so. A lint over `wost` and
`kernel` for a `pub fn` taking `impl FnOnce` in a module that also calls
`lock_leaf` would catch it, and is not worth writing until there is a fourth.
Recorded so the next one is a decision rather than an oversight.
