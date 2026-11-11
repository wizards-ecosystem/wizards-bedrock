# D-44 Untyped memory is a type discipline, not an accounting mechanism

Status: Accepted, 2026-08-22.

**Decision.** Reject seL4-style untyped-retype as the kernel-memory
accounting mechanism. Adopt Asterinas-style untyped memory as a type
discipline, arriving with its first caller in Phase 5. Answer kernel-memory
DoS at the handle layer with per-process quota accounting.

The discipline: externally modifiable memory (user-mapped, DMA-capable or
MMIO) gets a separate type admitting only plain-old-data copies. The user
address space accepts *only* untyped frames, so mapping kernel memory into
userspace is a type error, not a check that could be forgotten, and no
memory is DMA-accessible until a driver asks for untyped memory
specifically.

**Rationale.** The quota is a *policy* decision and stays revisable; making
callers donate memory is an *ABI* decision and is permanent. A counter on a
job buys everything untyped-retype buys without its stranded fragments,
revocation machinery and policy in the kernel (`008-research-notes-2.md`
§3.2). The discipline must be early because it cannot be retrofitted (`002`
pillar 5); it costs nothing until something can map a frame into an address
space that is not the kernel's.

**Consequences.** `002-architecture.md`'s open question is closed. The
naming collision is recorded so it is not re-litigated: "untyped" in this
kernel means *externally modifiable, POD-copy only*, never *donated for
kernel object creation*. Phase 5's handle-table doc inherits the quota
obligation. `wost::mm`'s module documentation names the absent split and
why.
