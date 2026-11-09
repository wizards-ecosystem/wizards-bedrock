# D-304 Qualification parser invariants need pre-record witnesses

Status: Accepted (2026-09-12), amends D-274, D-297 and D-301; advances PW-001

**Context.** Complete-sweep mutation 26 removed the nonempty advisory-database
check but survived the integrated forged-report test. That fixture changed its
transcript from one advisory to zero while its already-authored qualification
record still claimed one, so the later record comparison rejected the mismatch
even when the semantic parser accepted the empty database. The test proved the
whole verifier failed, but not that an empty population result could never
author a matching record.

**Decision.** Keep the integrated hostile-report matrix and add one direct
semantic-report witness before record binding. It supplies an otherwise valid,
revision-bound fetched report with an advisory count of zero and requires the
specific empty-database refusal. The retained mutation names that exact test.

**Proof.** With the nonempty guard replaced by the registry defect,
`an_empty_advisory_database_cannot_author_a_qualification` failed because
`verify_advisory_report` returned `AdvisoryFacts` with a zero count. After
restoration, the exact test and the registry source/check-name proofs pass. The
remaining complete-report outcomes and a clean successor sweep stay in PW-001.
