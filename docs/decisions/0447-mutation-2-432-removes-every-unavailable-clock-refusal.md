# D-447 Mutation 2,432 removes every unavailable-clock refusal

Status: Accepted (2026-10-27); amends D-438, advances PW-001

**Context.** The complete 2,412-entry sweep on `62ee03a` caught 2,411 and
mutation 2,432 survived. It disabled only `runs_until`'s entry refusal. D-446
then made the shared admitted-deadline loop recheck calibration before every
predicate, so the clockless delayed-read probe still refuses through the loop
and the mutation became equivalent for its witness. The entry check's remaining
property, a calibrated public clock that stops advancing, cannot be produced in
software: every revocation that freezes public time also clears calibration.

**Decision.** Permanent mutation 2,432 keeps its identity and defect but now
disables both the entry refusal and the loop's calibration guard, so an
unavailable clock again admits an otherwise-ready predicate as a successful
wait. Its named witness is unchanged. Mutation 2,444 continues to remove the
loop guard alone. No kernel source, check or count changes.

**Proof.** The retargeted mutation applies once and the registry tests pass.
Under it, release and debug each completed at 292/294 with `a delayed read
preserves interrupt admission and exact teardown` among the two failures; the
other is the clock-disagreement witness that mutation 2,444 owns. The restored
kernel source is byte-identical to `62ee03a`. The ordinary gate and a complete
current-registry sweep remain required.

**Alternatives.** Retiring 2,432 would leave no mutation that removes the
clockless refusal the delayed-read witness exists to prove. A test seam that
stalls a calibrated clock would add kernel surface to exercise a state the
clock cannot reach. Dropping the entry check would trade a bounded refusal for
an unbounded wait if calibrated time ever stopped advancing.
