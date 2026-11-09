# D-288 Ownership proofs conserve allocator state and name one process incarnation

Status: Accepted (2026-09-05), amends D-26, D-217, D-245 and D-261; advances PW-542

**Context.** Several cleanup checks compared raw free-frame counts. A correct
slab refill may retain free frames without changing ownership, so those checks
could fail or need warm-up even when cleanup was exact. The zombie witness also
used wall time, global retirement counts and a reusable slot, allowing an
unrelated process incarnation or scheduling delay to decide the result.

**Decision.** The pure ownership relation conserves checked
`free + slab + large` physical frames, requires the exact live-allocation
count, and permits only later retained slab frames to be greater than or equal
to the earlier snapshot. The large-frame bucket may move when a still-live
allocation changes capacity or class; conservation, rather than bucket
identity, proves that those frames remain owned. One interrupt-excluded epoch
covers both the allocator snapshot and the broader spawn-resource ledger; a
host-tested `libs/` helper owns the arithmetic. Lifecycle checks combine that
relation with their exact live owner and refusal tallies. Zombie collection
names the exact `(slot, non-repeating table)` incarnation, waits by scheduler
yield, observes the full retained `(launches, bootstraps, zombies)` tuple, uses
a distinct-slot probe and requires exactly two successful reclaim steps. The
sum returned by those two successes must equal the exact page and table tally
deltas. Mapping cleanup separately counts per-attempt partial returns without
double-counting them.

**Alternatives and evidence.** Warm-up allocations, fixed sleeps, order
assumptions and global counts were rejected because they tune the witness to
incidental allocator or scheduler state. Exact slab equality was rejected
because retaining a newly populated slab is intentional. Exact large-frame
equality was rejected after a debug run kept every live owner and the total
physical account exact while one still-live allocation moved one frame from
the free pool into its large capacity. That false red corrected the oracle
rather than weakening ownership. Pure boundary tests, the debug counterexample,
retained mutations and repeated current boots remain PW-542's close. Six
relation defects, two torn-observation epochs and four zombie/resource-ledger
defects remain mutations 1,301 through 1,312.
