# D-400 Stack-margin mutations are calibrated against each qualified profile

Status: Accepted, 2026-10-13. Amends D-359, D-361 and D-395; advances PW-001.

**Decision.** Keep production and the two-page boot-stack requirement
unchanged. Mutation 1,679, which adds a frame to the borrowing
`isolate_candidate` stage to prove that unit-wide VT-d establishment remains a
thin sibling of the other storage-candidate stages, uses profile-specific pressure, as the older
non-leaf stack mutation already does: 4 KiB in debug and 16 KiB in release.
Both values remain inside the guarded 32 KiB boot stack on the calibrated path
while making the intended regression exceed the retained two-page headroom
boundary. The array remains live across `iommu::establish`, so the check proves
the linked isolation path rather than a disconnected allocation. Applying the
mutation alone must fail `nvme candidates preserve two pages of boot-stack
headroom` without a guard fault or unrelated terminal verdict.

**Rationale.** The release sweep reported 1,679 as a survivor because the
release candidate's deepest path, not the mutated stage, stayed the high-water
mark; the 4 KiB pressure had been calibrated for the larger debug chain, so this
was a stale calibration rather than an unmeasured production frame. Lowering
the production requirement would answer no product regression, a stack guard
fault would not prove the measured margin, and moving the pressure into
`iommu::establish` would stop guarding the candidate-stage separation.
