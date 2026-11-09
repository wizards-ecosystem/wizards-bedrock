# D-301 A complete mutation report is a repair inventory, not a production verdict

Status: Accepted (2026-09-11), amends D-119 and D-244; advances PW-001

**Context.** The first clean complete run of the 1,359-entry current registry
finished on `f28428a`: 1,301 mutations were caught, 34 survived and 24 were
inconclusive. The report is source-, registry- and index-bound and therefore
settles which attempts ran. It does not settle that every survivor identifies
a missing production invariant. Mutation 344 changed the active-table choice
for mirrored FAT32 volumes, but the preceding parser guard already requires
their selector to be zero, so the changed branch could not alter behavior.
Mutation 567 changed the pairwise half-open overlap helper, but named a test
that exercises only the bitmap path; an existing reversed-adjacency assertion
does exercise the helper and failed under the defect.

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
reservation test. The other 56 reported outcomes remain open inventory, not
silently adjudicated equivalents.

**Proof.** With the port overlap changed from `<` to `<=`, only
`tests::a_request_that_overlaps_itself_is_refused` failed at the reversed
adjacent spans. With the mirrored-selector guard replaced by `false`, only
`tests::extended_flags_reject_reserved_bits_and_meaningless_selectors` failed
at flags `0x0001`. Both production changes were restored before the registry
edits. The ordinary gate proves that each replacement still matches once; the
next clean complete sweep remains PW-001's closing evidence.
