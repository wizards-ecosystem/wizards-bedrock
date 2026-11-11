# D-414 Paging self-test ownership is measured in one exclusion epoch

Status: Accepted, 2026-10-15. Amends D-413.

**Decision.** The machine check `an inactive lower-root unmap preserves the live
observer` holds a nested-safe interrupt exclusion from its allocator baseline
through local teardown and the final snapshot. The test still requires distinct
leaves, a warmed live translation, no `invlpg` for the inactive root, an
unchanged observer, complete unmap and reclaim, and exact allocator ownership.
Failure diagnostics name each sub-proof and both ownership snapshots.
Production paging behavior is unchanged.

**Rationale.** The debug kernel reproducibly failed this check while every
paging sub-proof passed: the service worker retired older zombies between setup
and teardown, so unrelated cleanup looked like an ownership imbalance. Removing
the conservation assertion would stop detecting fixture leaks, and retrying or
accepting the release-only pass would hide nondeterminism rather than separate
local ownership from concurrent cleanup.
