# D-354 Two fault witnesses name the incarnation that faulted

Status: Accepted, 2026-09-29. Amends D-217 and D-327; closes PW-564.

**Decision.** Each fault witness, `a_faulting_program_dies_alone` and
`a_program_cannot_aim_at_the_copy_window`, keeps the `Spawned` that
`user::run_faulting` or `user::run_into_the_copy_window` returned. It waits for
a fault whose slot names that process, then requires the fault's `rax` to
carry the argument the program left there: 1 for the deliberate `ud2`, and the
copy-window address for the indirect jump that faults on its fetch. Retirement
is proven by the exact last rites of that incarnation, not a global tally, and
the copy-window witness adds that the fault was a page fault the recovery arm
did not serve. The crossing witness's trailing retirement term, already
guaranteed true by its own guard, is removed. The change is self-test-only; no
kernel behaviour changes.

**Rationale.** Both witnesses read the global last user fault and waited on a
global retirement count, the shape D-327 and D-288 rejected, so a fault or a
retirement belonging to another process could stand in for the child's.
Mutation 1,536 makes the faulting witness require a foreign slot and mutation
1,537 does the same to the copy-window witness; each failed only its named
check.

**Alternatives.** Widening the fault wait was rejected because duration cannot
create identity. Reading `rax` without the slot was rejected because a second
program can leave the same value; the slot names the incarnation and the `rax`
names its intent. Retaining the global retirement counter was rejected because
slot reuse and unrelated retirements remain admissible kernel behaviour, which
is the gap D-327 closed for the crossing.

**Amended by D-355.** The one remaining identity-free lifecycle witness,
the busy-reap check, is bound to its incarnation the same way (PW-565).
