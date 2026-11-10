# D-44 Untyped memory is a type discipline, not an accounting mechanism

Status: Accepted, 2026-08-22.

**Context.** `002-architecture.md` has carried an open question since the
first research pass: *is seL4-style untyped-retype, with callers donating
memory for kernel objects, worth adopting to kill kernel-memory DoS?*
`007-memory.md` made the answer a Phase 3 obligation, because it shapes
every kernel-object API after this one.

`008-research-notes-2.md` §3.2 found that the question conflates two ideas
that share a word.

**seL4's untyped/retype is an accounting mechanism, and it is expensive in
three separate ways.** Each untyped capability carries one allocation
cursor, moved to the new object's alignment and then past its size. A
4 KiB object then a 16 KiB one strands the 12 KiB between them until *both*
children are revoked. The community critique goes further: fixing the
allocation policy in the kernel contradicts seL4's own rule of keeping
policy out of it, because the kernel will refuse a gap that would satisfy
the alignment. And `003-research-notes.md` already recorded that revocation
is long-running and needs abortable sub-operations.

**Asterinas's untyped memory is a type discipline, and it carries.** Safe
Rust references cannot uphold their guarantees for *externally modifiable*
memory (user-mapped, DMA-capable, or MMIO), so such memory gets a separate
type admitting only plain-old-data copies. The consequence is structural.
The user address space accepts *only* untyped frames, so mapping kernel
memory into userspace is a type error, not a check that could be forgotten.
The DMA abstractions take the same argument: no memory is DMA-accessible
until a driver asks for untyped memory specifically.

**Decision.** Reject seL4-style untyped-retype as the kernel-memory
accounting mechanism. Adopt Asterinas-style untyped memory as a type
discipline, arriving with its first caller in Phase 5. Answer kernel-memory
DoS at the handle layer with per-process quota accounting.

**Rationale.** The quota is a *policy* decision and stays revisable; making
callers donate memory is an *ABI* decision and is permanent. A counter on a
job buys everything untyped-retype buys, and the fragmentation, revocation
machinery and policy-in-kernel objection all go with it.

The discipline is the half that must be early, because it cannot be
retrofitted: `002` pillar 5's argument and Asterinas's own stated lesson. It
costs nothing today. `wost::mm` hands out `Frame` and `Segment`, and the
untyped variants join them when something can map a frame into an address
space that is not the kernel's.

**Consequences.** `002-architecture.md`'s open question is closed. The
naming collision is recorded so it is not re-litigated: "untyped" in this
kernel means *externally modifiable, POD-copy only*, never *donated for
kernel object creation*. Phase 5's handle-table doc inherits the quota
obligation. `wost::mm`'s module documentation names the absent split and
why.
