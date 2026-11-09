# D-254 APIC initialization owns exclusion, not the caller's entry flag

Status: Accepted, 2026-08-30. Amends D-33, D-35 and D-49; advances PW-126.

**Context.** The hardware-free lifecycle model rejected an IF-enabled caller,
while production deliberately disables interrupts and restores the exact entry
state. That stricter model contradicted the behavior PW-126 requires and could
hide a broken restoration path.

**Decision.** Beginning depends only on `Cold` lifecycle state. The privileged
transaction disables interrupts itself, retains that exclusion through every
hardware stage and readiness publication, then restores the saved entry flag.
Losing exclusion during progress poisons the controller; every late refusal
remains unpublished and terminal.

**Evidence.** The 82 APIC unit tests and doctest pass, including every ordered
stage, exclusion loss and late refusal. Removing cold-state admission failed
the exact host test. The release machine passed atomic initialization,
deferred-delivery and exact-entry-IF checks within its 247 checks.
