# D-303 Redundant capture bounds need phase-specific witnesses

Status: Accepted, 2026-09-12. Amends D-264 and D-301; advances PW-001.

**Decision.** Keep the shared limit at both phases and make their observations
independent. The noisy child now flushes its output and remains live, so its
test requires the polling observer to report the overflow rather than
eventually report a timeout. Final collection is routed through a small private
helper used by the production call and its retained-handle fixture, so that
fixture reaches collection without a live polling phase.

**Rationale.** Two survivors replaced the caller's child output limit with the
global ceiling during polling and final collection, and one integrated fixture
let either correct check reject the same output. D-264 requires both phases
because a live child must be stopped before it fills the capture, and a child
observed exiting still must not allocate beyond its caller's bound.
