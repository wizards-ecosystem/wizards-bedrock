# D-06 Native ABI: capability handles + async submission/completion queues

Status: Verified (handle model), 2026-07-18. Amended by D-181.

**Context.** Isaac directed the project to be "forward thinking, state of
the art, lightweight end to end, ahead of our time, matching theoretical
frameworks and 2026 standards." The original plan was POSIX-ish syscalls.

**Options.** (a) POSIX-native syscalls; (b) async and capability core with
POSIX as a translation layer; (c) sync now, async ABI v2 later.

**Decision.** (b), reversing the POSIX-ish choice. Every kernel object is
reached through a process-local handle carrying rights bits. The data-path
ABI is shared-memory submission/completion rings; a small synchronous
syscall set covers setup, handle operations, mapping and waits. Any
handle-less operation creating external authority was reserved for a
per-process **policy layer**.

D-181 narrows that early routing sketch: the current synchronous domain is
Exit, RingEnter, RingWait and ThreadBase. Duplicate is ring traffic, channels
are planned there, and mapping waits for a safe allocation/execution
contract, not a speculative call number. No current handle-less call
creates external object authority, so the enumerated ring, time and control
facilities have no policy object.

**Rationale, amended 2026-08-23 by `010` §2.** The original entry contrasted
batching and cache locality with trap avoidance. FlexSC does not support
that: batching cuts both direct mode-switch and indirect cache cost. The
larger result is structural asynchrony; removing the trap adds less and can
cost a dedicated core. Zircon's handle-centric
authorization remains verified as Fuchsia's deployed design; the proposed
policy escape hatch mirrors Zircon Job policy (verified caveat). Capsicum
independently converged on capabilities and no global namespaces
(unverified tier).

**Ring-ABI rules adopted from io_uring's security record.** Google kernelCTF
disables io_uring; ~60% of exploit submissions targeted it (unverified
tier). The rules: explicit version fields in ring structures (IoRing's
improvement), minimal opcode surface, no privileged opcodes, defensive
validation of everything read from shared memory.

**Consequences.** More design before first userspace (handle table, rights
taxonomy, queue layout, each with a doc). POSIX becomes a userspace
translation library, not the kernel's shape. WASI's authority model aligns,
but D-181 records the descriptor-adapter cost: numbering, preopens and
cursor state.
