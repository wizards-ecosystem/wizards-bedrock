# D-194 A forgotten spin-lock guard is an accepted, bounded liveness violation

Status: Accepted, 2026-08-27. Amends D-66, D-69 and D-85; closes PW-105.

**Decision.** Guard destruction is an explicit first-party liveness contract:
code that acquires a `SpinLock` must return through the guard's destructor and
must not deliberately forget it. Violating that contract may halt progress, but
does not widen memory or privilege authority. The bound is the current CPU and
every waiter for that lock; while the kernel has one CPU, that is the whole
machine. D-66's runtime backstops remain mandatory: enabling interrupts,
switching context, abandoning a stack and entering ring 3 all refuse while the
per-CPU held count is nonzero. Production does not force-release a forgotten
guard because it has no proof that a still-live holder cannot resume; only the
terminal panic path has that proof through `PanicToken`.

The self-test build gets a controlled witness, not a recovery API for shipped
code. It consumes a real guard, transfers its interrupt state into a
non-dereferencing recovery owner, marks the lock as deliberately forgotten and
calls `core::mem::forget` on the original guard. The machine can then
distinguish ordinary contention from that deliberate leak before dropping the
recovery owner and restoring the exact lock, accounting and interrupt state.
Neither the forgotten state nor the controlled-forget entry point exists in a
shipped build.

**Rationale.** Safe Rust does not promise that `Drop` runs:
`core::mem::forget(guard)` leaves the lock, the held-lock count and the disabled
interrupt flag permanently held. The forgotten reference cannot be used again,
so this creates no aliased access, but on one CPU the disabled interrupt flag
makes the liveness loss global. Rust can make accidental movement or nesting
difficult; it cannot guarantee progress from arbitrary safe code.

**Alternatives.** A closure-hidden lock was rejected because divergence inside
the closure has the same outcome, the closure still needs an internal guard
whose destructor panic-abort never runs, and converting every call would buy no
stronger invariant. A critical-section token was rejected as the liveness proof
because safe code may forget it, and `Drop`/allocator paths cannot receive one;
D-69's thread-context token remains useful for excluding blocking and
accidental nesting. Making `SpinLockGuard` private was rejected because the
kernel must borrow protected values and would merely move the internal guard
that can be leaked. General force-unlock was rejected as unsound without the
panic path's proof that the old holder can never resume.

**Consequences.** `forgotten lock ownership is distinct from contention` holds
an ordinary guard and observes contention, then deliberately forgets that guard
through the controlled test seam and observes the distinct forgotten state. It
also requires the lock to stay unavailable until recovery and the original
interrupt state and acquisition ability to return afterwards. Removing the
forgotten-state transition reddens only that check; it is mutation 102. The
witness adds no unsafe operation and no shipped-image state.
