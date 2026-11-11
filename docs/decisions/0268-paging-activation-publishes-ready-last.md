# D-268 Paging activation publishes `Ready` last

Status: Accepted, 2026-08-31. Amends D-86 and D-261; advances PW-127.

**Decision.** Activation is one `Cold -> Initializing -> Ready` transaction.
It refuses a reclaimed space before reading its root, acquires and retains
interrupt exclusion, writes CR3, publishes the active root, and publishes
`Ready` last with Release ordering. Every observer requires `Ready` before using
the root. An uncommitted software preflight aborts back to `Cold` while it still
owns interrupt exclusion; after CR3 is written, no fallible step remains before
publication.

**Rationale.** Activation stored a ready flag before CR3 and the active-root
identity became authoritative. A safe observer could authenticate a mapping
through half-published state, and reclaimed-space refusal read the root before
proving the space still owned one.

**Alternatives and current evidence.** Publishing the flag early and relying on
one CPU was rejected because interrupt handlers are observers on that CPU.
Reading the root before reclaimed-state validation was rejected because the
ownership precondition must dominate even a harmless read. Source contracts
pin CR3/root/readiness order, while registered early-ready, root-order,
interrupt-exclusion and reclaimed-order mutations name their exact witnesses.
The current restored machine run remains PW-127's close.
