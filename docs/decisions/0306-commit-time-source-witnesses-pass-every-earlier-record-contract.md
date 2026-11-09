# D-306 Commit-time source witnesses pass every earlier record contract

Status: Accepted (2026-09-12), amends D-244 and D-301; advances PW-001

**Context.** Complete-sweep mutation 47 removed the final source recheck before
a qualification record is renamed into authority, but its named test survived.
The model-record schema had gained six transcript artifacts since that fixture
was authored. Its handcrafted record omitted all six, so `validate_record`
refused it before the test could dirty the source and reach the mutated check.
A broad error assertion credited the earlier schema failure.

**Decision.** The fixture authors a complete current model record, including
every registered transcript artifact, while retaining the real commit and tree
captured from its private Git repository. After changing a tracked file, it
requires the specific stable-clean-checkout error from the immediate pre-rename
source capture. Production source binding and the retained mutation are
unchanged.

Weakening record validation for this test was rejected because a commit-time
witness must first satisfy every contract that precedes commit. Checking only
that no final record exists was rejected because any early return could satisfy
that consequence without exercising source identity.

**Proof.** With the final `require_current_source` call removed exactly as
mutation 47 specifies, `commit_record` returned success and
`source_is_rechecked_immediately_before_a_record_commits` failed at its required
error. With the guard restored, the complete fixture reaches and reports the
specific dirty-checkout refusal. The registry source and check-name proofs pass;
the remaining complete-report outcomes and a clean successor sweep stay in
PW-001.
