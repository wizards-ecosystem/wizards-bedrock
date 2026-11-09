# D-136 An accessor nothing reads back, and a mapping that survives being permuted

Status: Accepted (2026-08-25)

**Context.** The Phase 0–5 audit returned roughly forty checks that cannot
fail, verified by mutation rather than by reading: the crate copied aside, the
defect applied, `cargo test` run. Eight of them share a property that makes
them worth a decision rather than eight fixes — **a real defect reaches the
machine and nothing in the tree catches it.** This entry takes the five in
`libs/`, where the whole justification for the crate is that `cargo test`
reaches the arithmetic.

**Two shapes, and the second is the one worth naming.**

*An accessor no test reads back.* `Slot::index`, `SizeClass::index` and
`AccessRange::offset` were each called by production code and by no assertion.
Derived `PartialEq` compares *fields*, not accessors, so a test that round-trips
a value proves nothing about the getter — and each of these had one:

- `fn index(self) -> u32 { self.generation }` survives `handle-space`.
  `kernel/src/handle.rs` does `arena.slots.get(named.index())`, so every handle
  resolves to whichever slot the generation names, with the generation check
  still passing.
- `{ self.index + 1 }` survives `slab-layout`. `wost/src/mm/heap.rs` indexes
  `self.free[class.index()]` — the largest class runs off the array, a panic
  inside the allocator.
- `{ self.end }` survives `mmio-range`. `wost/src/mmio.rs` begins its write
  loop at `offset()`, under three `// SAFETY:` comments citing this type.

*A mapping that survives a consistent permutation.* `trap-cause` decodes a
vector to a `Fault` and `Fault::vector` maps back. Both existing tests route
through **both** directions, so any permutation applied to both satisfies them.
Swap `6 => InvalidOpcode` with `18 => MachineCheck` in `decode` *and* in
`vector` and the suite stays green.

That is not cosmetic. `kernel/src/arch/x86_64/trap.rs` decides whether a fault
is the program's to die of by matching on the **variant**. Under that swap a
ring-3 `ud2` arrives as `MachineCheck`, misses the survivable arm, and panics
the machine: **an unprivileged halt, from a permutation nothing could see.**
Only `Breakpoint` and `PageFault` were pinned to a number.

*And one bound never approached.* No test called `remaining_slice()` with
`vruntime` **past** `deadline` — the ones that looked like they did landed
exactly on it, where `wrapping_sub` gives zero anyway. Deleting the
`slice_is_spent` guard survives, and `deadline.wrapping_sub(vruntime)` then
wraps to near `u64::MAX`, which `kernel/src/thread.rs` arms the tickless timer
with. The running thread is never preempted again.

**Decision.** Each is now pinned against something that is not itself:

- The three accessors are asserted against values chosen to be **pairwise
  distinct**, so no getter can stand in for another and pass.
- `trap-cause` gains all twenty-one `(vector, variant)` pairs **transcribed
  from SDM Vol. 3A table 6-1**, both directions, against literals. The reserved
  gaps — 9, 15, 22–27, 31 — are asserted to stay gaps, so a variant renumbered
  into one is caught too.
- `eevdf` runs a thread one tick past its deadline, then a thousand slices
  past, and requires zero.

All five broken once and watched go red, with the exact mutation the audit
named.

**Rationale.** The accessor shape is the more common and the less visible.
Rust's derived equality makes a round-trip test feel like coverage: encode,
decode, compare — and the comparison never touches the getters that production
code actually calls. The rule that follows is worth stating in general: **a
`pub` accessor is part of the contract and needs an assertion of its own**;
a struct that survives a round trip has proved something about its
representation, not about its interface.

The permutation shape is rarer and worse. Two tests, both real, both testing a
genuine property — self-consistency — and neither testing the property that
matters, which is agreement with the *architecture*. That is D-124's
description-versus-thing at one remove: the pair described each other
faithfully and described the SDM not at all. The general form: **when a
mapping's inverse is also under test, one of the two must be pinned to an
external source**, or the pair can rotate freely.

**Consequences.**

- Five `libs/` crates gain a check each; no production code changed. Nothing
  here was a bug — every mapping and accessor is currently correct. What was
  missing was any way to find out if that stopped being true.
- `trap-cause`'s new test is the longest transcription in the tree, and that is
  the point: the numbers are the architecture's, so they are written out rather
  than referred to.
- The remaining Tier 1 findings — the `verw` counter's adjacency, the
  `xstate` save form, and the leaf-guard detector's read position — are in
  `kernel/` and `wost/` and are handled separately.

**Left owed.** The audit's other ~32 findings are Tier 2 and 3: bounds that are
too wide, observation windows that close too early, and names that claim more
than their bodies test. They are real and none of them is a soundness hole. The
sweep also noted its own blind spot, which is worth repeating: `cargo xtask
mutate`'s mutations touch nine files, and the checks it does *not* reach are
where most of these were found. A mutation table dense where it has already
been used is a mutation table that measures its own history.
