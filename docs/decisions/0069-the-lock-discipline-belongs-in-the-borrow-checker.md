# D-69 The lock discipline belongs in the borrow checker

Status: Accepted, 2026-08-23. Implemented by D-85 with its shape corrected.

**Context.** D-66 made "a spin lock is held" a runtime assert, checked at every
point where the interrupt flag is set or a thread is switched. It works, and it
caught the bug it was written for. But it is a runtime check for a property the
compiler could prove, and Phase 5 is about to multiply the number of places it
must hold.

The 2026-08-23 research found that Rust-for-Linux considered exactly this
pattern and rejected it, for reasons that do not apply here: a million-line C
call graph, C callers invoking Rust callbacks, and a maintainer preference
against runtime checks. Their answer was **klint**, an out-of-tree compile-time
preemption-count analyser pinned to a specific nightly — unavailable to a
stable-only project (D-01), and in maintenance mode since 2023.

**Decision.** `SpinLock::lock` will take `&'a mut ThreadContext` and return a
guard borrowing it for the guard's life, so that nothing requiring the token —
that is, anything that may block — can be called while a spin lock is held.
Trap context, which cannot mint a token by construction (D-62), gets a separate
`lock_from_trap()`. The runtime assert stays as the backstop for paths the
borrow checker cannot see.

The change is recorded now and made in Phase 5, alongside the lock work that
phase brings, rather than as a separate pass over 13 statics and 73 call sites.

**Rationale.** This is what klint exists to do, obtained on stable from the
borrow checker — and it is *stronger* in three ways klint's own documentation
concedes: it sees through trait objects and function pointers, which klint
assumes are sleepable; it handles `Drop` and drop flags, which are klint's worst
limitation; and it models `try_lock`, which klint cannot represent.

Verified on the pinned toolchain before being recorded. `lock(cx); drop(g);
sleep(cx)` compiles; `let _g = lock(cx); sleep(cx)` and `let _a = A.lock(cx);
let _b = B.lock(cx)` are both `E0499`.

**Two things testing found that reading did not.**

The guarantee **depends on the guard having a `Drop` impl**. Without one, NLL
ends the borrow at the call and both violations compile silently — the first
attempt at this proof was a false green for exactly that reason. Our guards do
have `Drop`, so the property holds; but the break-test for this ADR is
*delete the `Drop` impl and watch the compile error disappear*, not the more
obvious one.

And the trap-context path is not a compromise. In a handler the flag is already
clear, so "cannot block here" is enforced by the machine; the token would be
proving something already true. `lock_from_trap` can assert `in_trap()`, which
makes the two entry points mutually exclusive by construction.

**Consequences.**

- Two spin locks at once becomes a borrow error. Where nesting is genuinely
  needed, the second acquisition takes the *guard*, and that is where a
  type-level lock rank belongs — pure arithmetic, so `libs/` per D-41.
- Phase 5's lock work is larger by this refactor and smaller by every deadlock
  it forecloses.
- The runtime assert is not removed. Two independent checks is what OSTD ships,
  and the assert covers what the borrow checker cannot.
