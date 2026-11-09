# D-223 CPU-local guards and syscall wakes have durable direct witnesses

Status: Accepted (2026-08-29), amends D-68 and D-133; closes PW-405

**Context.** Two compiler and machine facts had each been checked only once.
D-68 manually observed `E0277` after making lock guards `!Send`, but `wost`
cannot run doctests and copying a guard-shaped stand-in into `libs/` would
only test the copy. D-133 fixed trap-context wakeups after finding that a
`RingEnter` completion could make a kernel waiter Ready without arming the
tickless timer, but its lasting check drove the keyboard proxy. The exact
syscall experiment was constructible and remained unbuilt.

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

The spawn and observation phases are separate functions. The first debug boot
put the observation frame beneath `spawn_transaction`'s deliberately large
unoptimised frame and reached the boot stack's guard while committing the
`AddressSpace`; keeping that frame out of the spawn call restored headroom
without enlarging every kernel stack. The debug gate then passed the exact
path that exposed the harness error.

**Proof.** The restored release machine boot observed both waits registered,
one exact RingEnter arm, the waiter returned, Echo completed and answered its
own token, the child exited without a user fault, and both thread slots were
returned. Reinstating D-133's trap exemption preserved every consequence but
left the RingEnter arm at zero; only `a syscall wake arms the timer for what it
woke` failed, at 211/212.

Changing `InterruptState`'s marker to `PhantomData<()>` made the actual
`SpinLockGuard` satisfy `Send`; changing the thread guard's marker did the same
there. Each made `contracts::guard_types_do_not_cross_cpus` fail because its
expected compiler refusal became a successful compile. Both changes persist
as mutations 234 and 235; D-133's existing mutation now names the direct
syscall witness.

**Alternatives.** A source grep for `PhantomData` was rejected because it
would bless spelling rather than the auto trait. A stand-in `libs/` type was
rejected because it could drift while the production guard became `Send`.
Keeping the keyboard proxy was rejected because the caller that exposed the
defect now exists and can be orchestrated without a race. A delay before
`RingEnter` was rejected because timing is not registration evidence.

**Consequences.** The machine-check count remains 212: the direct syscall
check replaces D-133's proxy. The mutation registry grows to 235. The guard
contract adds no Cargo feature graph and no shipped code; its temporary crate
is compiled only by the host test. PW-405 is closed.
