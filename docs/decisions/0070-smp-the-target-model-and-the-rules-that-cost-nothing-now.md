# D-70 SMP: the target model, and the rules that cost nothing now

Status: Accepted, 2026-08-23.

**Context.** SMP is Phase 12. The 2026-08-23 research asked whether that is a
foundational mistake. Deferring the *bring-up* is correct; deferring the
*design discipline* past Phase 5 is not, because Phase 5 fixes the handle
table and the queue ABI, a kernel's two most SMP-sensitive interfaces.

The decade-long retrofits usually cited were of large kernels with frozen
ABIs. Small kernels take months: Maestro eleven weeks, Theseus twenty-four
days, seL4 one release behind a big kernel lock.

Two of three reasons to call this untestable are false. Lock *ordering* is
observable on one CPU: lockdep's documentation says complex multi-CPU
deadlocks "can be detected on a plain, lightly loaded single-CPU system as
well". The one bug class a single CPU cannot show is Store→Load reordering,
the lost-wakeup re-check `002` already lists as owed before Phase 5.

**Decision.** No SMP code this side of Phase 12. These rules cost nothing now
and are expensive to retrofit:

1. **The target model is one big kernel lock at first bring-up, with per-CPU
   run queues and correct shootdown.** Today's coarse locking is the plan, not
   debt.
2. **Never write "disable interrupts" where "disable preemption" is meant.**
   No preempt-disable mechanism exists yet: the frame's interrupt flag is the
   only gate on preemption, so `SpinLock`'s `cli` does that job. A
   preempt-disabled-interrupts-on lock is the common, much cheaper case; it
   arrives in Phase 5 with the per-CPU word D-56 anticipates.
3. **No lock guard live across a context switch.** D-66's assert in
   `prepare()` enforces it; this records its SMP rationale. Redox's
   September-2025 multi-core hang took 292 lines of scoping to fix, so whoever
   weakens the assert should know it guards two things.
4. **`Relaxed` requires justification the way `unsafe` does.** `Acquire` and
   `Release` generate the same code as `Relaxed` on x86-64, so correctness is
   free today and unobtainable later.
5. **Interrupt handlers never block.** FreeBSD's opposite answer touched every
   driver.
6. **A full barrier on both sides of the lost-wakeup re-check:** one locked
   instruction on the block/wake slow path.

Three interface rules for Phase 5 come from the Scalable Commutativity Rule
(Clements et al., SOSP'13), which is analysable before an implementation or
the hardware exists:

7. **The handle table must not promise "lowest free handle."** sv6 measured
   the unordered alternative scaling linearly with *no* single-core penalty.
8. **The queue ABI must not promise global FIFO across submitters.**
9. **Unmap, close and release must not promise global visibility on return.**

**Rationale.** Every item is free or already true; together they make Phase 12
a bring-up, not a redesign. D-45, D-56 and D-60 track the mechanical SMP
debts. The semantic ones (lock ordering, memory ordering, deferred
reclamation, interface commutativity) were on no list; this is that list.

**Consequences.**

- `THREADS` should split into a thread table and a run queue: a struct
  boundary now, no scheduler rewrite later. `libs/eevdf` is already
  domain-agnostic and `Entity::place(average, lag, slice)` is the migration
  hook. Redox measured a *global* EEVDF run queue at four cores as three times
  slower than one.
- Rust does not save us here. Converos (ATC'25) model-checked twelve Asterinas
  concurrency modules and found twenty bugs in the safe code.
- `SPINLOCKS_HELD` is still a global, which stays on D-56's per-CPU list.
