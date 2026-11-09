# D-414 Paging self-test ownership is measured in one exclusion epoch

Status: Accepted, 2026-10-15. Amends D-413.

**Context.** D-413's full CI run passed release self-test 290/290 but the debug
kernel reproducibly failed `an inactive lower-root unmap preserves the live
observer`. Instrumented evidence showed that every translation, invalidation,
unmap and reclaim assertion passed. Only the global allocator snapshot differed:
the service worker had retired older zombies between this fixture's setup and
teardown, so unrelated cleanup looked like an ownership imbalance.

**Decision.** This one machine check now holds a nested-safe interrupt exclusion
from its allocator baseline through local teardown and the final snapshot. The
test still requires distinct leaves, a warmed live translation, no `invlpg` for
the inactive root, an unchanged observer, complete unmap and reclaim, and exact
allocator ownership. Failure diagnostics name each sub-proof and both ownership
snapshots. Production paging behavior is unchanged.

**Proof.** The unchanged debug image failed the same check twice. Its diagnostic
run proved the paging sub-check true and only ownership false while background
objects disappeared. With one exclusion epoch around the fixture, the debug
image passed all 290 checks, including the paging and subsequent clock checks.

**Alternatives.** Removing the allocator conservation assertion was rejected
because it would stop detecting fixture leaks. Retrying the check or accepting
the release-only pass was rejected because either would hide nondeterminism
rather than separating local ownership from concurrent cleanup.
