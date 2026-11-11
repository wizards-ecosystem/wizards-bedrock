# D-223 CPU-local guards and syscall wakes have durable direct witnesses

Status: Accepted, 2026-08-29. Amends D-68 and D-133; closes PW-405.

**Decision.** A host compile-fail harness creates a temporary `no_std` crate
which path-depends on this checkout's actual `wost`, asks a generic `T: Send`
function to accept `SpinLockGuard` and `ThreadLockGuard` separately, and
cross-checks each offline for the compiler's `E0277`. No production type or
auto-trait mechanism is copied. `SpinLockGuard` no longer repeats a marker:
its owned `InterruptState` is the CPU-local proof and the thread-lock guard
keeps its direct marker.

The machine harness uses the real user process and ring. A self-test-only gate
stops one child in ordinary kernel thread context before its first user
instruction. A kernel waiter registers on that child's `Queue::waiters`, and
the harness observes both registrations before releasing the child. Its Echo
submission and `RingEnter` trap then call the production `Queue::serve` →
`WaitQueue::wake_all` → `thread::unblock` path. A cancellation condition exists
only so failed setup cannot strand a test thread.

`RingEnter` samples D-133's production handler-arm counter immediately around
`serve`. That narrow observation is necessary: the child's following Exit
trap wakes the mandatory reap worker and legitimately increments the global
counter a second time. Waiting for the ring waiter to run is not substituted
for observing the arm; an already-armed slice can run it on the broken kernel.

The spawn and observation phases are separate functions, because the
observation frame beneath `spawn_transaction`'s large unoptimised frame reached
the debug boot stack's guard; keeping it out of the spawn call restores headroom
without enlarging every kernel stack.

**Proof.** The machine check observes both waits registered, one exact
RingEnter arm, the waiter returned, Echo completed with its own token, the child
exited without a user fault, and both thread slots returned. Reinstating
D-133's trap exemption leaves the RingEnter arm at zero and fails only `a
syscall wake arms the timer for what it woke`; D-133's existing mutation names
this direct witness. Making either guard's `PhantomData` type `()` lets it
satisfy `Send` and fails `contracts::guard_types_do_not_cross_cpus`, because
the expected compiler refusal becomes a successful compile; mutations 234 and
235 retain both.

**Alternatives.** A source grep for `PhantomData` was rejected because it
would bless spelling rather than the auto trait. A stand-in `libs/` type was
rejected because it could drift while the production guard became `Send`.
Keeping the keyboard proxy was rejected because the caller that exposed the
defect now exists and can be orchestrated without a race. A delay before
`RingEnter` was rejected because timing is not registration evidence.

**Consequences.** D-68 had observed `E0277` only once by hand, and D-133's
lasting check drove a keyboard proxy; the direct syscall check replaces that
proxy. The guard contract adds no Cargo feature graph and no shipped code; its
temporary crate is compiled only by the host test. PW-405 is closed.
