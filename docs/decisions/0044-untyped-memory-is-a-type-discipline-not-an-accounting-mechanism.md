# D-44 Untyped memory is a type discipline, not an accounting mechanism

Status: Accepted (2026-08-22)

**Context.** `002-architecture.md` has carried an open question since the first
research pass: *is seL4-style untyped-retype — callers donating memory for
kernel objects — worth adopting to kill kernel-memory DoS?* `007-memory.md`
made answering it a Phase 3 obligation, because the answer shapes every
kernel-object API after this one.

`008-research-notes-2.md` §3.2 found that the question conflates two different
ideas that share a word.

**seL4's untyped/retype is an accounting mechanism, and it is expensive in
three separate ways.** Each untyped capability carries a single watermark, moved
first to the new object's alignment and then past its size — so creating a
4 KiB object and then a 16 KiB one strands the 12 KiB between them until *both*
children are revoked. The community critique goes further: fixing the
allocation policy inside the kernel contradicts seL4's own principle of keeping
policy out of the kernel, because the kernel will refuse a gap that would
satisfy the alignment. And `003-research-notes.md` already recorded that
revocation is long-running and needs abortable sub-operations.

**Asterinas's untyped memory is a type discipline, and it carries.** Its
premise: safe Rust references cannot uphold their guarantees for *externally
modifiable* memory — user-mapped, DMA-capable, or MMIO — so such memory gets a
separate type admitting only plain-old-data copies. The consequence is
structural rather than procedural: the user address space accepts *only*
untyped frames, so mapping kernel memory into userspace is not a check that
could be forgotten but a signature that will not typecheck. The DMA
abstractions take the same argument, so no memory is DMA-accessible until a
driver asks for untyped memory specifically.

**Decision.** Reject seL4-style untyped-retype as the kernel-memory accounting
mechanism. Adopt Asterinas-style untyped memory as a type discipline, arriving
with its first real caller in Phase 5. Answer kernel-memory DoS at the handle
layer with per-process quota accounting.

**Rationale.** The quota is a *policy* decision and stays revisable; making
callers donate memory is an *ABI* decision and is permanent. Everything
untyped-retype buys can be had from a counter attached to a job, and the
fragmentation, the revocation machinery and the policy-in-kernel objection all
go away with it.

The discipline is the half that must be early, because it is the half that
cannot be retrofitted — which is `002` pillar 5's whole argument and
Asterinas's own stated lesson. It costs nothing today: `wost::mm` hands out
`Frame` and `Segment`, and the untyped variants join them when something can
map a frame into an address space that is not the kernel's.

**Consequences.** `002-architecture.md`'s open question is closed, and the
naming collision is recorded here so it is not re-litigated: "untyped" in this
kernel means *externally modifiable, POD-copy only*, never *donated for kernel
object creation*. Phase 5's handle-table doc inherits the quota obligation.
`wost::mm`'s module documentation names the absent split and why it is absent.
