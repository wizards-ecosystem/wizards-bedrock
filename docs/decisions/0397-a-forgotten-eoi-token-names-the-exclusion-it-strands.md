# D-397 A forgotten EOI token names the exclusion it strands

Status: Accepted, 2026-10-13. Amends D-322 and D-395; advances PW-001.

**Decision.** Mutation 773, which forgets the first ping delivery's `EoiToken`
instead of dropping it before reclaim, uses `Caught::Panic` with the exact
interrupt-ledger panic `interrupts became enabled inside an exclusion`. The
reason records the complete causal chain: forgetting the EOI token strands its
exclusion, trap return restores IF, and the next claim detects the impossible
enabled-inside-exclusion state. No arbitrary panic or earlier unrelated failed
check is accepted.

**Rationale.** The token owns both the single live EOI claim and the nested
interrupt-exclusion state, so both bounded observations reached that panic
before the registry's later abandoned-completion count check; the mutation
fails closed and its old witness was causally too late. Treating the panic as an
inconclusive harness failure would discard a stronger production invariant,
and weakening the ledger or repairing the forgotten token in self-test would
hide the ownership defect the mutation creates.
