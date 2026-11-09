# D-363 Historical handoff dispositions follow their finite owners

Status: Accepted, 2026-10-07. Amends D-325 and D-330; closes PW-577.

**Context.** The PW-006 closing draft found that the historical reconciliation
check unconditionally requires D-142 and D-148 to remain `READY`. Once their
finite register owners close, that check would reject the documentation-only
child D-330 permits, forcing another source change and another qualification.
The README and PW-009 also still name superseded evidence format numbers.

**Decision.** Each of these two dispositions is `READY` while any cited
register owner is live and `CLOSED` only when every cited owner is `DONE` or
`SUPERSEDED`. Every owner must exist and have an allowed state; absent,
unknown and stopped owners fail the check. Independent fixtures exercise
both directions and mixed ownership before the final qualification. Current
documentation names evidence v8 and qualification v4 under D-344.

The closing child records qualifications already completed on its parent.
Its own ordinary gate and evidence are produced and verified after committing
that child, as D-330 requires. No draft may assert those future results or
invent their digest; completion is established only by the resulting verified
artifacts. A failed closing gate or bundle keeps the foundation open.

**Proof required.** Retained host mutations must independently make completed
owners appear live and live owners appear completed. The real historical
reconciliation and ordinary gate must pass before this slice is committed.

**Evidence.** On 2026-10-07
`historical_handoffs_close_only_after_every_owner_finishes` passed. Mutation
1,599 forced `READY` after all owners completed, mutation 1,600 forced
`CLOSED` with a live owner, and mutation 1,601 admitted an invalid owner
state. Each selected that one test, returned exit 101 with the exact witness
marked `FAILED`, and was restored byte for byte.

**Alternatives.** Keeping `READY` after its owners finish would leave a false
backlog. Editing the check in the closing child would violate D-330's source
freeze. Weakening the check to accept either label would lose the relationship
between a historical obligation and its finite current owners.
