# D-395 Mutation completion witnesses exercise reachable behavior and the earliest causal verdict

Status: Accepted, 2026-10-13. Amends D-322 and D-394; advances PW-001.

**Context.** The restarted complete sweep on `ed2641f` exposed five distinct
registry problems rather than five equivalent escapes. Mutation 202 corrupts
the shared submission layout and now reaches the queue's terminal reserved-slot
invariant instead of its old timeout. Mutation 324 conditions the hardware-free
filesystem survey on PCI availability, but D-390 now refuses a machine without
PCI configuration before memory initialization, so every admitted execution
makes that condition true and the mutation is equivalent. Mutation 605 removes
the keyboard's interrupt-handoff latch, while the qualified QEMU topology
deliberately leaves that controller in polling mode and never exercises the
transition. Mutation 610 corrupts the QEMU TCG identity and is refused by CPU
authentication before the aggregate degraded-boot marker it used to name.
Mutation 660 removes the captured VMware receipt comparison, but its shared
test mixed a generated VMX first and therefore failed an earlier VMX-source
check without reaching receipt integrity. A survivor is a missing independent
witness only for 605 and 660; the other results are stale or unreachable
classifications which must not be hidden by broad acceptance.

**Decision.** The keyboard latch claim is factored into one private production
helper parameterized by `Once`. The routed path calls that helper with the
singleton latch; self-test calls the same helper with a fresh local latch and
requires the exact check `the keyboard interrupt handoff records its
ownership`. It does not forge a routed machine or change the singleton on a
polling boot. Mutation 605 removes the helper's claim and names that check.

Mutation 202 names the exact existing panic `a reserved completion slot was
not there`. Mutation 610 names the exact earlier panic `CPU state does not
authenticate a qualified DMA-handoff platform`. Mutation 324 is retained under
its permanent identity but removes the unconditional filesystem survey rather
than adding a condition that every admitted machine satisfies; the exact
degraded filesystem record remains its witness. The VMware generation test
also mixes an opaque machine image whose size and all independent structure
remain valid, so only the receipt-to-captured-digests equality can reject it;
mutation 660 keeps that exact test as its witness.

**Proof required.** The machine inventory contains 282 checks. Applying 202,
324, 605, 610 and 660 one at a time must produce respectively the named queue
panic, degraded filesystem-marker loss, keyboard handoff check failure, CPU
authentication panic and immutable-generation host-test failure, then restore
each isolated source byte for byte. The ordinary gate must pass after the
repairs. Because the running `ed2641f` sweep uses an immutable pre-repair
snapshot, a new complete sweep with the repaired registry fingerprint must
finish with no survivor or inconclusive result before release publication.

**Alternatives.** Inspecting production source text from a test was rejected
because spelling is not behavior. Marking the global keyboard latch during a
polling self-test was rejected because it would falsify the machine's routing
state. Forcing the qualified QEMU machine to route the keyboard was rejected
because it changes the interrupt-isolation profile merely to serve a test.
Accepting any panic or any failed check was rejected because an unrelated
failure is not mutation coverage. Keeping mutation 324's unreachable predicate
was rejected because no test can distinguish equivalent behavior on an
admitted machine. Mixing VMX bytes alone was rejected because it proves the
earlier source-equivalence guard, not the receipt boundary mutation 660 removes.
