# D-395 Mutation completion witnesses exercise reachable behavior and the earliest causal verdict

Status: Accepted, 2026-10-13. Amends D-322 and D-394; advances PW-001.

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

Applying 202, 324, 605, 610 and 660 one at a time produces respectively the
named queue panic, loss of the degraded filesystem record, keyboard handoff
check failure, CPU authentication panic and immutable-generation host-test
failure. A complete sweep with the repaired registry identity must finish
with no survivor or inconclusive result before release publication.

**Rationale.** The restarted sweep on `ed2641f` exposed five distinct registry
problems rather than five equivalent escapes. Mutation 202 now reaches the
queue's terminal reserved-slot invariant instead of its old timeout. Mutation
324 conditioned the filesystem survey on PCI availability, which D-390 makes
true on every admitted machine, so it was equivalent. Mutation 605 removes a
latch the qualified QEMU topology never exercises because the controller stays
polled. CPU authentication refuses mutation 610's corrupted TCG identity before
its old degraded-boot record. Mutation 660's shared test failed an earlier
VMX-source check without reaching receipt integrity. Only 605 and 660 lacked an
independent witness; the others were stale or unreachable classifications that
broad acceptance must not hide. Inspecting production source text from a test
was rejected because spelling is not behavior; marking the global latch during
a polling self-test would falsify the routing state; routing the keyboard on
the qualified machine would change the interrupt-isolation profile to serve a
test; and accepting any panic or failed check would count an unrelated failure
as coverage.
