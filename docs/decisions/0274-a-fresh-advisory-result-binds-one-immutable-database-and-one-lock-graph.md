# D-274 A fresh advisory result binds one immutable database and one lock graph

Status: Accepted, 2026-08-31. Amends D-220, D-244 and D-262; advances PW-028.

**Decision.** Advisory qualification remains exceptional and outside ordinary
CI. It first resolves and digests `cargo-audit-audit 0.22.2`, invalidates the
old pass, creates an empty private database location, and performs one bounded
population run from the exact RustSec URL. The resulting Git revision, tree,
origin and clean status become immutable authority for a second bounded
`--no-fetch` audit of the exact repository `Cargo.lock` under the committed
empty-ignore policy and warning denial.

Only canonical JSON with a nonempty database and dependency graph, that exact
database revision, empty target filters, null severity, no ignored advisories,
the expected informational-warning policy, zero vulnerabilities and zero
dependency warnings is a pass. The record binds both commands, the source,
lockfile, policy, tool, database identity, UTC observation and bounded
stdout/stderr transcript. Evidence schema v5 carries those exact artifacts and
reconstructs the semantic result offline.

**Rationale.** D-220 required a fresh RustSec review but did not bind freshness
to one immutable database checkout. A default audit may fetch while it runs,
and human output does not carry the complete result schema. An old pass can
also outlive a changed lockfile, policy, tool, source revision or advisory
database unless starting a new attempt invalidates it.

**Alternatives and current evidence.** Fetching during the final audit was
rejected because the result would not name one stable database. A digest without
semantic JSON validation, human summaries, ignored advisories and networked CI
were rejected because each weakens or obscures the claim. Host fixtures and
retained mutations prove the mechanism's source, path, bound, schema and
replay contracts only. PW-028 remains required until an unmodified clean
closing commit runs the real tool against a fresh database and the resulting
bundle verifies offline; no fresh advisory result is claimed here.
