# D-325 Hostile-surface live classifications are checked in both directions

Status: Accepted (2026-09-27), advances PW-010

**Context.** The hostile-surface oracle required every live register row to
appear somewhere in the census, but it did not prove that a row explicitly
advertised as `OPEN` was still live. Completed rows therefore remained mixed
into open lists and the live cross-cutting table after their register state
changed. Their presence overstated unfinished work and obscured which evidence
the phase close still owes.

**Decision.** In a hostile-surface proof cell, the exact `` `OPEN`: `` segment
ends at its next semicolon or at the cell boundary. Every `PW-*` identity in
that segment must exist in the finite register and have one of its four live
states: `REQUIRED`, `READY`, `VERIFY` or `DECIDE`. Closed evidence may be cited
before the segment, and stopped triggers or related work may follow its
semicolon, but neither is classified as open. Every identity in the explicitly
live cross-cutting table must meet the same rule. The existing reverse
coverage—every live register row occurs somewhere in the census—remains
independent.

**Evidence.** Before the census cleanup, the new check failed with the exact
diagnostic `PW-524 (DONE)`; moving that closed proof outside the segment made
the same focused test pass. The audit also removed completed `PW-518`,
`PW-520`, `PW-521`, `PW-525`, `PW-526`, `PW-532`, `PW-533` and `PW-544` from
open or live classifications without erasing their underlying proof. Retained
mutations replace live `PW-519` with completed `PW-520` in an `OPEN` segment
and live `PW-541` with completed `PW-525` in the cross-cutting table. The latter
first made only the focused check red with `PW-525 (DONE)`, then passed after
restoration, so neither reverse rule can silently stop enforcing register state.

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
