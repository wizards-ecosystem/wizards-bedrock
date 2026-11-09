# D-336 APIC initialization and completion each have one linear transaction

Status: Accepted, 2026-09-28. Amends D-254, D-257, D-267 and D-313; closes PW-123 and PW-126.

**Context.** A non-duplicable `EoiToken` did not by itself prove that one
delivery could mint only one token or that abandoning one returned the live
claim. Likewise, the APIC lifecycle model and exact SVR readback did not by
themselves prove that production kept readiness unpublished through the whole
interrupt-excluded initialization transaction or that every late refusal
remained terminal. Closure required current causal machine evidence for those
ownership and publication edges, plus compiler evidence that completion
authority cannot be cloned.

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

**Evidence.** On 2026-09-28 retained mutations 736, 773, 774, 771, 850 and
1,174 were applied one at a time and restored exactly. Bypassing the EOI latch
failed only `one delivery mints one completion authority` at 261/262. Forgetting
an abandoned token failed `an abandoned completion can be reclaimed for the
same delivery`, then reached the complete terminal panic `interrupts became
enabled inside an exclusion`. Leaving the latch claimed in `Drop` produced the
complete early panic `vector 0x41 has no matching APIC in-service authority`.
Adding `Clone` made the hostile `EoiToken<'static>` compiler probe succeed and
failed the exact linear-authority contract.

Publishing `Ready` in the initial compare-exchange failed both
`local apic initialization stayed unpublished and interrupt-excluded` and
`every late or ignored-write apic refusal stays unavailable and terminal` at
260/262. Returning a failed transaction to `Cold` failed only the latter at
261/262. D-313 already records current mutation 849, which restored interrupts
before the Release publication and failed the exact ordered-publication
contract. D-257 records current mutation 851, the independent omitted-SVR-
readback break which failed only the ignored-write machine witness.

Exact restoration passed all 82 `x86-apic` unit tests and its doctest, the
linear-authority, typed-policy, ordered-publication and explicit-interrupt-arm
contracts, and all 1,412 mutation applicability and named-check registrations.
Restored release QEMU passed 262/262, including exact init exclusion, EOI
uniqueness, abandoned-token recovery, terminal refusal, deferred delivery and
successful interrupt progress.

**Alternatives.** Relying on a non-`Copy` token alone was rejected because its
mint can still duplicate a live delivery and the type can later gain `Clone`.
Leaking an abandoned claim was rejected because it converts a recoverable live
interrupt into permanent exclusion. Returning failed initialization to `Cold`
was rejected because possibly changed hardware cannot authorize a fresh
transaction. Publishing readiness after restoring interrupts was rejected
because an interrupt handler is a concurrent observer on the same CPU.
