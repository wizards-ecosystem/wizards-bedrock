# D-396 Corrupt-state witnesses isolate one queue invariant at a time

Status: Accepted, 2026-10-13. Amends D-395; advances PW-001.

**Context.** Permanent mutation 686 changes the file-work queue's exclusive
head bound from `head >= WORK_DEPTH` to `head > WORK_DEPTH`. The existing
self-test did assign `WORK_DEPTH` to the head, but it retained the tail left by
an earlier wrapped round trip. That tail was inconsistent with the forged head
even under the mutation, so the independent tail/length relation still refused
the state and the named check passed. The sweep therefore correctly reported a
survivor: the test constructed two simultaneous faults and could not show which
guard rejected them.

**Decision.** Before forging the exclusive head value, the test sets the empty
queue's tail to the modulo-equivalent zero value which is consistent with that
head under every other relation. Production must then reject the state solely
because `head == WORK_DEPTH` is outside the slot domain. The following tail,
length and occupancy corruptions remain separate observations, and mutation
686 keeps the existing exact machine check as its witness.

**Proof required.** The unmutated round-trip check passes. Applying mutation
686 alone makes `a cancelled operation and its cancellation complete
separately` fail, and restoring it returns the check to green. The repaired
registry and source must be included in the next complete clean sweep.

**Alternatives.** Merely changing the expected mutation result was rejected
because the mutation genuinely removes an index bound. Adding another broad
queue check was rejected because the existing check already owns the complete
fixed-work and cancellation accounting model. Accepting rejection by any
second corrupt field was rejected because it leaves the mutated head predicate
unobserved.
