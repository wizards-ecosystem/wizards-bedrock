# D-55 Trap context is a count the heap and the scheduler can ask

Status: Accepted (2026-08-22)

**Context.** `008-research-notes-2.md` §4.2 named the bug class no lint
here can catch — sleep-in-atomic-context, and its memory-safety cousin,
allocation from a handler that interrupted the allocator's own refill — and
said the idea was expressible on stable as a context token in the
`PanicToken` idiom, worth an ADR when the heap landed. The heap landed in
D-47; slice 2 adds the first switch out of a handler, which is where the
rule starts being breakable.

**Decision.** `wost::interrupt` keeps a trap-depth count: incremented around
every handler call, zero in thread context. The heap's `alloc` asserts it is
zero — loudly, because a silent null would be blamed on memory. `yield_now`
asserts it is zero. A switch out of a handler parks the count with the
thread it belongs to, so the thread switched to sees its own depth and the
preempted one gets its back on resumption.

The token *type* — a value only thread context can mint, taken by every API
that may sleep — is deferred to the first API that may sleep, which is
`WaitQueue::wait` in slice 4. Today's two callers assert against the count
directly, and a token over two call sites would be ceremony. Klint itself is
a custom driver and out under D-01.

**Rationale.** The count is the mechanism either way; the token is sugar
over it that pays off when there are enough call sites to forget one. The
allocation assert is the half that matters now, because it is the half with
memory safety behind it.

**Consequences.**

- Nothing in the kernel allocates in trap context today, which the first
  boot with the assert confirmed by not panicking — a fact that was not on
  record before.
- Not yet enforced: yielding while holding a lock. It cannot corrupt memory
  (the next thread spins with the flag it inherited), only hang. The token
  of slice 4 is where it becomes a type error.
