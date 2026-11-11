# D-399 A quarantined VT-d root names the scheduling failure it causes

Status: Accepted, 2026-10-13. Amends D-358 and D-395; advances PW-001.

**Decision.** Keep every production refusal and grade mutation 1,555, which
programs VT-d `RTADDR` with the context-table address and accepts that wrong
value on readback, by the earliest causal panic `the bound-space witness did not
retire`. Its reason binds the complete chain from wrong live `RTADDR`, through
quarantined VT-d ownership and refused interrupt admission, to the
non-retiring scheduled task. The successful-boot root-table readback check
remains unchanged as a positive runtime diagnostic; its later position does not
override a stronger earlier fail-closed verdict.

**Rationale.** The early fabric boundary rechecks `RTADDR` through
`boundary_active`, quarantines the unit and returns no interrupt-isolation
proof, so `start_interrupts` leaves maskable interrupts disabled, and the
bound-space witness spins rather than yields, so without timer preemption its
task cannot retire. The same panic repeated twice and follows
deterministically, so it is not flakiness. Weakening `boundary_active`,
admitting interrupts for a quarantined unit, yielding inside the witness or
reordering checks to recover the historical witness would make a security
refusal less direct or hide its runtime consequence. Rewriting the dated D-358
evidence was rejected; it remains true of the earlier source, while this
decision records the later causal ordering.
