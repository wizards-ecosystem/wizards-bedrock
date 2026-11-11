# D-85 A lock has two levels, and the leaf is the one a `Drop` can take

Status: Accepted, 2026-08-24. Implements D-69.

**Decision.** D-69's counts (13 statics, 73 call sites) were exact. The shape
was not: `SpinLock` acquires in two ways, and the second needs the
justification.

- **`lock(&'a mut ThreadContext)`** is the default. The guard borrows the token
  for its life, so nothing that can block and no second lock is reachable while
  it is held.
- **`lock_leaf()`** is for an acquisition that *is* the whole operation: take,
  mutate, return, calling nothing. The obligation is on the body, not the
  caller, and it is short enough to read. A leaf may be taken while a
  token-bearing lock is held, which makes a nested acquisition sound rather than
  an exception.

`try_lock` and `try_lock_leaf` mirror them. `ThreadLock` gets no token-bearing
form. Its only holders are the two consoles, whose work is to format bytes and
return. A token on `ThreadLock::lock` would add a parameter to every `info!`
call site, since `log::record` takes the serial console's lock.

**`lock_from_trap()` is not built.** Every acquisition a handler makes is a
leaf, so `lock_leaf` already serves it. The trap-context rule is not weakened:
`ThreadLock` still asserts it, and a handler still cannot mint a token.

**`libs/lock-rank` is not built**: a rank with one non-trivial pair is not an
abstraction (`005`, no speculative API). If a multi-level nesting ever
appears, the rank goes there.

**The heap stops nesting, which deletes D-47's lock order.** `take_slot` now
returns null on an empty free list; the caller allocates the frame with the
lock *dropped*, re-acquires, and carves. A preemption in that window costs at
most one spare slab frame. `HEAP → FRAMES` is no longer an order this kernel
has, and `wost` holds no lock across a call: every non-leaf lock is the
kernel's own.

**Verified, in both directions.**
`let _a = a.lock(&mut cx); let _b = b.lock(&mut cx)` is `E0499`.
`let _g = a.lock(&mut cx); sleep_ms(&mut cx, 1)` is `E0499`.
`let g = a.lock(&mut cx); drop(g); sleep_ms(&mut cx, 1)` compiles. D-69's own
break-test, **delete `Drop for SpinLockGuard` and watch the error disappear**,
was run. Both `E0499`s vanish, because without a `Drop` impl NLL ends the borrow
at the call. The guarantee depends on `Drop`, so this break tests it where the
obvious one would not.

**Rationale.** A `Drop` impl cannot receive a token: `Frame` and `Segment` free
themselves to `FRAMES` from wherever they die, and `GlobalAlloc::alloc`'s
parameters are fixed by the language, so a token on every acquisition is
unavailable at any price. All 73 sites were already take-mutate-return under
D-66's assert, so the token has nothing to exclude at 68 of them; the
two-level form gets D-69's property where it is live, at the cost of one method
name. D-47's order existed because refilling under the guard was convenient
rather than necessary, and deleting an order is worth more than type-checking
it.

**Alternatives.** A token on every acquisition (rejected: `Drop` and
`GlobalAlloc` cannot supply one, so the most critical sites would need an
`unsafe`-shaped escape). A type-level rank lattice (rejected as speculative: one
pair). `lock_from_trap` as D-69 specified (rejected: no caller). Keeping the
heap's nesting under a rank (rejected: deleting the order is strictly better
than proving it safe).

**Consequences.**

- `ThreadContext` lives in `wost::sync`, where its `in_trap` fact already
  lives; the kernel re-exports it. The blocking API (`yield_now`, `sleep_ms`,
  `sleep_ns`, `park`, `block`, `WaitQueue::wait`, `input::read_key`) takes it
  by unique reference.
- D-47's `HEAP → FRAMES` is retired. `009-multitasking.md`'s lock-order section
  is superseded by `011-userspace.md`'s.
- The runtime assert D-66 added is untouched, per D-69: it covers what the
  borrow checker cannot.
- **The compile-time property has no automated check**, a gap. It lives in
  `wost`, which is cross-compiled, so a `compile_fail` doctest cannot run
  against it (D-41). It was verified by hand against the pinned toolchain, as
  above, and re-verifying it is a manual step when the guard types change.
  `libs/` could hold it only as a copy that would drift.
- The machine check *"a refill costs one frame and balances"* covers the
  restructure. Its other half, that no slot is handed out twice, is checked as
  arithmetic in `libs/slab-layout`, because breaking it on the machine corrupts
  the allocator every later check runs on: a crashing break proves less than a
  failing one.
