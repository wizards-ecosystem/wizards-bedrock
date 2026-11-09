# D-128 Growing the arena is two allocator operations, and one was left behind

Status: Accepted, 2026-08-25.

**Context.** `handle.rs`'s `reserve_a_slot` exists for one reason, and its doc
says so: `Vec` growth calls the allocator, which takes `HEAP` and possibly
`FRAMES`, and doing that under `ARENA`'s guard would rebuild the `ARENA` →
`HEAP` order D-85 deleted. So the new vector is allocated with no lock held and
handed over afterwards.

The handover was `arena.slots = grown`. That assignment drops the vector it
replaces — which after `append` has length zero and still owns its buffer. So
the *free* happened with the guard live, and a free is `dealloc`, and `dealloc`
takes `HEAP`. The function reconstructed the exact order it was written to
prevent, in the one line that was supposed to be the safe half.

Only half the operation had been moved out. Allocating and freeing are two
calls into the allocator, and the reasoning that identified the first as a
hazard did not carry to the second.

It was latent, and stayed latent for a reason worth recording: the first growth
replaces a vector of capacity zero, which owns nothing and whose drop is not a
`dealloc` at all. The second growth is the first one that frees anything, and
it needs sixteen handles live at once. No boot had ever held sixteen. D-126's
check is a runtime counter and sees only paths a boot walks, so it named this
nowhere — and D-126 says so in its own text, naming this function as the
worked example of what it cannot catch.

**Options.**

1. **Copy the entries out and back** rather than moving the vector, so no
   vector is discarded. Trades one allocation for `n` moves and leaves the same
   drop somewhere else.
2. **Keep the old buffer**, growing into a second vector and never releasing the
   first. A leak that grows with the arena.
3. **Name the discarded vector and free it after the guard**, which is what the
   function already does for the allocation.

**Decision.** Option 3. The `if` becomes an expression yielding whichever
vector this call is left holding — the arena's old buffer where the handover
happened, the unused room where another caller got there first — and the guard
is dropped explicitly before it.

The explicit `drop(arena)` is required, not decoration: the discarded vector is
declared after the guard, so end-of-scope order would release it first, which
is the bug.

**Rationale.** Option 1 is the version that looks safest and is not. Moving `n`
entries under the guard replaces a bounded `dealloc` with an unbounded copy, and
the discarded vector still has to be freed somewhere — the drop moves, it does
not disappear.

The general shape is the one D-124 named: a comment that states a property is
not the property. This function's doc asserted twice that every acquisition it
makes is a leaf, and the assertion was read as evidence for eight commits.

**Why it stayed hidden is the part worth keeping**, and it is a different
failure from the four D-126 fixed. Those were all *visible calls* — a function
that takes a lock, called from under a guard, findable by reading the call
site. This one had no call site. `reserve_a_slot` is right about every
acquisition it makes; the lock it took was taken by a `Drop`, and a `Drop` is
invisible at the point that triggers it. There is nothing to read.

That is the strongest argument yet for D-126's counter over a lint. A static
analysis of lock order would have to model destructor insertion across every
scope exit to see this, which is most of a borrow checker for a property the
borrow checker does not have. A runtime counter sees it the first time a boot
executes the drop, and needs to know nothing about why the drop is there.

**Consequences.**

- One machine check, `the handle arena grows more than once`: forty handles in
  one table, which walks the arena's capacity through 16, 32 and 64. Two
  reallocations, so the second one — the first that frees a real buffer — is on
  the path.
- Broken and watched go red (D-28), and the red is D-126's check rather than
  this one's own assertions: on the unfixed code the boot prints
  `wost/src/mm/heap.rs took a lock under the leaf guard at kernel/src/handle.rs`
  and fails. That is the intended division — this check is the *scenario*, and
  D-126's is the detector. It is also the first thing D-126's check found that
  D-126 had not already fixed.
- The arena's growth path is now exercised by every self-test boot, which it
  was not. Forty handles is above `HANDLE_QUOTA`, so no program reaches it;
  before this, only the first growth had ever run.

**Left owed.** `HandleTable`'s own `Drop` sizes its release vector before taking
the guard for the same reason and gets it right, and `close` releases the object
after the guard for the same reason and gets it right. Both were written by
someone holding the rule in mind. The rule is now checkable on any path a boot
walks (D-126), and unwalked paths remain the gap — the arena's third growth is
reached by this check, a fourth is not.
