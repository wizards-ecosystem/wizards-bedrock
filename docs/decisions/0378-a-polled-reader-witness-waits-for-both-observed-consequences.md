# D-378 A polled-reader witness waits for both observed consequences

Status: Accepted, 2026-10-10. Closes PW-587.

**Decision.** Retain the existing one-second bound and every production
predicate. The wait ends only when both observations are true: the reader has
produced a result and the background-poller pass count has advanced beyond the
pre-injection snapshot. Only then does the witness require the exact `A` key,
proof that the reader blocked, and zero dropped events. No production input,
scheduling or polling behavior changes. A host source contract binds the
conjunctive bounded wait, and mutation 1,768 restores the early-ending
reader-only wait and fails that contract.

**Rationale.** `the blocked reader woke with the injected key through polling`
failed on correct behavior: the foreground `await_armed_key` path may itself
poll and wake the reader, and when it did so before the permanent 10 ms poller
completed its next round, the witness sampled the pass count once and failed
according to scheduler timing. Retrying the gate would hide a known observation
race, dropping the pass-count predicate would stop proving that the permanent
poller advanced, and sleeping before the assertion would encode a schedule
rather than an outcome.
