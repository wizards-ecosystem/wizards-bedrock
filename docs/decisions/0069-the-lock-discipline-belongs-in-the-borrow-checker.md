# D-69 The lock discipline belongs in the borrow checker

Status: Accepted, 2026-08-23. Implemented by D-85 with its shape corrected.

**Context.** D-66 made "a spin lock is held" a runtime assert, checked
wherever the interrupt flag is set or a thread is switched, and it caught its
bug. It is still a runtime check for a property the compiler could prove, and
Phase 5 will multiply the places it must hold.

The 2026-08-23 research found that Rust-for-Linux considered this pattern and
rejected it for reasons that do not apply here: a million-line C call graph, C
callers invoking Rust callbacks, and a maintainer preference against runtime
checks. Their answer was **klint**, an out-of-tree compile-time
preemption-count analyser pinned to a specific nightly: unavailable to a
stable-only project (D-01), and in maintenance mode since 2023.

**Decision.** `SpinLock::lock` will take `&'a mut ThreadContext` and return a
guard borrowing it for the guard's life, so nothing requiring the token
(anything that may block) can be called while a spin lock is held. Trap
context, which cannot mint a token (D-62), gets a separate `lock_from_trap()`.
The runtime assert stays as backstop for paths the borrow checker cannot see.

The change lands in Phase 5 with that phase's lock work, not as a separate
pass over 13 statics and 73 call sites.

**Rationale.** The borrow checker does klint's job on stable, and is
*stronger* in three ways klint's documentation concedes: it sees through trait
objects and function pointers, which klint assumes are sleepable; it handles
`Drop` and drop flags, klint's worst limitation; and it models `try_lock`,
which klint cannot represent.

Verified first on the pinned toolchain: `lock(cx); drop(g); sleep(cx)`
compiles; `let _g = lock(cx); sleep(cx)` and `let _a = A.lock(cx); let _b =
B.lock(cx)` are both `E0499`.

**Two things testing found that reading did not.**

The guarantee **depends on the guard having a `Drop` impl**. Without one, NLL
ends the borrow at the call and both violations compile silently; the first
attempt at this proof was a false green for that reason. Our guards have
`Drop`, so the property holds, and the break-test for this ADR is *delete the
`Drop` impl and watch the compile error disappear*, not the more obvious one.

The trap-context path is not a compromise. In a handler the flag is already
clear, so the machine enforces "cannot block here" and a token would prove
something already true. `lock_from_trap` can assert `in_trap()`, making the
two entry points mutually exclusive.

**Consequences.**

- Two spin locks at once becomes a borrow error. Where nesting is needed, the
  second acquisition takes the *guard*, and a type-level lock rank belongs
  there: pure arithmetic, so `libs/` per D-41.
- Phase 5's lock work grows by this refactor and shrinks by every deadlock it
  forecloses.
- The runtime assert stays: OSTD ships two independent checks, and it covers
  what the borrow checker cannot.
