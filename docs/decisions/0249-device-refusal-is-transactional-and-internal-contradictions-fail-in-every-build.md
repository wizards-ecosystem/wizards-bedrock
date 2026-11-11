# D-249 Device refusal is transactional, and internal contradictions fail in every build

Status: Accepted, 2026-08-30. Amends D-37, D-41, D-48, D-115, D-133 and D-206; verification tracked by PW-533.

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

**Rationale.** Safe hardware boundaries returned refusal and their callers
discarded it: framebuffer reads substituted zero and writes updated the shadow
when no device byte changed, and i8042 continued after a failed flush or
command so a later read could consume stale output. Queue reservations, handle
commitments, page-table indices and TSS alignment supported permanent ownership
or unsafe pointer preconditions, yet `debug_assert!` removed them from release
kernels. Those are not graceful fallbacks: they leave a stale register, false
evidence, or divergent ownership while execution continues.

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
small geometry/request combinations plus distinct padded-stride and
short-shadow cases, and catch copying padding, advancing by visible width,
accepting downward copies and omitting the source bound. Heap corruption
mutations name slot, frame and direct-map provenance failures. PW-533 closes on
the restored debug, release and degraded boots. No ABI changed.
