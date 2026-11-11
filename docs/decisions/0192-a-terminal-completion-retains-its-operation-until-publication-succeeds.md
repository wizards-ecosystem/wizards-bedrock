# D-192 A terminal completion retains its operation until publication succeeds

Status: Accepted, 2026-08-27. Amends D-115 and D-167; closes PW-106.

**Decision.** A pending entry remains in its array and carries a private
settling state while a terminal path posts. Other cancellation, expiry and
teardown paths skip that state. A post refused before accounting changes
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

**Rationale.** Cancellation, expiry and teardown removed the operation before
posting and treated a missing reservation as unreachable with `debug_assert!`,
so in release the operation vanished, its reservation stayed outstanding for
ever, and collection could release pages the lost operation could reach. D-167
required teardown to wait, but its implementation had turned a failed wait into
unobserved success. A missing reservation can be reported before queue
accounting changes; a completion-page bounds failure is found only after it has
advanced, and the slot derives entirely from internal capacities and the fixed
page layout. Pending ownership and completion reservation are one transaction:
neither may commit alone. Keeping the operation in place makes a refusal
recoverable without allocating, copying target memory or inventing a second
reservation. A separate teardown side list would duplicate ownership and need
its own capacity and reclamation rules. Treating every post failure as fatal
would discard the available retry path before queue state changes. Letting
collection proceed after a logged refusal would preserve diagnosis while
reintroducing reuse beneath live work.

**Consequences.** `a refused settlement retains every owner` injects the
pre-accounting refusal through cancellation, deadline expiry and a real
faulting process. It requires the pending and reservation tallies to remain
live, cancellation to report pressure, the first collector to retain the
zombie, and a retry to publish exactly one terminal record before reclaim.
Removing the pending-entry restoration reddens only that check; it is mutation
100. The former teardown, deadline and cancellation mutations target the new
ownership path and still apply uniquely. No unsafe operation was added.
