# D-293 Every static lock in `wost` is reclaimed by the panic path, and a contract says so

Status: Accepted, 2026-09-06. Amends D-41 and D-67.

**Context.** `wost::panic::reclaim` force-unlocks the locks a panic may have
interrupted so the reporter can use them. Its comment said that every lock in
a `static` inside the crate belongs in that list and that nothing but the
comment would catch an omission. Three statics had already been omitted: the
paging module's retained-stack ledgers and the PCI configuration lock. The
reporter does not take them today, so no panic deadlocked, but the obligation
was a comment and the comment was false.

**Decision.** Each `wost` module that owns a static `SpinLock` or
`ThreadLock` exposes one `force_unlock(&PanicToken)` that reclaims every such
static, and `panic::reclaim` calls each module's door. A source contract in
`xtask/src/contracts.rs` parses every `wost` source file, collects the static
locks per module, and requires the module door to name each of them and the
panic path to name each module. Adding a static lock without extending the
door, or a door without extending the panic path, fails the host gate.

**Proof.** Two retained mutations delete one module door call and one static
from a door; each reds only the contract. The reclaim order is unchanged and
force-unlock remains sound for the reason `SpinLock::force_unlock` states:
the interrupted holder never resumes.
