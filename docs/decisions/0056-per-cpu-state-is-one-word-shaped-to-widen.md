# D-56 Per-CPU state is one word, shaped to widen

Status: Accepted (2026-08-22)

**Context.** One CPU today; SMP is a later phase. The scheduler needs "which
thread is on this CPU", and how that is stored decides whether SMP is a
widening or a rewrite.

**Decision.** `wost::task::CURRENT` is a single atomic word, read and written
only under the task table's lock with the flag clear. Everything that is
per-CPU in principle — the current slot, the trap depth, the interrupt
hand-off flag — is a `static` today and is named as such in its doc comment,
so the SMP phase has a list rather than a hunt. GS base and `swapgs` stay
where D-29 left them, waiting for ring 3.

**Rationale.** A per-CPU abstraction designed before there is a second CPU
would guess at what it needs (APIC IDs, stack switching on entry, IPIs) and
be wrong in the details. Keeping the statics few and named is the cheap form
of the same intent.

**Consequences.** The SMP phase owes: `CURRENT` and `TRAP_DEPTH` become
per-CPU cells, `prepare`'s single-CPU exclusivity argument becomes a per-CPU
one, and D-45's TLB-shootdown debt comes due the moment anything unmaps.
