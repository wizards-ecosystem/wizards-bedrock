# D-06 Native ABI: capability handles + async submission/completion queues

Status: Verified (handle model), 2026-07-18. Amended by D-181.

**Decision.** An async capability core with POSIX as a translation layer,
reversing the original POSIX-ish syscall plan. Every kernel object is
reached through a process-local handle carrying rights bits. The data-path
ABI is shared-memory submission/completion rings; a small synchronous
syscall set covers setup, handle operations, mapping and waits. Any
handle-less operation creating external authority was reserved for a
per-process **policy layer**.

D-181 narrows that early routing sketch: the current synchronous domain is
Exit, RingEnter, RingWait and ThreadBase. Duplicate is ring traffic,
channels are planned there, and mapping waits for a safe
allocation/execution contract, not a speculative call number. No current
handle-less call creates external object authority, so the enumerated ring,
time and control facilities have no policy object.

**Ring-ABI rules**, from io_uring's security record: explicit version fields
in ring structures, minimal opcode surface, no privileged opcodes, defensive
validation of everything read from shared memory.

**Rationale, amended 2026-08-23 by `010` §2.** The gain is structural
asynchrony, not trap avoidance: batching cuts both direct mode-switch and
indirect cache cost (FlexSC). Zircon's handle-centric authorization is
Fuchsia's deployed design, and the policy layer mirrors Zircon Job policy.

**Consequences.** POSIX becomes a userspace translation library, not the
kernel's shape. WASI's authority model aligns, but D-181 records the
descriptor-adapter cost: numbering, preopens and cursor state.
