# D-70 SMP: the target model, and the rules that cost nothing now

Status: Accepted, 2026-08-23.

**Context.** SMP is Phase 12. The 2026-08-23 research asked whether that is a
foundational mistake and answered: deferring the *bring-up* is correct, and
deferring the *design discipline* past Phase 5 is not — because Phase 5 fixes
the handle table and the queue ABI, the two most SMP-sensitive interfaces a
kernel has.

The decade-long retrofits usually cited were retrofits of large kernels with
frozen ABIs. Small kernels do it in months: Maestro in eleven weeks, Theseus in
twenty-four days, seL4 in one release behind a big kernel lock.

Two of the three reasons to think this untestable turn out to be false. Lock
*ordering* is observable on one CPU — lockdep's own documentation says complex
multi-CPU deadlocks "can be detected on a plain, lightly loaded single-CPU
system as well". The one bug class a single CPU genuinely cannot show is
Store→Load reordering, which is exactly the lost-wakeup re-check `002` already
lists as owed before Phase 5.

**Decision.** No SMP code this side of Phase 12. These rules, which cost
nothing to hold now and are expensive to retrofit:

1. **The target model is one big kernel lock at first bring-up, with per-CPU
   run queues and correct shootdown.** Today's coarse locking is therefore the
   plan, not debt.
2. **Never write "disable interrupts" where "disable preemption" is meant.** We
   currently have no preempt-disable mechanism at all — the only gate on
   preemption is the frame's interrupt flag, so `SpinLock`'s `cli` is doing
   that job. A preempt-disabled-interrupts-on lock is the common case and much
   cheaper; it arrives in Phase 5 with the per-CPU word D-56 anticipates.
3. **No lock guard live across a context switch.** Already enforced by D-66's
   assert in `prepare()`. Recorded here so the SMP rationale is attached to it:
   Redox's September-2025 multi-core hang was fixed with 292 lines of scoping
   for precisely this, and someone weakening the assert later should know it
   guards two things.
4. **`Relaxed` requires justification the way `unsafe` does.** `Acquire` and
   `Release` are identical codegen to `Relaxed` on x86-64, so correctness here
   is free today and unobtainable later.
5. **Interrupt handlers never block.** FreeBSD's opposite answer touched every
   driver.
6. **A full barrier on both sides of the lost-wakeup re-check** — one locked
   instruction on the block/wake slow path.

And three interface rules for Phase 5, from the Scalable Commutativity Rule
(Clements et al., SOSP'13), whose decisive property is that it is analysable
before an implementation exists and before the hardware is available:

7. **The handle table must not promise "lowest free handle."** sv6 measured the
   unordered alternative scaling linearly with *no* single-core penalty.
8. **The queue ABI must not promise global FIFO across submitters.**
9. **Unmap, close and release must not promise global visibility on return.**

**Rationale.** Every item above is either free or already true; what they buy is
that Phase 12 becomes a bring-up rather than a redesign. The mechanical SMP
debts were already tracked in D-45, D-56 and D-60; the semantic ones — lock
ordering, memory ordering, deferred reclamation, interface commutativity — were
on no list, and this is that list.

**Consequences.**

- `THREADS` should be split into a thread table and a run queue: a struct
  boundary now, a scheduler rewrite avoided later. `libs/eevdf` is already
  domain-agnostic and `Entity::place(average, lag, slice)` is the migration
  hook. Redox measured a *global* EEVDF run queue at four cores being three
  times slower than one.
- Rust does not save us here. Converos (ATC'25) model-checked twelve Asterinas
  concurrency modules and found twenty bugs in the safe code.
- `SPINLOCKS_HELD` is still a global, which stays on D-56's per-CPU list.
