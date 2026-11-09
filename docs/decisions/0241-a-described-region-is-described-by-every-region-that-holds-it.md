# D-241 A described region is described by every region that holds it

Status: Accepted (2026-08-29), amends D-177 and D-224; advances PW-006

**Context.** `libs/memory-map`'s `covers` says, in its own words, "Input order is
not assumed, so each bounded step searches the complete map." It searched the
complete map and took the *first* region holding the cursor, then advanced to
that region's end — so with two regions sharing a base, which one was listed
first decided both the verdict and the stride. Swapping them swapped the answer.

Three consequences followed from the one cause. `covers` and `covers_memory`
were order-dependent. `first_memory_run` could name a base that `covers_memory`
denied was memory, because the filter chose from a direct-mapped region while
the walk landed in a reserved one overlapping it — two stated contracts about
one address, disagreeing, with D-177 saying callers act on the first.
`allocatable_bytes` summed lengths bare, which panics under the debug gate and
wraps under release, handing a frame allocator a total smaller than the memory
it describes.

Nothing anywhere required regions to be disjoint. `grep -niE
"overlap|disjoint|sorted"` over the crate, `001-boot.md` and `007-memory.md`
returns nothing, and `limine_glue::take_memory_map` passes the bootloader's
entries through unsorted and unchecked. What keeps this latent is the Limine
protocol, which specifies that usable and bootloader-reclaimable entries do not
overlap any other entry — a guarantee this repository never records. So the
crate was correct by an external promise that is neither stated in its doc,
checked at its boundary, nor written in the boot document, which is a worse
position than being wrong in a visible way.

**Decision.** A step of the walk consults every region holding the cursor. It
advances by the furthest of their ends, so the stride cannot depend on order,
and a region whose kind is not admitted refuses the range wherever it overlaps
— the safe direction under a contradiction, since the distinction exists to
keep a cacheable alias off device registers. `first_memory_run` returns only a
base its sibling contract agrees is memory, making the two answers consistent by
construction rather than by the caller checking both. `allocatable_bytes`
saturates. On a conforming map exactly one region holds any cursor, so none of
this changes what a real boot observes.

**Alternatives.** Rejecting overlap in `try_from_regions` was rejected for now:
it is the strictest closure and would refuse a firmware map this project has
never seen, which is a boot failure traded for a latent inconsistency. Stating
the precondition and documenting the queries as undefined under overlap was
rejected because it leaves a trap in a public API and the crate cannot check
that its one caller honours it. Admitting the range wherever *some* region
allows it was rejected as the unsafe reading of the same contradiction.

**Consequences.** Found by a second session running a property tier against a
detached copy of the crate, transcribing the claims from the doc comments rather
than from the bodies — which is the only way this class is reachable, since the
code is total on every input and stays total while implementing the wrong rule.
Three checks now carry the three claims; restoring first-region-wins, dropping
the cross-contract agreement and restoring the bare sum each redden only their
own, and the last reproduces the overflow panic itself. Mutations 292 through
294 retain them. No unsafe operation, wire ABI or machine-check count changed,
and the Limine guarantee this rested on is now stated where the walk relies on
it rather than nowhere.
