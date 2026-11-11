# D-294 The audited registry is rebound once per compiler round

Status: Accepted, 2026-09-06. Amends D-263 and D-275.

**Decision.** `compile_graph` still rebinds the copied and original source
trees after every graph, before any diagnostic is read. The registry is
rebound once after the pristine round and once after every instrumented
round, before that round's target libraries are rechecked and before any
verdict is derived. Cargo metadata discovery keeps its own registry proof.

**Rationale.** The API audit materializes the locked registry into a private
Cargo home and proves it unchanged by hashing every file it holds. D-275's
implementation repeated that proof after every compiler graph, each proof
reading 6,853 files and 119 MB. The source copy and the original tree stay
cheap to rebind, but the registry proof alone cost more per round than the
compilation it guarded.

**Why this loses nothing.** Neither placement observes a change made and
reverted inside one compilation; both detect a change that persists to the
next proof. Under either placement no verdict is derived from diagnostics
whose inputs failed a proof, because a failed proof ends the audit. What the
per-round proof changes is the number of graphs whose diagnostics are
discarded together when tampering is found, which is not a property the
verdict depends on.

**Proof.** The retained copied-source and original-source rebind mutations
were re-anchored to the new guard block and still red their exact shape
tests; a new mutation deleting the per-round registry proof reds the shape
test that requires it before the round's target-library recheck.
