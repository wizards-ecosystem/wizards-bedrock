# D-301 A complete mutation report is a repair inventory, not a production verdict

Status: Accepted, 2026-09-11. Amends D-119 and D-244; advances PW-001.

**Decision.** PW-001 uses the complete report as its exact repair inventory.
A survivor is closed by strengthening a real proof gap or by replacing a
non-load-bearing mutation with a defect that removes the stated invariant. An
inconclusive attempt is repaired until it compiles and reaches its named
witness; a different failure is not credited. A mutation may name only a check
that executes its changed path. Redundant guards are not weakened merely to
make a partial mutation observable: their mutation must remove the complete
property or target one independently observable layer.

The FAT32 entry now removes the parser's rejection of a nonzero selector while
mirroring is active and names the existing meaningless-selector test. The port
entry keeps its original half-open defect and names the existing pairwise
reservation test. The other reported outcomes remain open inventory, not
silently adjudicated equivalents, and the next clean complete sweep is
PW-001's closing evidence.

**Rationale.** The first clean complete report, on `f28428a`, is source-,
registry- and index-bound, so it settles which attempts ran but not that every
survivor names a missing production invariant. The FAT32 survivor changed a
branch an earlier parser guard already made unreachable, and the port survivor
named a test that never ran the changed helper.
