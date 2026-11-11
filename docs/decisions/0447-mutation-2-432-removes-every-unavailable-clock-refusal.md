# D-447 Mutation 2,432 removes every unavailable-clock refusal

Status: Accepted, 2026-10-27. Amends D-438, advances PW-001.

**Decision.** Permanent mutation 2,432 keeps its identity and defect but now
disables both the entry refusal and the loop's calibration guard, so an
unavailable clock again admits an otherwise-ready predicate as a successful
wait. Its named witness, `a delayed read preserves interrupt admission and
exact teardown`, is unchanged. Mutation 2,444 continues to remove the loop guard
alone. No kernel source, check or count changes.

**Rationale.** The complete sweep on `62ee03a` reported mutation 2,432
survived: it disabled only `runs_until`'s entry refusal, and after D-446 the
loop's recheck still refused the clockless delayed-read probe. The entry
check's remaining property, a calibrated public clock that stops advancing,
cannot be produced in software because every revocation that freezes public
time also clears calibration. Retiring 2,432 would leave no mutation that
removes the clockless refusal the delayed-read witness exists to prove, a seam
that stalls a calibrated clock would add kernel surface for an unreachable
state, and dropping the entry check would trade a bounded refusal for an
unbounded wait if calibrated time ever stopped advancing.
