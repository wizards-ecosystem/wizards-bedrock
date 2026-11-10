# D-68 A lock guard cannot leave the CPU that took it

Status: Accepted, 2026-08-23.

**Context.** `SpinLockGuard` holds an `InterruptState` and restores it on
drop; `ThreadLockGuard` holds a borrow of its lock. Both were `Send`, as
everything they contain is, so nothing in the type system stopped a guard
being released on another thread.

On one CPU this is latent: there is nowhere else for it to go. On several it is
the bug the interrupt-flag design exists to prevent: the restored flag would
belong to the CPU that cleared it, leaving that CPU interruptible inside its
critical section while another releases a lock it never took.

**Decision.** At adoption both guards carried a private `PhantomData<*const
()>`, which made them `!Send` and `!Sync`. D-223 later centralizes the spin
guard's same fact in the `InterruptState` it owns; the thread-lock guard keeps
that field directly.

**Rationale.** One line each, no runtime cost, and a class closed before it
opens. The idiom is already here (`ThreadContext` is built the same way,
D-62), and it is the stable-Rust form of what Asterinas states as a safety
principle with `impl !Send`, which needs nightly.

Verified before adoption: `assert_send::<SpinLockGuard<'_, u32>>()` compiled
before the change and is `E0277` after it. D-223 makes that compiler refusal a
durable host check over both production guard types.

**Consequences.**

- The type system now pins guards to their acquiring context, not the absence
  of a second CPU. The SMP debt in D-56 shrinks by one item.
- The two construction sites gain a field. Nothing else changes: no caller
  moved a guard across threads, as there are none.
- Broken once, as D-28 requires: a `Send` assertion on the guard fails to
  compile.
