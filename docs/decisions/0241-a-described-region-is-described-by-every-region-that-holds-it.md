# D-241 A described region is described by every region that holds it

Status: Accepted, 2026-08-29. Amends D-177 and D-224; advances PW-006.

**Decision.** A step of the walk consults every region holding the cursor. It
advances by the furthest of their ends, so the stride cannot depend on order,
and a region whose kind is not admitted refuses the range wherever it overlaps,
the safe direction under a contradiction, since the distinction exists to keep
a cacheable alias off device registers. `first_memory_run` returns only a base
its sibling contract agrees is memory, making the two answers consistent by
construction rather than by the caller checking both. `allocatable_bytes`
saturates. On a conforming map exactly one region holds any cursor, so none of
this changes what a real boot observes. The Limine guarantee this rests on,
that usable and bootloader-reclaimable entries overlap no other entry, is
stated where the walk relies on it.

**Rationale.** `libs/memory-map`'s `covers` searched the complete map but took
the first region holding the cursor, so with two regions sharing a base the
listing order decided both verdict and stride. `covers` and `covers_memory`
were order-dependent, `first_memory_run` could name a base `covers_memory`
denied was memory, contradicting D-177's contract that callers act on the
first, and `allocatable_bytes` summed lengths bare, panicking under the debug
gate and wrapping under release. Nothing required regions to be disjoint, and
`limine_glue::take_memory_map` passes entries through unsorted and unchecked,
so the crate was correct only by an external promise it neither stated nor
checked. A property tier transcribing claims from the doc comments rather than
the bodies found it, the only way this class is reachable, since the code is
total on every input while implementing the wrong rule.

**Alternatives.** Rejecting overlap in `try_from_regions` was rejected for now:
it is the strictest closure and would refuse a firmware map this project has
never seen, which is a boot failure traded for a latent inconsistency. Stating
the precondition and documenting the queries as undefined under overlap was
rejected because it leaves a trap in a public API and the crate cannot check
that its one caller honours it. Admitting the range wherever *some* region
allows it was rejected as the unsafe reading of the same contradiction.

**Consequences.** Three checks carry the three claims, and the bare-sum one
reproduces the overflow panic itself. Mutations 292 through 294 keep them red
for first-region-wins, a dropped cross-contract agreement and the bare sum.
