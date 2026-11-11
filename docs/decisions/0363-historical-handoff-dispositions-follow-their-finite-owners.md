# D-363 Historical handoff dispositions follow their finite owners

Status: Accepted, 2026-10-07. Amends D-325 and D-330; closes PW-577.

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

**Rationale.** The PW-006 closing draft found that the historical
reconciliation check unconditionally required D-142 and D-148 to remain
`READY`. Once their finite register owners closed, that check would reject
the documentation-only child D-330 permits, forcing another source change and
another qualification. The README and PW-009 also still named superseded
evidence format numbers.

**Alternatives.** Keeping `READY` after its owners finish would leave a false
backlog. Editing the check in the closing child would violate D-330's source
freeze. Weakening the check to accept either label would lose the relationship
between a historical obligation and its finite current owners.

**Consequences.** Retained host mutations 1,599 (`READY` after all owners
completed), 1,600 (`CLOSED` with a live owner) and 1,601 (an invalid owner
state admitted) each fail
`historical_handoffs_close_only_after_every_owner_finishes`.
