# D-62 The context token arrives with the API that needed it

Status: Accepted (2026-08-23)

**Context.** D-55 built the trap-depth count and deferred the *token* —
`008-research-notes-2.md` §4.2's idea — until there was an API that could
sleep, on the grounds that a token over two call sites would be ceremony.
D-58 and D-61 are that API.

**Decision.** `ThreadContext` is a zero-sized token minted only by
`ThreadContext::claim`, which refuses in trap context. `yield_now`,
`sleep_ms` and `WaitQueue::wait` take one by reference. It is neither `Send`
nor `Sync`, so it cannot be stashed in a `static` and produced later by
something with no right to it.

**Rationale.** The count was always the mechanism; the token is what turns
three run-time asserts into one compile-time rule. With three call sites and
more coming in Phase 5, it now pays for itself.

**Consequences.**

- `ThreadContext::require` exists for the two places that are structurally in
  thread context and would rather panic than thread an `Option` through —
  the boot thread's final loop, and the self-checks.
- The token proves the *context*, not the absence of a lock. Yielding while
  holding one is still possible and still only a hang. A guard-carrying
  variant is the obvious next step and has no caller yet.
