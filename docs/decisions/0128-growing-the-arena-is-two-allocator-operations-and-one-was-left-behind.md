# D-128 Growing the arena is two allocator operations, and one was left behind

Status: Accepted, 2026-08-25.

**Decision.** `reserve_a_slot` frees the discarded vector after the guard, as
it already allocated the new one before it. The `if` is an expression yielding
whichever vector this call is left holding (the arena's old buffer where the
handover happened, the unused room where another caller got there first), and
the guard is dropped explicitly before it. The explicit `drop(arena)` is
required, not decoration: the discarded vector is declared after the guard, so
end-of-scope order would release it first, which is the bug.

**Rationale.** Allocating and freeing are two calls into the allocator, and
`arena.slots = grown` freed the old buffer under `ARENA`'s guard, rebuilding the
`ARENA` → `HEAP` order D-85 deleted in the one line meant to be the safe half.
Copying entries out and back would replace a bounded `dealloc` with an unbounded
copy and still move the drop somewhere; keeping the old buffer is a leak that
grows with the arena. A comment that states a property is not the property
(D-124), and the lock was taken by a `Drop`, which is invisible at the point
that triggers it: the strongest argument yet for D-126's counter over a lint.

**Consequences.**

- `the handle arena grows more than once` holds forty handles in one table,
  walking the arena's capacity through 16, 32 and 64, so the first growth that
  frees a real buffer is on the path. This check is the *scenario*, and
  D-126's is the detector: on the unfixed code D-126's check names
  `heap.rs` under `handle.rs` and fails. Forty handles is above
  `HANDLE_QUOTA`, so no program reaches it.

**Left owed.** The rule is checkable on any path a boot walks (D-126), and
unwalked paths remain the gap: the arena's third growth is reached by this
check, a fourth is not.
