# D-234 A terminal observation keeps exact failures that preceded it

Status: Accepted (2026-08-29), amends D-209 and D-232; advances PW-001

**Context.** The complete 278-entry diagnostic successor to D-233 ran every
mutation on clean commit `8de1d4de9c3740617999cfef9ac564b35f706f48` in
56 minutes 30 seconds. It found no survivor and caught 274 entries exactly.
Mutation 14 twice failed unrelated ring-3 checks before its late trap-logging
trigger. Mutations 123 through 125 each made their own ownership check red,
then the deliberately corrupted state caused a later page fault at
`0xffffffffffffffe8`. The serial observer had already recorded those named
failures, but `SelftestOutcome::Panicked` discarded them and exposed only the
later panic. Retrying reproduced the information loss rather than resolving
it.

**Decision.** A terminal self-test outcome carries both its exact panic
transcript and every named check that failed before the panic. A check mutation
is caught if its own named failure is present, even when later consequences
stop the kernel before its verdict. A different earlier failure remains
inconclusive. A panic mutation still requires its exact panic marker; an
earlier check cannot stand in for it.

The unknown-syscall adversary runs before the long stateful ring-3 group. It
has no dependency on an earlier user program and waits for its own retirement,
so placing it first gives the trap-context logging mutation its exact terminal
assertion before unrelated later state can obscure it.

**Alternatives.** Crediting any panic was rejected because three ownership
mutations and one unrelated fault would become indistinguishable. Discarding
the named check because a later panic occurred was rejected because it throws
away the exact causal observation D-209 requires. Adding more retries was
rejected because both observations already contained the same evidence.
Masking the later page fault in each check was rejected because corrupted
ownership is allowed to have further terminal consequences after its exact
check has reported it.

**Consequences.** The unmutated reordered image passed all 215 release checks.
Restoring trap-context logging then stopped at the first adversarial user
program with exactly `a trap handler may not take a thread lock`. Discarding
the pre-panic failures failed only the serial-outcome test, and refusing to
credit the exact retained name failed only the mutation-assessment test.
Mutations 279 and 280 retain those two observer defects. No unsafe operation,
wire ABI or machine-check count changed.
