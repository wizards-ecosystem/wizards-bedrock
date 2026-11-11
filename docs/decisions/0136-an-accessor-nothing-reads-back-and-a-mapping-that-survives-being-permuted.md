# D-136 An accessor nothing reads back, and a mapping that survives being permuted

Status: Accepted, 2026-08-25.

**Decision.** Two rules, each with its instances pinned against something that
is not itself:

- **A `pub` accessor is part of the contract and needs an assertion of its
  own.** Derived `PartialEq` compares *fields*, not accessors, so a round trip
  proves something about representation, not interface. `Slot::index`,
  `SizeClass::index` and `AccessRange::offset` are asserted against values
  chosen to be **pairwise distinct**, so no getter can stand in for another and
  pass. Each wrong getter reached the machine: a handle resolving to the slot
  its generation names, the allocator indexing off its array, an MMIO write
  loop starting at the end.
- **When a mapping's inverse is also under test, one of the two must be pinned
  to an external source**, or the pair can rotate freely. `trap-cause` asserts
  all twenty-one `(vector, variant)` pairs **transcribed from SDM Vol. 3A table
  6-1**, both directions, against literals, and the reserved gaps (9, 15,
  22 to 27, 31) stay gaps. A consistent swap of `InvalidOpcode` and `MachineCheck`
  would make a ring-3 `ud2` miss the survivable arm and panic the machine: **an
  unprivileged halt, from a permutation nothing could see.**

`eevdf` runs a thread one tick past its deadline, then a thousand slices past,
and requires `remaining_slice()` to be zero; without the `slice_is_spent` guard
it wraps to near `u64::MAX` and the running thread is never preempted again.

**Rationale.** These were found by mutation, not reading, and **a real defect
reached the machine and nothing in the tree caught it.** The permuted pair
described each other faithfully and described the SDM not at all: D-124's
description-versus-thing at one remove. Every mapping and accessor is currently
correct; what was missing was any way to find out if that stopped being true.

**Consequences.** `trap-cause`'s test is the longest transcription in the tree,
and that is the point: the numbers are the architecture's, so they are written
out rather than referred to.

**Left owed.** The audit's other Tier 2 and 3 findings: bounds that are too
wide, observation windows that close too early, and names that claim more than
their bodies test. They are real and none of them is a soundness hole.
`cargo xtask mutate`'s mutations touch few files, and the checks it does *not*
reach are where most of these were found. A mutation table dense where it has
already been used is a mutation table that measures its own history.
