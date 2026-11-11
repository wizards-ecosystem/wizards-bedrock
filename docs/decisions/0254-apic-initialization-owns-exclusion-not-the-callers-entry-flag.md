# D-254 APIC initialization owns exclusion, not the caller's entry flag

Status: Accepted, 2026-08-30. Amends D-33, D-35 and D-49; advances PW-126.

**Decision.** Beginning depends only on `Cold` lifecycle state. The privileged
transaction disables interrupts itself, retains that exclusion through every
hardware stage and readiness publication, then restores the saved entry flag.
Losing exclusion during progress poisons the controller; every late refusal
remains unpublished and terminal.

**Rationale.** The hardware-free lifecycle model rejected an IF-enabled
caller, while production deliberately disables interrupts and restores the
exact entry state, so the stricter model contradicted the behavior PW-126
requires and could hide a broken restoration path.

**Evidence.** Host tests cover every ordered stage, exclusion loss and late
refusal, and removing cold-state admission fails the exact host test. Machine
checks cover atomic initialization, deferred delivery and the exact entry IF.
