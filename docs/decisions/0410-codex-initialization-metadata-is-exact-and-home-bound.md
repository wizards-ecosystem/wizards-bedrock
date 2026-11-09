# D-410 Codex initialization metadata is exact and home-bound

Status: Accepted (2026-10-15), amends D-406

**Context.** After D-409 admitted the current login-status stream, the owned
Deep qualifier reached its effective-configuration reader but rejected the
qualified Codex app-server's initialization result. The runtime now returns
`userAgent`, `codexHome`, `platformFamily` and `platformOs` metadata instead of
the formerly empty object. No access advisory or Deep scan had started.

**Decision.** The reader admits exactly those four fields. It requires the
reported Codex home to be the same ordinary or Windows-verbatim spelling as the
private projected home, requires both platform values to identify Windows and
requires the user agent to bind the qualified Codex version and exact Deep
client identity while permitting the bounded printable host-version segment.
Missing, extra, foreign-home, foreign-platform and foreign-client metadata
remain refused. The effective-config schema and every D-406 credential,
sandbox, runtime, source and artifact check remain independently enforced.

**Proof.** The fake app-server transcript test exercises the current metadata
shape and rejects extra fields, a foreign Codex home, a foreign platform and a
foreign client identity. Mutation 2,064 removes the private-home comparison and
fails `config_response_correlation_and_transport_fail_closed`.

**Alternatives.** Restoring acceptance of any initialization object was
rejected because it would lose both forward schema drift detection and proof
that the reader actually uses the private credential projection. Pinning the
complete Windows build string was rejected because it is host metadata rather
than qualified executable identity and would make routine OS servicing look
like a Codex security boundary change.
