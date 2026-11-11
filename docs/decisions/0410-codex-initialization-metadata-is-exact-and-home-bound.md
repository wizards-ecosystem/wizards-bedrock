# D-410 Codex initialization metadata is exact and home-bound

Status: Accepted, 2026-10-15. Amends D-406.

**Decision.** The reader admits exactly the
`userAgent`, `codexHome`, `platformFamily` and `platformOs` metadata instead of
the formerly empty object. It requires the
reported Codex home to be the same ordinary or Windows-verbatim spelling as the
private projected home, requires both platform values to identify Windows and
requires the user agent to bind the qualified Codex version and exact Deep
client identity while permitting the bounded printable host-version segment.
Missing, extra, foreign-home, foreign-platform and foreign-client metadata
remain refused. The effective-config schema and every D-406 credential,
sandbox, runtime, source and artifact check remain independently enforced.
Mutation 2,064 removes the private-home comparison and fails
`config_response_correlation_and_transport_fail_closed`.

**Rationale.** Accepting any initialization object would lose forward schema
drift detection and the proof that the reader uses the private credential
projection. The complete Windows build string is host metadata rather than
qualified executable identity, so pinning it would make routine OS servicing
look like a Codex security boundary change.
