# D-249 Device refusal is transactional, and internal contradictions fail in every build

Status: Accepted (2026-08-30), amends D-37, D-41, D-48, D-115, D-133 and D-206; verification tracked by PW-533

**Context.** Several safe hardware boundaries returned refusal and their
callers discarded it. Framebuffer reads substituted zero and writes updated
the RAM shadow even when checked MMIO changed no device byte. The i8042 path
continued after a failed flush or controller command, so a later read could
consume stale output; configuration writes were neither admitted nor verified.
Queue and time-page helpers likewise ignored impossible shared-page bounds.

The same policy split by build mode. Queue reservations, handle commitments,
page-table indices and TSS alignment supported permanent ownership or unsafe
pointer preconditions, yet `debug_assert!` removed them from release kernels.
Some syscall answers, test cleanup, the NVMe timeout repair, the shipped tmpfs
survey and the synthetic degraded PCI claim also discarded their results.
Those are not graceful fallbacks: they leave a stale register, false evidence,
or divergent ownership while execution continues.

**Decision.** A framebuffer lease is checked at construction; any later MMIO
refusal contradicts that geometry and fails stopped before the shadow changes.
Framebuffer scroll arithmetic is a checked `libs/pixel` row plan. Each visible
source row is published through one bounds-checked MMIO slice before the same
span commits to the RAM shadow; the device boundary performs only volatile
word writes. This removes the raw copy shortcut and avoids copying padding
while retaining the pitch as the stride.
The i8042 controller admits every required command, verifies each required
configuration byte and types refusal as `TransportRefused` or
`ConfigurationRefused`. All six operations that begin by draining output stop
immediately when draining fails. The redundant best-effort set-2 command is
removed: reset already establishes the protocol default, while arriving bytes
remain the decoder's arbiter under D-239.

Internal queue, handle, heap, page-table, TSS, framebuffer-shadow, time-page and
shared-memory bounds use release assertions. Operation-style boolean APIs and
the time-page publication outcome are `#[must_use]`; predicates remain ordinary
booleans. A source contract keeps that distinction explicit and another rejects
known discarded or saturating cleanup results. Every returning syscall uses one
answer helper which records refusal and then fails stopped. NVMe invokes its
cleanup primitive directly, tmpfs reports enumeration failure, the keyboard's
one-shot interrupt handoff is asserted, and the degraded image must prove it
reserved PCI configuration.

**Alternatives.** Treating MMIO refusal as a blank pixel was rejected because
it made the shadow lie. Continuing an i8042 query after command refusal was
rejected because there is no way to attribute the next buffered byte. Silent
release-only recovery was rejected under D-206: a missing promised slot or an
unsafe bound contradiction has no coherent state to return to. Making every
boolean `#[must_use]` was rejected because predicates are routinely and safely
queried for control flow; only operations with a refusal outcome carry it.

**Evidence and consequences.** Three machine checks inject configuration,
flush and transport refusal while requiring exact live-state restoration.
Framebuffer qualification shortens both the validated lease and nonempty
shadow by one word and names each terminal marker. Host row-plan checks cover
12,400 small geometry/request combinations plus
distinct padded-stride and short-shadow cases. Copying padding, advancing by
visible width, accepting downward copies and omitting the source bound each
reddened its named host check before restoration. Heap corruption mutations
name slot, frame and direct-map provenance failures. Every target graph checks,
the integrated release suite passed 218/218 before the final result-audit
edits, and all 505 retained mutation patterns match exactly. PW-533 closes on
the restored debug, release and degraded boots. No ABI changed.
