# D-300 Qualification fixtures own their source-control premise

Status: Accepted, 2026-09-10. Amends D-208; closes PW-032.

**Context.** The clean mutation sweep on 1ddef80 passed the repaired linker
boundary, then its pristine host baseline failed in
`streamed_model_tree_digest_preserves_the_length_delimited_schema`. That test
used its compile-time repository path and asked Git for the real `models/`
inventory. D-208 deliberately runs tests from a tracked-file copy with no
`.git`; the test therefore required authority the isolated source does not
contain, even though its subject was only equivalence between streamed and
legacy digest encodings.

**Decision.** The compatibility test creates a uniquely named temporary Git
repository, writes two differently named and sized model inputs, stages them,
and compares the production streamed digest with its independent in-memory
encoding there. Production model qualification still discovers the real
tracked tree from the clean repository and retains every source-bound check.

Skipping the test outside a Git checkout was rejected because the mutation
baseline would then prove less than ordinary CI. Copying `.git` into D-208's
workspace was rejected because mutable source-control state is not a build
input and would weaken isolation. Continuing to borrow the live repository was
rejected because a unit fixture must own every premise needed to reproduce its
claim.

**Proof.** The real isolated host baseline failed with "no tracked model inputs
were found" while ordinary CI passed. The private two-file repository passes
from the ordinary checkout. Replacing the streamed path-length frame with zero
reddened its exact digest comparison before restoration; the existing defect
remains mutation 877. The complete current sweep remains PW-001.
