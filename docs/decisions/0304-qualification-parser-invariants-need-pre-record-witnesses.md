# D-304 Qualification parser invariants need pre-record witnesses

Status: Accepted, 2026-09-12. Amends D-274, D-297 and D-301; advances PW-001.

**Decision.** Keep the integrated hostile-report matrix and add one direct
semantic-report witness before record binding. It supplies an otherwise valid,
revision-bound fetched report with an advisory count of zero and requires the
specific empty-database refusal. The retained mutation names that exact test,
`an_empty_advisory_database_cannot_author_a_qualification`.

**Rationale.** The integrated forged-report fixture also left its authored
record claiming one advisory, so the later record comparison refused it even
when the parser accepted an empty database. It proved the whole verifier
failed, not that an empty population result could never author a matching
record.
