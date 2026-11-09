# D-85 A lock has two levels, and the leaf is the one a `Drop` can take

Status: Accepted (2026-08-24), implements D-69

**Context.** D-69 decided that `SpinLock::lock` would take `&'a mut
ThreadContext` and return a guard borrowing it, so that blocking with a lock
held, or taking two locks, become borrow errors. It deferred the work to Phase
5 and predicted its shape: 13 statics, 73 call sites, a `lock_from_trap()` for
handlers, and a type-level lock rank in `libs/` for the nestings that remain.
The counts were exact. The shape was not.

**What the doing found.** Three things reading could not have.

*A `Drop` impl cannot receive a token.* `Frame` and `Segment` free themselves
back to the physical allocator when they die, from wherever they die, and
`drop` takes no arguments. `FRAMES` therefore cannot be a token-bearing lock,
whatever the discipline says. The same is true of `GlobalAlloc::alloc`, whose
signature is the language's.

*This kernel holds no lock across a call.* Every one of the 73 sites is
take-mutate-return: Phase 4's scoping — forced by D-66's assert, which refuses
a switch with a guard live — had already produced the property D-69 wanted to
enforce, everywhere. The token has nothing to exclude at 68 of them.

*So the discipline is two levels, not a rank lattice.* A rank with one
non-trivial pair is not an abstraction (`005`, no speculative API).

**Decision.** `SpinLock` acquires in two ways, and the second is the one that
needs justifying.

- **`lock(&'a mut ThreadContext)`** — the default. The guard borrows the token
  for its life, so nothing that can block and no second lock is reachable while
  it is held.
- **`lock_leaf()`** — for an acquisition that *is* the whole operation: take,
  mutate, return, calling nothing. The obligation is on the body, not the
  caller, and it is short enough to read. A leaf may be taken while a
  token-bearing lock is held, which is what makes a nested acquisition sound
  rather than an exception.

`try_lock` and `try_lock_leaf` mirror them. `ThreadLock` gets no token-bearing
form, because its only holders are the two consoles and their work is to format
bytes and return — and a token on `ThreadLock::lock` would infect every `info!`
call site with a parameter, since `log::record` takes the serial console's lock.

**`lock_from_trap()` is not built.** D-69 expected handlers to need their own
entry point. They do not: every acquisition a handler makes is a leaf, so
`lock_leaf` already serves it, and adding a second name for the same operation
would have been API with no caller. The trap-context rule is not weakened —
`ThreadLock` still asserts it, and a handler still cannot mint a token.

**`libs/lock-rank` is not built**, for the same reason. If a genuine multi-level
nesting ever appears, this is where the rank goes.

**The heap stops nesting, which deletes D-47's lock order.** `HEAP` was the one
lock held across a call: `take_slot` refilled from inside the guard, and refill
takes `FRAMES`. It now returns null on an empty free list, and the caller
allocates the frame with the lock *dropped*, re-acquires, and carves. The
window that opens costs at most one spare slab frame when a preemption lands in
it. `HEAP → FRAMES` is therefore no longer an order this kernel has, and
`wost` holds no lock across a call at all: every non-leaf lock in this kernel
is the kernel's own.

**Verified, in both directions.** `let _a = a.lock(&mut cx); let _b =
b.lock(&mut cx)` is `E0499`. `let _g = a.lock(&mut cx); sleep_ms(&mut cx, 1)`
is `E0499`. `let g = a.lock(&mut cx); drop(g); sleep_ms(&mut cx, 1)` compiles.
And D-69's own break-test — **delete `Drop for SpinLockGuard` and watch the
error disappear** — was run: both `E0499`s vanish, because without a `Drop`
impl NLL ends the borrow at the call. The guarantee is `Drop`-dependent, and
that is the break that tests it rather than the obvious one.

**Rationale.** The two-level form gets D-69's property where the property is
live, at the cost of one extra method name, and it is honest about the 68 sites
where the token would have been ceremony. The alternative — a token on every
acquisition — is not available at any price, because `Drop` impls exist.

The heap restructure is the better half of the change. D-47's order was the only
one this kernel had, and it was there because refilling under the guard was
convenient rather than necessary. Deleting an order is worth more than
type-checking it.

**Alternatives.** A token on every acquisition (rejected: `Drop` and
`GlobalAlloc` cannot supply one, so the discipline would have needed an
`unsafe`-shaped escape at its most load-bearing sites); a type-level rank
lattice (rejected as speculative — one pair); `lock_from_trap` as D-69
specified (rejected: no caller); keeping the heap's nesting and expressing it
with a rank (rejected: deleting the order is strictly better than proving it
safe).

**Consequences.**

- `ThreadContext` moves from `kernel::thread` to `wost::sync`, where the
  `in_trap` fact it is built on already lives; the kernel re-exports it, so no
  call site changed name. The blocking API — `yield_now`, `sleep_ms`,
  `sleep_ns`, `park`, `block`, `WaitQueue::wait`, `input::read_key` — takes it
  by unique reference now instead of shared.
- D-47's `HEAP → FRAMES` is retired. `009-multitasking.md`'s lock-order section
  is superseded by `011-userspace.md`'s.
- The runtime assert D-66 added is untouched, per D-69: two independent checks
  is what OSTD ships, and the assert covers what the borrow checker cannot.
- **The compile-time property has no automated check**, and this is a real gap.
  It lives in `wost`, which is cross-compiled, so a `compile_fail` doctest
  cannot run against it (D-41). It was verified by hand against the pinned
  toolchain, above, and re-verifying it is a manual step when the guard types
  change. `libs/` cannot hold it without holding a copy that would drift.
- The machine check the restructure earned is *"a refill costs one frame and
  balances"*. Its other half — that no slot is handed out twice — is checked as
  arithmetic in `libs/slab-layout`, because breaking it on the machine corrupts
  the allocator every later check runs on and produces a general-protection
  fault instead of a red verdict. Phase 4 learned twice that a break which
  crashes proves less than one that fails; this is the first time that lesson
  chose where a check lives.
- Eighty named machine checks.
