# D-303 Redundant capture bounds need phase-specific witnesses

Status: Accepted, 2026-09-12. Amends D-264 and D-301; advances PW-001.

**Context.** Complete-sweep mutations 870 and 871 replaced the caller's child
output limit with the global ceiling during polling and final collection. Both
survived one integrated fixture: while either correct check remained, it
rejected the same 4 KiB output against the 32-byte caller limit. D-264 requires
both phases because a live child must be stopped before it fills the capture,
and a child observed exiting still must not allocate beyond its caller's bound.

**Decision.** Keep the shared limit at both phases and make their observations
independent. The noisy child now flushes its output and remains live, so its
test requires the polling observer to report the 32-byte overflow rather than
eventually report a timeout. Final collection is routed through a small private
helper used by the production call and its retained-handle fixture, so that
fixture reaches collection without a live polling phase.

**Proof.** Substituting the global limit in the polling closure made
`a_caller_specific_capture_limit_is_polled_while_the_child_is_live` wait to its
two-second deadline and fail its overflow assertion. After restoration,
substituting the global limit in the collection helper made
`captured_output_cannot_grow_collection_memory_past_its_limit` accept six bytes
against a five-byte limit. Both exact tests pass after restoration, and the
registry source and check-name proofs pass. The remaining complete-report
outcomes and a clean successor sweep stay in PW-001.
