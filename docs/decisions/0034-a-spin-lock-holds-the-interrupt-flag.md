# D-34 A spin lock holds the interrupt flag

Status: Accepted, 2026-08-14. Amended by D-77.

**Decision.** Acquiring a `SpinLock` clears `IF`; the guard restores what it
found on drop. Every kernel lock does this; there is no second lock type.

**Rationale.** A lock taken both in ordinary kernel context and from an
interrupt handler deadlocks, and on one CPU that is a certainty, not a race
(`001-boot.md`). The fix belongs in the lock, not at the call sites.
Restoring instead of enabling makes nesting correct: an inner lock that
found the flag clear puts back a clear flag, so it cannot re-enable
interrupts while an outer section holds one. Release order is unlock, then
restore; the other order lets an interrupt arrive at code still holding the
lock.

**Consequences.**

- The surviving hazards are entries `IF` does not mask: a fault can be
  raised while a lock is held, and an NMI can arrive at any time. So D-31's
  rule that a trap handler must not log still stands, for a narrower reason.
- Interrupt latency is now bounded by the longest critical section.
  Scrolling the framebuffer console holds a lock across a screenful of pixel
  copies. Acceptable with no scheduler; revisit with one.
- Recovery is private to `wost`'s non-returning panic boundary. Kernel panic
  reporting never waits for or forcibly releases a kernel lock; a busy
  output channel may be omitted.
