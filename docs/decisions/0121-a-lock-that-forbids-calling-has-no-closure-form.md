# D-121 A lock that forbids calling has no closure form

Status: Accepted (2026-08-24)

**Context.** D-105 split the lock API in two. `lock(&mut ThreadContext)` takes
a context, and the borrow checker then refuses a second acquisition while the
guard lives. `lock_leaf()` takes nothing, which is what makes it usable from a
trap and from `Drop`, and it buys that by moving the obligation onto the
caller: **nothing is called while the guard is live.** A leaf lock is the last
one in the order because nothing under it can reach another.

Three statics offered a closure over their contents — `with_memory_map`,
`with_io_apic`, and the `with_active_space` shape they were copied from. Each
took its leaf guard, handed the caller a reference, and ran the caller's code.
A closure *is* a call. The obligation was on the caller, nothing checked it,
and the audit found a caller walking page tables under `MAP`'s guard: a second
structure, with its own reachable locks, entered with the map's held.

The reference the D-105 sweep had, and did not act on: this is the shape of the
callback-with-lock-held bug that Rust's own `RefCell` guidance names, and the
reason Linux's `->readdir` was reworked into `iterate_shared` — a caller-
supplied function under a lock is an unbounded call, and its bound cannot be
read from the site that takes the lock.

**Options.**

1. **Document the obligation harder.** Free, and exactly what was already
   there. The comment said it and a caller violated it, which is the whole
   argument against.
2. **Rank the leaf locks.** A type-level order under `lock_leaf` would catch a
   *lock* taken under one. It would not catch the page-table walk, which takes
   no lock — it dereferences, and a fault there is taken with the guard held.
3. **Delete the closure forms.** Replace each with queries that take the lock,
   read what was asked, and give it back — a number, an `Option`, or a copy.

**Decision.** Option 3. `with_memory_map` became `map_covers_memory`,
`first_region`, `past_every_region` and `direct_mapped_extents`;
`with_io_apic` became `routing_of`. Where a predicate is genuinely needed the
argument is a **`fn` pointer, not a closure**: it cannot capture, so it cannot
reach a lock the caller happened to be holding, and the whole of what it can do
is readable at its definition.

Where the answer is a structure rather than a number, it is **copied out and
the guard dropped** before the caller touches it. `direct_mapped_extents`
returns sixty-four extents by value — a page and a half of stack, which the one
caller's boot-time walk has room for — and reports the true count separately,
so a map with more stretches than fit makes the caller refuse rather than
silently check part of memory.

**Rationale.** The obligation stops being an obligation. A query takes the lock
and returns; there is no window in which caller code runs with the guard live,
so there is nothing for a caller to get wrong. That is the same move D-105 made
one level up — replacing a rule a reviewer must apply with a shape the compiler
or the API enforces — and it is worth more here, because the failure mode is a
deadlock or a fault-with-guard-held rather than a wrong answer.

The cost is real and small: four calls where there was one, four lock
acquisitions instead of one, at boot, on an uncontended lock.

**Consequences.**

- Three closure-taking accessors are gone. Nothing in the tree hands a caller a
  reference under a leaf guard.
- `MAX_EXTENTS = 64` is a new ceiling, and a caller past it refuses. That is a
  bound where there was an unbounded loop under a lock.
- `first_region` takes `fn(&MemoryRegion) -> bool`. The non-capturing
  restriction is the point, not an inconvenience: a `move` closure over a guard
  is precisely the thing being prevented, and the type refuses it.
- The rule generalises and is written here so the next static does not have to
  rediscover it: **a `lock_leaf` static may not expose a closure form.** If a
  caller needs to do work with the contents, the contents are copied out.

**Left owed.** The rule is not mechanically enforced — a fourth static could
grow a `with_` method tomorrow and nothing would say so. A lint over `wost` and
`kernel` for a `pub fn` taking `impl FnOnce` in a module that also calls
`lock_leaf` would catch it, and is not worth writing until there is a fourth.
Recorded so the next one is a decision rather than an oversight.
