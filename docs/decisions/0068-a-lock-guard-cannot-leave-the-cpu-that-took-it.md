# D-68 A lock guard cannot leave the CPU that took it

Status: Accepted (2026-08-23)

**Context.** `SpinLockGuard` holds an `InterruptState` and restores it on drop;
`ThreadLockGuard` holds a borrow of its lock. Both were `Send`, because
everything they contain is. Nothing in the type system stopped a guard being
moved to another thread and released there.

On one CPU this is latent — there is nowhere else for it to go. On several it
is the bug the whole interrupt-flag design exists to prevent: the flag restored
would belong to the CPU that cleared it, leaving that CPU interruptible inside
its own critical section while another CPU releases a lock it never took.

**Decision.** At adoption both guards carried a private
`PhantomData<*const ()>`, which made them `!Send` and `!Sync`. D-223 later
centralizes the spin guard's same fact in the `InterruptState` it owns; the
thread-lock guard retains its direct marker.

**Rationale.** One line each, no runtime cost, and it closes a class before it
can open. The idiom is already in this codebase — `ThreadContext` is built the
same way (D-62) — and it is the stable-Rust form of what Asterinas states as a
safety principle with `impl !Send`, which needs nightly.

Verified before adoption: `assert_send::<SpinLockGuard<'_, u32>>()` compiled
before the change and is `E0277` after it. D-223 makes that compiler refusal a
durable host check over both production guard types.

**Consequences.**

- Guards are now pinned to their acquiring context by the type system, not by
  the absence of a second CPU. The SMP debt in D-56 shrinks by one item.
- The two construction sites gain a field. Nothing else changes; no caller
  moved a guard across threads, because there are no threads to move it to.
- Broken once, as D-28 requires: a `Send` assertion on the guard fails to
  compile.
