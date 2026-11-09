# D-309 Semantic parser witnesses precede later shape validation

Status: Accepted, 2026-09-12. Amends D-244, D-297 and D-301; advances PW-001.

**Context.** Complete-sweep mutations 448, 508 and 509 weakened three semantic
rejections but survived integrated parser tests. A duplicate evidence key was
still rejected later because rendering its map could not reproduce two input
lines. The TLC fixtures inserted an unrelated error or Java failure between a
behavior marker and its first state, so exact trace-shape validation rejected
them even when the corresponding error classifier was removed. Each broad
error assertion therefore proved only that some later contract remained.

**Decision.** Give each semantic rejection a witness whose input satisfies
every later structural contract. Duplicate-key insertion is tested directly on
`Record` before manifest rendering. The unrelated-error and runtime-failure
witnesses place one hostile line in an otherwise valid clean TLC transcript;
clean verdicts have no behavior trace whose adjacency the added line could
disturb. Each retained mutation names its dedicated test.

Removing the integrated hostile-input cases was rejected because they still
exercise the complete parser path. Relaxing canonical rendering or trace shape
for a test was rejected because a witness must isolate one contract without
weakening another production defense.

**Proof.** Allowing duplicate replacement made
`a_duplicate_evidence_key_is_rejected_before_canonical_rendering` receive
success. Removing the unrelated-error classifier made
`an_unrelated_tlc_error_is_rejected_before_verdict_shape_validation` classify
the hostile transcript as clean. Accepting every runtime-failure predicate made
`a_tlc_runtime_failure_is_rejected_before_verdict_shape_validation` do the
same. All three exact tests pass after restoring their guards; the remaining
complete-report outcomes and a clean successor sweep stay in PW-001.
