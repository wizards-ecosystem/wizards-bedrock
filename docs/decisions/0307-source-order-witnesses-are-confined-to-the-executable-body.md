# D-307 Source-order witnesses are confined to the executable body

Status: Accepted, 2026-09-12. Amends D-244 and D-301; advances PW-001.

**Decision.** The order witness first isolates the exact `mutate::run` body
between its public declaration and the next item, then requires each lifecycle
call to occur exactly once inside that body before comparing offsets. The
non-load-bearing mutation is replaced by removing the production-body anchor,
which admits registry literals and directly violates the stated invariant.

**Rationale.** The test searched all of `mutate.rs`, and the persistent
registry necessarily quotes production source, so a mutation that moved one
`begin_mutation` match into a registry fixture still left the offsets ordered:
the assertion accepted a sequence assembled from two source regions.

**Alternatives.** Escaping or renaming registry strings was rejected because
persistent mutations must continue quoting exact source. Retaining `rfind`
over the whole file was rejected because incidental item order, rather than an
executable-body boundary, would remain part of the proof.
