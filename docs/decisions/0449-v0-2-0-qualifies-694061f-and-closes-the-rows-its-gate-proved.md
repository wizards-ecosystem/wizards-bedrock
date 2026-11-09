# D-449 v0.2.0 qualifies `694061f` and closes the rows its gate proved

Status: Accepted (2026-10-28); applies D-425, D-441 and D-448

**Context.** D-425 tags the qualified commit directly and closes, in the next
commit, every register row its gate proves. `694061f` carries D-447's
retargeted mutation and D-448's version.

**Decision.** v0.2.0 is the annotated tag on `694061f` and its GitHub release,
the first qualified diagnostic release. All of the following ran on that exact
commit. `cargo xtask ci` passed: release and debug self-tests at 294/294, shipped
and both degraded boots, all 2,235 public API callers and 946 host tests. The
complete 2,412-entry sweep caught every mutation (report sequence 2). Model
checking, a fresh advisory review of 111 locked dependencies against 1,380
advisories, `debug --check` and the six-boot baseline passed. Debug boots
reproduced the exact reds of mutations 2,441, 2,442 and 2,444 before
restoration. Two independently generated VMware self-test machines passed
294/294 and the shipped machine observed one ring-3 call. Each was accepted by
`qualify-vmware`, and in each the guest read capabilities `0x44` and selected
paired roots. `cargo xtask evidence` and `verify-evidence` passed with the
`same-commit` relation and byte-identical artifacts from independent roots.
The grade and Deep campaigns are `deferred`. A 2026-10-27 independent
post-patch review of D-392's current code found no actionable finding. The
release carries the runtime archive, the evidence archive, the manifest and
their digests.

The register closes PW-001 through PW-005, PW-009, PW-028, PW-128, PW-201,
PW-301, PW-305, PW-534, PW-536, PW-537, PW-540, PW-543, PW-571, PW-578 through
PW-580, PW-592, PW-594 through PW-599, PW-706, PW-709 and PW-713 through PW-723,
and the D-142 and D-148 dispositions. PW-006 and PW-010 stay live: the
phase-close hostile-surface audit has not been redone. TRIGGER rows keep their
named conditions, and no bare-metal hardware is qualified.

**Consequences.** The kernel foundation does not close while PW-006 and PW-010
remain. A mutation report binds the exact Git index, so an operator must not
restore or edit checkout files between the sweep and evidence; doing so here
forced the second sweep. Every qualification record must also resolve the same
`git` executable, so all qualifications run from one shell environment.
