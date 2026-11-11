# D-311 Boundary witnesses fail at the named guard

Status: Accepted, 2026-09-12. Amends D-244 and D-301; advances PW-001.

**Decision.** Expose narrow target and scratch-boundary helpers to their local
tests so each redirected path is refused before later setup or destructive
work. Retain the shared real-directory ancestry check as the sole model scratch
boundary and remove its redundant parent comparison. Give the final unleased
probe command restoration a dedicated ignored-write and retry-authority test,
`an_ignored_final_probe_command_restore_retains_retry_authority`. Place the
nested inherent implementation inside an outer implementation's method so
finding it requires the exact visitor recursion under test.

**Rationale.** Four survivors had witnesses that passed without the changed
predicate: a missing repository failed before target preparation, a parent
equality repeated the ancestry check, the PCI fixture exercised leased rather
than unleased command restoration, and the nested implementation was reachable
without recursion.

**Alternatives.** Keeping broad end-to-end error assertions was rejected
because an earlier or later guard could continue satisfying them. Retaining the
redundant model parent predicate was rejected because no filesystem state can
pass the shared ancestry validation and then violate it. Reusing the leased
command fixture was rejected because the two restoration paths hold different
authority.
