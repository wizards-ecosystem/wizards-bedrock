# D-126 A leaf lock is the last lock, and now something says so

Status: Accepted, 2026-08-24.

**Decision.** D-121's rule is enforced at run time. `LEAVES_HELD` is
incremented only by `lock_leaf` and `lock_from_trap`, and any acquisition made
while it is non-zero records `Location::caller()` for **both ends**: the lock
being taken, and the outermost leaf that was held. A boot-time check, `no lock
is taken while a leaf guard is held`, reports the pair and fails. The outermost,
specifically: recording every leaf acquisition names whichever was taken most
recently, held or not.

The obligation is asymmetric. A leaf taken *under* a token-bearing lock is the
design: it is the only acquisition a `Drop` can perform, since `drop` takes no
arguments and a freed `Frame` must reach the allocator from wherever it dies.
Only an acquisition under a leaf is the violation.

D-121's left-owed lint was to wait for a fourth offender; there were four
already. Each is fixed by D-121's move, copying out what the caller needs and
dropping the guard: `pic::silence` claims its ports between its two
acquisitions, and `user::serve_ring`, `user::ring_shape_of` and
`user::bootstrap_of` no longer hold `BOOTSTRAPS` across their work.
`Bootstrap::ring` and `Bootstrap::table` are `Arc`s so a clone is what leaves
the guard.

**The user-copy recovery arm is gated on `frame.code_selector() & 0b11 == 0`.**
It runs before the ring-3 test and never looked at the saved `CS`, so a ring-3
thread with `RIP` inside the copy window had its fetch fault "recovered". A
**real ring-3 thread** drives the check `a program cannot aim at the copy
window`: the emitted program jumps to an argument in the kernel's half, aimed
at `usercopy::fault_window()`.

**Rationale.** A rule applied once, at the moment the code is written, decays
silently, which is D-119's argument about D-28. The property is *dynamic*:
`serve_ring` takes no lock itself, it calls `Queue::serve`, which calls
`WaitQueue::wake_all`, which takes one, so a lint would need a whole-program
call graph and a counter sees it on the first boot that exercises the path. A
`debug_assert` would overstate a latent deadlock as a crash and stop the boot
before the others are found. The detector also caught a violation with **no
call site**: D-128's growth scenario dropped a `Vec` under `ARENA`'s guard, a
lock taken by a destructor at scope exit, which a static analysis would have to
model destructor insertion to see.

What this does **not** catch: a path no boot exercises. A green result is not a
proof.

**Consequences.**

- Every `SpinLock` acquisition costs one relaxed load and a branch.
- `acquire` takes a `leaf: bool`, and `lock`/`try_lock` pass `false` while
  `lock_leaf`/`try_lock_leaf`/`lock_from_trap`/`try_lock_from_trap` pass `true`.
  `SpinLockGuard` carries the flag so its release knows which counter to undo.
- The branch-target assertion in `program.rs` consumes opcode and displacement
  as a pair: a byte scan over emitted code is a decoder pretending not to be
  one.

**Left owed.** ~~The check is one word for the whole machine, which is right on
one CPU and wrong the moment there are two: `LEAVES_HELD` must move into
`arch::percpu` beside `SPINLOCKS_HELD` before an AP starts, or two CPUs will
report each other's nesting.~~ **Closed by D-154**, which built
`arch::percpu` and moved both counters and the violation record behind it.
`arch::percpu` did not exist when this was written, which is why the sentence
says "move into" a module that had to be created first.

`lock_from_trap` counts as a leaf, which is conservative: a trap handler's
acquisition is take-mutate-return by the same argument, but nothing checks that
it is, and the panic path takes locks under it deliberately.
