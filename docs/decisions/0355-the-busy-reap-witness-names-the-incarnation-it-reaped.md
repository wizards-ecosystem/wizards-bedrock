# D-355 The busy-reap witness names the incarnation it reaped

Status: Accepted, 2026-09-29. Amends D-354; closes PW-565.

**Context.** D-354 bound two fault witnesses to the incarnation that faulted.
One lifecycle witness still kept the identity-free shape:
`a_busy_machine_reaps_a_program_without_another_spawn` dropped the `Spawned`
that `user::run_for_bootstrap` returned and proved a reap had happened by
watching the global retirement, worker-reap and reclaim counters, so a
retirement or a reap belonging to another program could satisfy it.

**Decision.** The witness keeps the `Spawned`. It confirms that exact
incarnation retired through its last rites, which orphaning records at
retirement and which persist until the slot is reused, and this check spawns
nothing else to reuse it. It then confirms the busy worker reaped that
incarnation without a spawn: the slot holds no zombie and the worker-reap and
reclaim counters advanced. Because the rites are recorded before reclamation
and D-117's reclaim path does not clear them, the identity survives however
fast the busy worker reaps, and no wait races the reap. The change is
self-test-only; no kernel behaviour changes.

**Evidence.** On 2026-09-29 the debug self-test passed 265/265 and three
consecutive debug images repeated it, ruling out a reap the witness could race.
Mutation 1,538 inverts the last-rites check so the witness would accept a reap
without its child's own rites, and it failed only `a busy machine reaps a
program without another spawn`.

**Alternatives.** Observing the zombie before the reap was rejected because the
busy worker can reclaim it before the check runs; the persistent rites need no
such window. Keeping only the global reap counters was the defect: they prove a
reap happened, not that this program's reap happened.
