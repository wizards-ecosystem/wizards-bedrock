# D-328 Deleted public APIs earn closure through exact compiler reintroduction

Status: Accepted (2026-09-27), closes PW-519, PW-523 and D-136

**Context.** D-275 made the public-API audit compiler-semantic and explicitly
left the real PW-519 and PW-523 reintroduction runs unclaimed. Both APIs had
already been deleted: `path_walk::is_relative` duplicated a weaker fragment of
the checked path policy, while `TrapCause::is_resumable` exposed a speculative
policy that no production caller owned. D-136 could not close until each
retained mutation proved that the audit rejects precisely the deleted surface.

**Decision.** A deleted public API closes its audit disposition only after its
exact retained reintroduction is compiled alone, the structured audit verdict
names exactly that item, the source is restored byte-for-byte and the complete
clean public graph resolves every item to a caller. A source search or a green
audit before the red run is insufficient because neither proves that the
retained defect still reaches the current compiler graph.

**Evidence.** Reintroducing `is_relative` expanded the graph from 1,723 to
1,724 items; `cargo xtask api-audit` exited with only `fn is_relative` at
`libs/path-walk/src/lib.rs:181`. After byte-identical restoration,
reintroducing `TrapCause::is_resumable` again produced 1,724 items and exited
with only `method impl::is_resumable` at `libs/trap-cause/src/lib.rs:105`.
The final byte-identical restoration audited all 1,723 public items with every
caller compiler-resolved. The retained register mutation moves with PW-519's
closed hostile-surface evidence so the disposition check itself remains able
to fail.

**Alternatives.** Adding tests for either deleted API was rejected because it
would preserve the surface under review. Treating the mutation registry entry
as execution evidence was rejected because applicability proves only that its
source replacement still matches. Combining both reintroductions in one audit
was rejected because a multi-item verdict would not isolate either causal red.

**Amended by D-341.** This decision re-anchored D-325's retained `OPEN`-segment
mutation, so it also amends D-325.
