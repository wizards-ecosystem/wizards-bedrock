# D-248 The shipped object surface stops at one opaque bootstrap identity

Status: Accepted, 2026-08-30. Amends D-95, D-110, D-111, D-127, D-130, D-181 and D-217; advances PW-532.

**Decision.** The current `Channel` is one opaque, counted identity. `spawn`
creates exactly one, installs it as the child's sole handle and creates no peer
or transport state. The `Channel` tag remains because the current bootstrap
ABI and the future Wizzy handoff need a concrete typed identity; removing the
tag would either break that birth contract or substitute an untyped placeholder.
Bytes, transferred handles, waiting, close and pairing return only when the
post-gate bootstrap caller fixes their protocol. `Event` is removed. The
direct `WaitQueue::wait_unless` lost-wakeup proof remains because production
ring waiting uses that primitive independently of either speculative object.

Channel has no type-specific rights. The child's initial entry still carries
the universal rights, and ring-3 `Duplicate` requests `INSPECT`, not a fictional
channel `READ`. Queue owns the one implemented reservation/commit delegation
path; duplicate HandleTable convenience operations are removed. Test weights,
fault injection, resource counters, birth/death snapshots and inspection APIs
compile only under `selftest`. `Spawned` and `Birth` correlate through the
authority table's existing non-repeating identity rather than an invented
object peer. Every retained raw self-test address space reclaims its tables on
every setup refusal and verifies the exact frame-ledger baseline. All
`expect(dead_code)` attributes are gone: a surface is shipped because a caller
uses it or is explicitly self-test-only.

**Rationale.** The pre-Wizzy caller audit asked D-184's question of every
kernel surface: what concrete current caller forces this mechanism? `Event`,
channel messages, send/receive, backpressure, blocking, peer-close and
HandleTable send/accept/close/duplicate were reached only by self-tests or by
prose about a future userland, which made their own tests look like demand;
queue `Duplicate`, the only shipped handle operation, already owned
reservation, attenuation and completion. Production `spawn` created a weakly
joined peer only so `Spawned` and `Birth` could prove pairing, so every shipped
process began with an already-orphaned fictional endpoint. Several test-only
weights, seams, ledgers and snapshots were compiled into the shipped kernel,
and raw self-test `AddressSpace` setup refusals could abandon page-table frames
while purporting to prove cleanup.

**Alternatives.** Keeping mechanisms because tests exercised them was rejected:
it reverses the caller/dependency direction and freezes a pre-Wizzy protocol.
Keeping the weak peer only under `selftest` was rejected because the pairing
claim itself described no production invariant. Deleting `Channel` entirely
was rejected because the one typed bootstrap entry is a current machine and
handoff contract. Replacing queue delegation with the removed wrappers was
rejected because it would move ownership away from the only concrete caller.

**Evidence and consequences.** The machine suite drops the tests for deleted
behavior. Retained mutations corrupt the exact birth-table identity, widen
Channel's meaningful rights, bypass the WaitQueue condition recheck, erase the
test weight and skip raw-space reclaim. Target checks, API audit, Clippy and
restored debug/release boot evidence remain PW-532 acceptance work; historical
Phase 5 channel and Event sections remain design history but no longer claim
to inventory the current kernel. No unsafe operation was added.
