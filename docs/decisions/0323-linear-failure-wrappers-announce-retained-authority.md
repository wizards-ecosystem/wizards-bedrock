# D-323 Linear failure wrappers announce retained authority

Status: Accepted, 2026-09-14. Advances PW-548.

**Decision.** Every public foundation struct whose name ends in `Failure` is
`#[must_use]`. Its diagnostic names the cleanup authority, retry state or
reservation at risk. A recursive syntax contract covers kernel, library and
`wost` source so new wrappers cannot silently omit the policy. The rule is
diagnostic rather than reclamation: callers still must consume the returned
authority explicitly.

**Rationale.** Public failure wrappers often carry the only token that can
retry cleanup or release a fail-stopped reservation, and twelve of fourteen
did not announce that dropping one deliberately retains ownership.

**Alternatives.** Relying only on `Result` was rejected because a caller can
extract an error before discarding it. Adding destructors was rejected because
several wrappers intentionally expose their owner by value and fail-stop
leakage remains the safe terminal behavior. A retained mutation removing one
annotation keeps the source contract load-bearing.
