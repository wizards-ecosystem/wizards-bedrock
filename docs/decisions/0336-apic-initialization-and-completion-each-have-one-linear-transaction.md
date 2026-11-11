# D-336 APIC initialization and completion each have one linear transaction

Status: Accepted, 2026-09-28. Amends D-254, D-257, D-267 and D-313; closes PW-123 and PW-126.

**Decision.** One in-service delivery may latch one exact completion claim.
The token consumes the live frame, vector, interrupt-exclusion and highest-ISR
observations before EOI; consuming it completes once, while dropping it
releases the latch for the same still-live delivery. `EoiToken` remains neither
`Copy` nor `Clone`.

APIC initialization remains one `Cold -> Initializing -> Ready` transaction.
It owns the caller's exact entry interrupt state, keeps dependent vectors and
register access unpublished until every ordered hardware stage and exact SVR
readback succeeds, publishes `Ready` with Release ordering while interrupts
are still excluded, and restores the saved interrupt state last. Every late or
ignored-write refusal publishes terminal poison rather than making the
controller retryable.

**Rationale.** A non-duplicable `EoiToken` alone did not prove that one
delivery mints one token or that abandoning one returns the live claim, and
the lifecycle model and exact SVR readback alone did not prove that
production kept readiness unpublished through the whole interrupt-excluded
transaction or that every late refusal stays terminal.

**Evidence.** Retained mutations 736, 773, 774 and 1,174 bypass the latch,
forget or keep an abandoned claim and add `Clone`; mutations 771 and 850
publish `Ready` early and return a failed transaction to `Cold`. Each fails its
exact machine, panic or compiler witness. D-313 records mutation 849, which
restored interrupts before the Release publication, and D-257 records mutation
851, the omitted SVR readback.

**Alternatives.** Relying on a non-`Copy` token alone was rejected because its
mint can still duplicate a live delivery and the type can later gain `Clone`.
Leaking an abandoned claim was rejected because it converts a recoverable live
interrupt into permanent exclusion. Returning failed initialization to `Cold`
was rejected because possibly changed hardware cannot authorize a fresh
transaction. Publishing readiness after restoring interrupts was rejected
because an interrupt handler is a concurrent observer on the same CPU.
