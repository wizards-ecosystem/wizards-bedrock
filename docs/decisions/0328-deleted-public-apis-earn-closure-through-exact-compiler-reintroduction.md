# D-328 Deleted public APIs earn closure through exact compiler reintroduction

Status: Accepted, 2026-09-27. Closes PW-519, PW-523 and D-136.

**Decision.** A deleted public API closes its audit disposition only after its
exact retained reintroduction is compiled alone, the structured audit verdict
names exactly that item, the source is restored byte-for-byte and the complete
clean public graph resolves every item to a caller. A source search or a green
audit before the red run is insufficient because neither proves that the
retained defect still reaches the current compiler graph.

**Rationale.** D-275 made the public-API audit compiler-semantic but left the
PW-519 and PW-523 reintroduction runs unclaimed, and D-136 could not close
until each retained mutation proved that the audit rejects precisely the
deleted surface: `path_walk::is_relative` duplicated a weaker fragment of the
checked path policy, and `TrapCause::is_resumable` exposed a speculative
policy no production caller owned. Reintroduced alone, they made
`cargo xtask api-audit` name only `fn is_relative` at
`libs/path-walk/src/lib.rs:181` and only `method impl::is_resumable` at
`libs/trap-cause/src/lib.rs:105`.

**Alternatives.** Adding tests for either deleted API was rejected because it
would preserve the surface under review. Treating the mutation registry entry
as execution evidence was rejected because applicability proves only that its
source replacement still matches. Combining both reintroductions in one audit
was rejected because a multi-item verdict would not isolate either causal red.

**Consequences.** The retained register mutation moves with PW-519's closed
hostile-surface evidence so the disposition check itself remains able to fail.

**Amended by D-341.** This decision re-anchored D-325's retained `OPEN`-segment
mutation, so it also amends D-325.
