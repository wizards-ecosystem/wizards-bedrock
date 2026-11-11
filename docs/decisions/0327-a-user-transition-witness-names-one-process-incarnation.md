# D-327 A user-transition witness names one process incarnation

Status: Accepted, 2026-09-27. Amends D-101 and D-288; closes PW-549.

**Decision.** The witness keeps the `Spawned` identity returned by `user::run`.
It waits for immutable last rites whose slot and non-repeating authority-table
identity both name that process, then requires the published syscall record to
name the same slot before checking privilege, vector and user-stack fields.
The change is self-test-only and reuses the exact-incarnation helper already
used by later program witnesses; no kernel scheduling or user-transition
behavior changes.

**Rationale.** `a thread crosses into ring three and calls back` failed once in
debug QEMU and passed on rerun. It waited for a global syscall count, read the
last global frame and sampled a global retirement count, which cannot prove
that the process returned by `user::run` both made the recorded call and
reached retirement; D-288 had already rejected that identity-free evidence
shape for other lifecycle checks. Mutations 1,390 and 1,391, inverting the
exact-retirement result and substituting a foreign syscall slot, each fail
only the named crossing.

**Alternatives.** Increasing the two-second wait was rejected because duration
cannot create identity. Treating the clean rerun as sufficient was rejected
because the failed gate exposed a real proof gap even if its schedule is rare.
Retaining the global retirement counter with another comparison was rejected
because slot reuse and unrelated retirements remain admissible kernel behavior.

**Amended by D-341.** Mutations 1,390 and 1,391 above are permanent ids 1,407
and 1,408.

**Amended by D-354.** The same exact-incarnation shape now binds the two fault
witnesses `a_faulting_program_dies_alone` and
`a_program_cannot_aim_at_the_copy_window`; the redundant retirement term this
check carried is removed (PW-564).
