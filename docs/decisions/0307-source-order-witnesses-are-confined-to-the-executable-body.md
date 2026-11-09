# D-307 Source-order witnesses are confined to the executable body

Status: Accepted, 2026-09-12. Amends D-244 and D-301; advances PW-001.

**Context.** Complete-sweep mutation 85 changed the mutation-lifecycle order
test from its last `begin_mutation` marker to its first, but survived. The test
searched all of `mutate.rs`; the persistent mutation registry necessarily
quotes production source, so the first marker belonged to a registry fixture.
The other markers still came from the executable function, and their numeric
offsets remained ordered. The assertion therefore accepted a sequence assembled
from two different source regions.

**Decision.** The order witness first isolates the exact `mutate::run` body
between its public signature and the next item, then requires each lifecycle
marker to occur exactly once inside that body before comparing offsets. The
non-load-bearing mutation is replaced by removing the production-body anchor,
which admits registry literals and directly violates the stated invariant.

Escaping or renaming registry strings was rejected because persistent mutations
must continue quoting exact source. Retaining `rfind` over the whole file was
rejected because incidental item order, rather than an executable-body boundary,
would remain part of the proof.

**Proof.** Replacing the production-body offset with zero made
`exceptional_tasks_commit_only_after_their_terminal_verdict` find three
`begin_mutation` markers and fail its uniqueness requirement. With the anchor
restored, all five lifecycle markers are unique and ordered. The replacement
mutation matches once and its named check exists; the remaining report outcomes
and a clean successor sweep stay in PW-001.
