# D-325 Hostile-surface live classifications are checked in both directions

Status: Accepted, 2026-09-27. Advances PW-010.

**Decision.** In a hostile-surface proof cell, the exact `` `OPEN`: `` segment
ends at its next semicolon or at the cell boundary. Every `PW-*` identity in
that segment must exist in the finite register and have one of its four live
states: `REQUIRED`, `READY`, `VERIFY` or `DECIDE`. Closed evidence may be cited
before the segment, and stopped triggers or related work may follow its
semicolon, but neither is classified as open. Every identity in the explicitly
live cross-cutting table must meet the same rule. The existing reverse
coverage, every live register row occurs somewhere in the census, remains
independent.

**Rationale.** The oracle required every live row to appear in the census but
never proved that a row advertised as `OPEN` was still live, so completed rows
stayed in open lists and the live cross-cutting table, overstating unfinished
work and obscuring which evidence the phase close still owes.

**Evidence.** Before the census cleanup the new check failed with exactly
`PW-524 (DONE)`. Retained mutations replace live `PW-519` with completed
`PW-520` in an `OPEN` segment and live `PW-541` with completed `PW-525` in the
cross-cutting table, so neither direction can silently stop enforcing
register state.

**Alternatives.** Treating any mention of a row as an open classification was
rejected because one proof cell can legitimately cite closed evidence, current
work and a future trigger. Requiring authors to remember the distinction was
rejected because that was the mechanism which drifted. Removing completed
identities everywhere was rejected because historical evidence remains useful;
only its current-state label was wrong.

**Amended by D-341.** The evidence above is the run as recorded on 2026-09-27,
when the cross-cutting mutation replaced live `PW-541`; a later commit had
rewritten that name in place, and D-341 restores it. As rows closed, both
retained mutations were re-anchored; they are now ids 18 and 19, anchored on
closed evidence and on the cross-cutting heading, which no closure moves. D-341
also refuses a second `OPEN` segment in one proof cell, an `OPEN` label outside
the proof column and an unreadable cross-cutting row.
