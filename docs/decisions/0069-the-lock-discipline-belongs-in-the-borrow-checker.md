# D-69 The lock discipline belongs in the borrow checker

Status: Accepted, 2026-08-23. Implemented by D-85 with its shape corrected.

**Decision.** `SpinLock::lock` will take `&'a mut ThreadContext` and return
a guard borrowing it for the guard's life, so nothing requiring the token
(anything that may block) can be called while a spin lock is held. Trap
context, which cannot mint a token (D-62), gets a separate
`lock_from_trap()`, which can assert `in_trap()`, making the two entry
points mutually exclusive. The runtime assert stays as backstop for paths
the borrow checker cannot see. The change lands in Phase 5 with that phase's
lock work, not as a separate pass over 13 statics and 73 call sites.

**Rationale.** D-66's assert is a runtime check for a property the compiler
could prove. Rust-for-Linux's answer, klint, needs a pinned nightly (D-01)
and is in maintenance mode. The borrow checker does klint's job on stable,
and is *stronger*: it sees through trait objects and function pointers,
handles `Drop` and drop flags, and models `try_lock`. In a handler the flag
is already clear, so the machine enforces "cannot block here" and a token
would prove something already true.

**Consequences.**

- The guarantee **depends on the guard having a `Drop` impl**. Without one,
  NLL ends the borrow at the call and both violations compile silently. The
  break-test for this ADR is *delete the `Drop` impl and watch the compile
  error disappear*.
- Two spin locks at once becomes a borrow error. Where nesting is needed,
  the second acquisition takes the *guard*, and a type-level lock rank
  belongs there: pure arithmetic, so `libs/` per D-41.
- The runtime assert stays: it covers what the borrow checker cannot.
