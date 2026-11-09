# D-323 Linear failure wrappers announce retained authority

Status: Accepted (2026-09-14), advances PW-548

**Context.** Public failure wrappers often carry the only token that can retry
cleanup or release a fail-stopped reservation. The returned `Result` warned
when ignored, but the extracted failure value did not consistently announce
that dropping it deliberately retains ownership. Two wrappers had the warning;
twelve equivalent wrappers did not.

**Decision.** Every public foundation struct whose name ends in `Failure` is
`#[must_use]`. Its diagnostic names the cleanup authority, retry state or
reservation at risk. A recursive syntax contract covers kernel, library and
`wost` source so new wrappers cannot silently omit the policy.

**Alternatives and evidence.** Relying only on `Result` was rejected because a
caller can extract an error before discarding it. Adding destructors was
rejected because several wrappers intentionally expose their owner by value and
fail-stop leakage remains the safe terminal behavior. Removing one annotation
failed the exact source contract; the retained mutation keeps that rule
load-bearing. The rule is diagnostic rather than reclamation: callers still
must consume the returned authority explicitly.
