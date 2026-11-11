# D-62 The context token arrives with the API that needed it

Status: Accepted, 2026-08-23.

**Decision.** `ThreadContext` is a zero-sized token minted only by
`ThreadContext::claim`, which refuses in trap context. `yield_now`,
`sleep_ms` and `WaitQueue::wait` take one by reference. It is neither `Send`
nor `Sync`, so it cannot be stashed in a `static` and produced later by
something with no right to it.

**Rationale.** D-55 deferred the token until an API could sleep; D-58 and
D-61 are that API. The count was always the mechanism; the token turns three
run-time asserts into one compile-time rule.

**Consequences.**

- `ThreadContext::require` serves two places structurally in thread context
  that would rather panic than pass an `Option` through: the boot thread's
  final loop, and the self-checks.
- The token proves the *context*, not the absence of a lock. Yielding while
  holding one is still possible and still only a hang. A guard-carrying
  variant is the next step and has no caller yet.
