# D-70 SMP: the target model, and the rules that cost nothing now

Status: Accepted, 2026-08-23.

**Decision.** No SMP code this side of Phase 12. These rules cost nothing
now and are expensive to retrofit:

1. **The target model is one big kernel lock at first bring-up, with per-CPU
   run queues and correct shootdown.** Today's coarse locking is the plan,
   not debt.
2. **Never write "disable interrupts" where "disable preemption" is meant.**
   No preempt-disable mechanism exists yet: the frame's interrupt flag is
   the only gate on preemption, so `SpinLock`'s `cli` does that job. A
   preempt-disabled-interrupts-on lock is the common, much cheaper case; it
   arrives in Phase 5 with the per-CPU word D-56 anticipates.
3. **No lock guard live across a context switch.** D-66's assert in
   `prepare()` enforces it; this records its SMP rationale (Redox's
   multi-core hang was this bug class), so whoever weakens the assert should
   know it guards two things.
4. **`Relaxed` requires justification the way `unsafe` does.** `Acquire` and
   `Release` generate the same code as `Relaxed` on x86-64, so correctness
   is free today and unobtainable later.
5. **Interrupt handlers never block.**
6. **A full barrier on both sides of the lost-wakeup re-check:** one locked
   instruction on the block/wake slow path.

Three interface rules for Phase 5 come from the Scalable Commutativity Rule
(Clements et al., SOSP'13), which is analysable before an implementation or
the hardware exists:

7. **The handle table must not promise "lowest free handle."**
8. **The queue ABI must not promise global FIFO across submitters.**
9. **Unmap, close and release must not promise global visibility on
   return.**

**Rationale.** Phase 5 fixes the handle table and the queue ABI, a kernel's
two most SMP-sensitive interfaces, so the design discipline cannot wait for
the bring-up. Lock ordering is observable on one CPU; the one bug class a
single CPU cannot show is Store→Load reordering, the lost-wakeup re-check.
Every item is free or already true; together they make Phase 12 a bring-up,
not a redesign. D-45, D-56 and D-60 track the mechanical SMP debts; this is
the list of semantic ones (lock ordering, memory ordering, deferred
reclamation, interface commutativity).

**Consequences.**

- `THREADS` should split into a thread table and a run queue: a struct
  boundary now, no scheduler rewrite later. `libs/eevdf` is already
  domain-agnostic and `Entity::place(average, lag, slice)` is the migration
  hook.
- Rust does not save us here: model checking found concurrency bugs in
  Asterinas's safe code.
- `SPINLOCKS_HELD` is still a global, which stays on D-56's per-CPU list.
