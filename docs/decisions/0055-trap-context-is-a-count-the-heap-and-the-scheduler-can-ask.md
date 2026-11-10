# D-55 Trap context is a count the heap and the scheduler can ask

Status: Accepted, 2026-08-22.

**Context.** `008-research-notes-2.md` §4.2 named the bug class no lint here can
catch: sleep-in-atomic-context, and its memory-safety cousin, allocation from a
handler that interrupted the allocator's own refill. It called the idea
expressible on stable as a context token in the `PanicToken` idiom, worth an ADR
once the heap landed. The heap landed in D-47. Slice 2 adds the first switch out
of a handler, where the rule becomes breakable.

**Decision.** `wost::interrupt` keeps a trap-depth count, incremented around
every handler call and zero in thread context. The heap's `alloc` asserts it
is zero, loudly, since a silent null would be blamed on memory.
`yield_now` asserts the same. A switch out of a handler parks the count with
its thread: the thread switched to sees its own depth, and the preempted one
gets its back on resumption.

The token *type*, a value only thread context can mint, taken by every API
that may sleep, waits for the first such API: `WaitQueue::wait` in slice 4.
Today's two callers assert against the count directly; a token over two call
sites would add nothing. Klint itself is a custom driver and out under D-01.

**Rationale.** The count is the mechanism either way; the token is sugar
that pays off once there are enough call sites to forget one. The
allocation assert matters now: memory safety is behind it.

**Consequences.**

- Nothing in the kernel allocates in trap context today. The first boot
  with the assert confirmed it by not panicking, a fact not recorded
  before.
- Not yet enforced: yielding while holding a lock. It cannot corrupt memory
  (the next thread spins with the flag it inherited), only hang. The slice 4
  token is where it becomes a type error.
