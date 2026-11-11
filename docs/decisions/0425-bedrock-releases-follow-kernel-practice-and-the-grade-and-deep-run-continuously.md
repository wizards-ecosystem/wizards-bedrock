# D-425 Bedrock releases follow kernel practice and the grade and Deep run continuously

Status: Accepted, 2026-10-19. Amends D-330, D-373 and D-406; supersedes PW-025 and PW-026 as release prerequisites.

**Decision.** Bedrock adopts Linux-kernel-style release practice. The v0.1.0
gate is `cargo xtask ci`, a complete curated `cargo xtask mutate` sweep,
`cargo xtask model-check`, the RustSec `cargo xtask qualify-advisory`
qualification, both human VMware qualifications (self-test and shipped), and
`cargo xtask evidence` followed by `cargo xtask verify-evidence` on the exact
qualified commit. That commit is tagged directly: D-330's `same-commit`
relation is the planned close. Its `parent-docs-only-v1` child stays supported
code, but no release plans on it.

The independent grade and the owned Deep scan become continuous, non-blocking
campaigns against `main`. Their tools, ownership and PASS authority are
unchanged: only `qualify-mutants-grade --run` and `qualify-security-deep --run`
can create those records. A release bundle verifies a campaign strictly when
its record is present for the bundle's exact source and otherwise declares it
deferred. Evidence advances to `wizard-evidence-v11`; qualification records
stay `wizard-qualification-v6`. The manifest always carries
`qualification.cargo-mutants-grade.status` and
`qualification.codex-security-deep.status`, each exactly `pass` or `deferred`.
Its expected key set is a fixed function of those two values: `pass` requires
every bundle file of that campaign and verifies its record exactly as before,
including D-406's owned Deep authority on both the online and offline paths,
while `deferred` admits none of its keys and refuses a bundle whose campaign
directory still carries a record. A missing or other status is refused rather
than defaulted. Only those two kinds are deferrable; the mutation, advisory,
model and both VMware records stay mandatory. Evidence now takes the release
source from the mutation record rather than the Deep record. A record present
for another source still fails, so a stale campaign result must be rerun for
the release source or removed, never silently deferred.

v0.1.0 is a release, not a foundation close. Register rows the gate proves are
closed in the commit after the tag; every other row stays live and the release
lists it as a known open item.

**Consequences.** PW-025 and PW-026 are superseded as release prerequisites,
with their history retained; their campaigns continue against `main`, and a
passing record still enters the release whose source it names. PW-009's closing
bundle is the v11 same-commit bundle. Evidence v10 manifests are refused.

Permanent mutations 2,262 through 2,271 cover the new checks, and mutations
1,744 and 1,907 keep their identities while targeting the guarded Deep calls.
On 2026-10-19 each named witness passed on the pristine source and failed under
its mutation: mutations 2,262, 2,263 and 2,264, which drop the deferral check, accept the
record it observed or never observe a present record, red
`a_deferred_campaign_carrying_a_record_is_refused`; mutation 2,265, which drops
a passing campaign's files, reds
`only_a_deferred_campaigns_files_leave_the_bundle`; mutations 2,266 and 2,267,
which treat a passing campaign as deferred or make the model deferrable, red
`only_the_grade_and_deep_campaigns_are_deferrable`; mutation 2,268, which reads
any other status as deferred, reds
`campaign_status_is_exactly_pass_or_deferred`; mutation 2,269, which defaults
an omitted status, reds `an_omitted_campaign_status_is_refused_not_defaulted`;
and mutations 1,744, 1,907, 2,270 and 2,271, which remove or invert the offline
and online Deep guards, red
`online_and_offline_release_paths_share_the_owned_deep_verifier`. These are
implementation checks; only the complete current-registry sweep on the release
commit qualifies them.

**Rationale.** Under D-330, D-373 and D-406 every qualification was bound to
one frozen commit and the independent grade and owned Deep scan were mandatory
release inputs, yet each is a multi-day or externally authenticated campaign
whose prerequisites (provider runtime tuples, account access, a several-day
host reservation) move independently of the kernel source. Every source repair
restarted both, so their external dependencies, not kernel defects, set the
release cadence, and no release had been cut. Omitting a campaign's keys when
absent would make absence indistinguishable from a manifest that lost them, so
`deferred` keeps the schema exact; a record from an older `main` commit would
qualify a source the release does not ship; and a direct tag of the qualified
commit needs no relation beyond the identity evidence already verifies.
