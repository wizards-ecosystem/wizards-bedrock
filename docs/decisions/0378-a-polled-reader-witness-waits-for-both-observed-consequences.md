# D-378 A polled-reader witness waits for both observed consequences

Status: Accepted, 2026-10-10. Closes PW-587.

**Context.** The exact-final-tree `cargo xtask ci` run reached release QEMU
with 279 of 280 checks passing. The sole failure was `the blocked reader woke
with the injected key through polling`; an earlier run of the same executable
code passed 280 of 280. The witness took a background-poller pass count before
injection, but its bounded wait ended as soon as the blocked reader produced a
key. The foreground `await_armed_key` path is itself allowed to poll and wake
that reader. When it did so before the permanent 10 ms poller completed its
next round, the witness sampled the pass count once and failed correct behavior
according to scheduler timing.

**Decision.** Retain the existing one-second bound and every production
predicate. The wait now ends only when both observations are true: the reader
has produced a result and the background-poller pass count has advanced beyond
the pre-injection snapshot. Only then does the witness require the exact `A`
key, proof that the reader blocked, and zero dropped events. No production
input, scheduling or polling behavior changes.

**Proof required.** The host source contract must bind the conjunctive bounded
wait so a later edit cannot restore the sequential sample. A retained mutation
must restore the early-ending reader-only wait and fail that contract. Restored
release and debug machine checks must each pass all 280 checks inside the full
gate.

**Evidence.** The old sequential witness produced the exact 279/280 failure
above while its adjacent poller-live, injected-key, editing and translated-key
checks all passed. The source contract and mutation 1,768 provide the retained
structural red. Final restored machine and integrated-gate results are recorded
by the closing commit.

**Alternatives.** Retrying the gate would hide a known observation race.
Removing the pass-count predicate would stop proving that the permanent poller
advanced after the snapshot. Sleeping before the assertion would encode a
schedule rather than an outcome. Waiting for the existing conjunction preserves
the intended evidence without extending the deadline or production surface.
