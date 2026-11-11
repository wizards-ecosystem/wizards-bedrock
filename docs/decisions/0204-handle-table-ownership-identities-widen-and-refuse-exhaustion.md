# D-204 Handle-table ownership identities widen and refuse exhaustion

Status: Accepted, 2026-08-28. Amends D-78 and D-132; closes PW-402.

**Decision.** A table identity is a non-wrapping `u64`. The arena owns the
next identity beside quota commitment, under the same leaf lock, so reserving
the identity and committing the table's complete quota is one transaction.
`u64::MAX` is the exhausted-state sentinel and is never issued; construction
returns typed `IdentityExhausted` without changing commitment when no successor
exists. The checked successor is pure arithmetic in `libs/handle-space`, where
the maximum boundary is host-testable.

The 32-bit userspace handle format does not change. `Mixer::for_table` derives
from the identity's low 32 bits, so mixers may repeat after 2^32 tables; D-132
already makes the mixer an accidental-value defence rather than authority.
The full 64-bit identity remains on every arena entry and is the ownership
comparison, so equal mixers cannot cross tables.

**Rationale.** The arena authorises a decoded handle with one comparison,
`entry.owner == table.id`, and the identity had been a wrapping `u32`, so after
four billion constructions an old table's handle could become authority in a
new one. This is independent of the deliberately wrapping 12-bit slot
generation: a generation limits stale-handle detection for one reused slot,
while the table identity is the isolation boundary between principals, and the
Wizzy process manager will create tables for the life of the system. Refusing
wrap makes the security invariant structural: no live or dead identity is ever
issued again. Widening before refusal makes exhaustion unreachable under any
plausible uptime without pretending infinity, and the sentinel avoids a second
flag or a wider atomic the target does not provide. Keeping identity
allocation inside `ARENA` also avoids a quota rollback path and prevents two
constructors from observing the same last value.

**Alternatives.** Saturating the old `u32` was rejected because process churn
can make four billion a system-lifetime concern and a permanent refusal should
not be that close. Reusing identities after scanning for live owners was
rejected because a dead table's leaked handle value could become valid again,
and the scan would turn construction into a global lifetime protocol. Random
identities were rejected because collision refusal still needs a registry and
entropy is not uniqueness. Widening the handle or mixer was rejected because
neither is the authority check and both would spend ABI bits without fixing
ownership.

**Consequences.** The host boundary check proves the penultimate state advances
to the sentinel and the sentinel has no successor. Release QEMU holds the last
constructible table and an event handle live, observes two typed exhaustion
refusals with unchanged arena commitment, resolves the original handle
afterwards, and restores the exact baseline on teardown. Wrapping the
production successor and separately stopping only its final advance each
redden only `table identities never wrap or repeat`; three persistent mutations
keep the boundary.
