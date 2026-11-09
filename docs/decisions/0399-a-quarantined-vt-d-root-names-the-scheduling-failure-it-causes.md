# D-399 A quarantined VT-d root names the scheduling failure it causes

Status: Accepted (2026-10-13), amends D-358 and D-395; advances PW-001

**Context.** Permanent mutation 1,555 programs VT-d `RTADDR` with the
context-table address and accepts that same wrong value on readback. Its
historical witness was the later check `the unit walks the kernel's root
table`. The early fabric boundary added after that evidence independently
rechecks `RTADDR` through `boundary_active`, classifies the already-programmed
unit as quarantined, retains every hardware owner and table frame, refuses DMA
retargeting and bus-master authority, and returns no interrupt-isolation proof.
`start_interrupts` consequently leaves maskable interrupts disabled. Self-test
still starts, but its bound-space witness waits for a newly runnable task by
spinning rather than yielding; without timer preemption that task cannot retire.
Two bounded observations therefore reached the exact panic `the bound-space
witness did not retire` before the historical root-table check.

**Decision.** Keep every production refusal and grade mutation 1,555 by that
earliest causal panic. Its reason binds the complete chain from wrong live
`RTADDR`, through quarantined VT-d ownership and refused interrupt admission,
to the non-retiring scheduled task. The successful-boot root-table readback
check remains unchanged as a positive runtime diagnostic; its later position
does not override a stronger earlier fail-closed verdict.

**Proof required.** Applying mutation 1,555 alone must repeat the exact
bound-space panic through the ordinary release self-test observer, then restore
the source byte for byte. The panic registry contract, exact mutation
application, named-check inventory and ordinary gate must pass. The repaired
registry must participate in the next complete clean sweep.

**Alternatives.** Weakening `boundary_active`, admitting interrupts for a
quarantined unit, yielding inside the bound-space witness or reordering checks
solely to recover the historical marker was rejected because each would make a
security refusal less direct or would hide its real runtime consequence.
Calling the result unrelated flakiness was rejected because the same exact
panic repeated twice and follows deterministically from the missing timer
preemption. Rewriting the dated D-358 evidence was rejected; it remains true of
the earlier source, while this decision records the later causal ordering.
