# D-397 A forgotten EOI token names the exclusion it strands

Status: Accepted, 2026-10-13. Amends D-322 and D-395; advances PW-001.

**Context.** Permanent mutation 773 forgets the first ping delivery's
`EoiToken` instead of dropping it before reclaim. The token owns both the
single live EOI claim and the nested interrupt-exclusion state. Returning from
the trap re-enables the saved interrupt flag without unwinding that forgotten
exclusion. On the next ping, `claim_eoi` disables interrupts again and the
exclusion ledger observes that interrupts had become enabled at nonzero depth.
Both bounded observations therefore reached the exact panic `interrupts became
enabled inside an exclusion` before the registry's later abandoned-completion
count check. The mutation fails closed; its old check witness was causally too
late.

**Decision.** Mutation 773 uses `Caught::Panic` with that exact interrupt-ledger
marker. The reason records the complete causal chain: forgetting the EOI token
strands its exclusion, trap return restores IF, and the next claim detects the
impossible enabled-inside-exclusion state. No arbitrary panic or earlier
unrelated failed check is accepted.

**Proof required.** Applying mutation 773 must repeat the exact terminal panic
through the ordinary release self-test observer, restore the isolated source
byte for byte and record `caught`. The repaired registry must then participate
in the complete clean sweep required by PW-001.

**Alternatives.** Keeping the later check witness contradicts two repeatable
terminal observations. Treating the panic as an inconclusive harness failure
would discard a stronger production invariant. Weakening the exclusion ledger
or forcibly repairing a deliberately forgotten token in self-test was rejected
because either would hide the ownership defect the mutation creates.
