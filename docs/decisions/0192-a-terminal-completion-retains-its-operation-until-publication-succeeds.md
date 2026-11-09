# D-192 A terminal completion retains its operation until publication succeeds

Status: Accepted (2026-08-27), amends D-115 and D-167; closes PW-106

**Context.** Cancellation, timer expiry and process teardown removed a pending
operation before posting its terminal completion. They treated a missing
reserved slot as unreachable with `debug_assert!`, then returned. In a release
image that assertion vanished: the operation was gone, its completion
reservation remained outstanding forever, and process collection could see an
empty pending array and release every page the lost operation had been allowed
to reach. D-167 required teardown to wait, but its implementation had turned a
failed wait into unobserved success.

There were two distinct refusal points. A missing completion reservation can be
reported before queue accounting changes. A completion-page bounds failure is
discovered only after the reservation has advanced; returning `false` there
would invite a retry against already-spent accounting even though the slot is
derived entirely from the queue's internal capacities and fixed page layout.

**Decision.** A pending entry remains in its array and carries a private
settling marker while a terminal path posts. Other cancellation, expiry and
teardown paths skip that marker. A post refused before accounting changes
restores the entry to live state and increments a kernel-side refusal tally;
only a successful post removes it. Cancellation reports `STATUS_NO_ROOM` for
its own operation while leaving its target retryable. Expiry and teardown stop
their sweep and let the next caller retry.

Process collection keeps the complete `Bootstrap` in the zombie table while it
settles the ring. It removes and reclaims that exact owner only after
`Queue::is_quiet`; a refusal is warned, counted and leaves the zombie intact.
The final completion-page write is a release assertion because its bounds are
an internal construction invariant and reservation accounting has already
changed. It is diagnosable fatal corruption, not a recoverable post refusal.

**Rationale.** Pending ownership and completion reservation are one
transaction: neither may commit alone. Keeping the operation in place makes a
refusal recoverable without allocating, copying target memory or inventing a
second reservation. A separate teardown side list would duplicate ownership
and need its own capacity and reclamation rules. Treating every post failure as
fatal would prevent silent loss but would discard the available retry path
before queue state changes. Letting collection proceed after a logged refusal
would preserve diagnosis while reintroducing reuse beneath live work.

**Consequences.** `a refused settlement retains every owner` injects the
pre-accounting refusal through cancellation, deadline expiry and a real
faulting process. It requires the pending and reservation tallies to remain
live, cancellation to report pressure, the first collector to retain the
zombie, and a retry to publish exactly one terminal record before reclaim.
The release image passed all 197 checks. Removing the pending-entry restoration
reddened only that check with `passed=196 failed=1 expected=197`; it is mutation
100. The former teardown, deadline and cancellation mutations were retargeted
to the new ownership path and still apply uniquely. No unsafe operation was
added. The image now carries 197 machine checks.
