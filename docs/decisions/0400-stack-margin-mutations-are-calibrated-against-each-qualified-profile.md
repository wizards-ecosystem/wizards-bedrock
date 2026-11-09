# D-400 Stack-margin mutations are calibrated against each qualified profile

Status: Accepted (2026-10-13), amends D-359, D-361 and D-395; advances PW-001

**Context.** Permanent mutation 1,679 adds a 4 KiB frame to the borrowing
`isolate_candidate` stage to prove that unit-wide VT-d establishment remains a
thin sibling of the other storage-candidate stages. The complete release sweep
reported a survivor. Its passing record still left more than the required two
pages because the release candidate's existing deepest path, not the mutated
isolation stage, remained the high-water mark. The original 4 KiB pressure was
calibrated for the larger debug call chain; it did not create the claimed
release-margin violation. Production continues to borrow the prepared device
through a separate stage and the painted-stack observer covered the mutation,
so this is a stale mutation calibration rather than an unmeasured production
frame or a DMA-isolation escape.

**Decision.** Keep production and the two-page boot-stack requirement
unchanged. Mutation 1,679 uses profile-specific pressure, as the older
non-leaf stack mutation already does: 4 KiB in debug and 16 KiB in release.
Both values remain inside the guarded 32 KiB boot stack on the calibrated path
while making the intended regression exceed the retained two-page headroom
boundary. The array remains live across `iommu::establish`, so the check proves
the linked isolation path rather than a disconnected allocation.

**Proof required.** The unmodified release self-test must pass
`nvme candidates preserve two pages of boot-stack headroom`. Applying mutation
1,679 alone must make that exact check fail without a guard fault or unrelated
terminal verdict, and restoring the source must return it to green. The
registry application and named-check contracts, the ordinary gate and the
next complete clean sweep must all include the repaired mutation.

**Alternatives.** Lowering the production headroom requirement was rejected
because the survivor showed no product regression. Crediting any machine
failure was rejected because a stack guard fault would not prove the measured
margin. Moving the pressure into `iommu::establish` was rejected because the
mutation specifically guards the candidate-stage separation. Keeping one
pressure size for both profiles was rejected because their optimized call
chains have materially different high-water marks.
