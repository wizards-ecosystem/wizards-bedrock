# D-396 Corrupt-state witnesses isolate one queue invariant at a time

Status: Accepted, 2026-10-13. Amends D-395; advances PW-001.

**Decision.** Before forging the exclusive head value, the test sets the empty
queue's tail to the modulo-equivalent zero value which is consistent with that
head under every other relation. Production must then reject the state solely
because `head == WORK_DEPTH` is outside the slot domain. The following tail,
length and occupancy corruptions remain separate observations, and mutation
686 keeps `a cancelled operation and its cancellation complete separately` as
its exact witness.

**Rationale.** Mutation 686 changes the file-work queue's exclusive head bound
from `head >= WORK_DEPTH` to `head > WORK_DEPTH`, but the test had kept the
tail from an earlier wrapped round trip, so the independent tail/length
relation still refused the forged state and the mutation survived: two
simultaneous faults could not show which guard rejected them. Changing only the
expected result would ignore a genuinely removed bound, another broad queue
check would duplicate the existing accounting model, and accepting rejection by
any second corrupt field leaves the mutated head predicate unobserved.
