# D-06 Native ABI: capability handles + async submission/completion queues

Status: Verified (handle model) (amended by D-181)

**Context.** Isaac directed the project to be "forward thinking, state of
the art, lightweight end to end, ahead of our time, matching theoretical
frameworks and 2026 standards." The original plan was POSIX-ish syscalls.

**Options.** (a) POSIX-native syscalls; (b) async + capability core with
POSIX as a translation layer; (c) sync now, async ABI v2 later.

**Decision.** (b), reversing the earlier POSIX-ish choice. Every kernel
object is reached through a process-local handle carrying rights bits;
the data-path ABI is shared-memory submission/completion rings; a small
synchronous syscall set covers setup, handle operations, mapping, and
waits. Any handle-less operation that creates external authority was reserved
for a per-process **policy layer**.

D-181 narrows that early routing sketch: the current synchronous domain is
Exit, RingEnter, RingWait and ThreadBase. Duplicate is ring traffic, channels
are planned there, and mapping waits for a safe allocation/execution contract
rather than inheriting a speculative call number. No current handle-less call
creates external object authority, so no policy object is implemented for the
enumerated ring, time and control facilities.

**Rationale, amended 2026-08-23 by `010` §2.** The original entry contrasted
batching and cache locality with trap avoidance. FlexSC does not support that
contrast: batching reduces both direct mode-switch cost and indirect cache
cost. The larger result is structural asynchrony; eliminating the trap is a
smaller additional gain and can cost a dedicated core. Zircon's handle-centric
authorization remains verified as Fuchsia's deployed design; the proposed
policy escape hatch mirrors Zircon Job policy (verified caveat). Capsicum independently
converged on capabilities + no global namespaces (unverified tier).

**Ring-ABI rules adopted from io_uring's security record** (Google
kernelCTF disables io_uring; ~60% of exploit submissions targeted it —
unverified tier): explicit version fields in ring structures (IoRing's
improvement), minimal opcode surface, no privileged opcodes, defensive
validation of everything read from shared memory.

**Consequences.** More design work before first userspace (handle table,
rights taxonomy, queue layout — each gets a doc). POSIX becomes a
userspace translation library rather than the kernel's shape. WASI's authority
model aligns, but D-181 records the real descriptor-adapter cost: numbering,
preopens and cursor state.
