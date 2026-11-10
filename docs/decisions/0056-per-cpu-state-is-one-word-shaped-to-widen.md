# D-56 Per-CPU state is one word, shaped to widen

Status: Accepted, 2026-08-22.

**Context.** One CPU today; SMP is a later phase. The scheduler needs "which
thread is on this CPU", and how that is stored decides whether SMP is a
widening or a rewrite.

**Decision.** `wost::task::CURRENT` is one atomic word, read and written
only under the task table's lock with the flag clear. Everything per-CPU in
principle (the current slot, the trap depth, the interrupt hand-off flag) is
a `static` today, named as such in its doc comment, so the SMP phase has a
list instead of a hunt. GS base and `swapgs` stay where D-29 left them,
waiting for ring 3.

**Rationale.** A per-CPU abstraction designed before a second CPU would
guess at its needs (APIC IDs, stack switching on entry, IPIs) and get the
details wrong. Few, named statics are the cheap form of that intent.

**Consequences.** The SMP phase owes: `CURRENT` and `TRAP_DEPTH` become
per-CPU cells, `prepare`'s single-CPU exclusivity argument becomes a per-CPU
one, and D-45's TLB-shootdown debt comes due once anything unmaps.
