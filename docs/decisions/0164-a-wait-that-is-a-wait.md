# D-164 A wait that is a wait

Status: Accepted, 2026-08-25. Amended by D-181.

**Context.** `014` opens by saying this is "the phase that gives the kernel
something to wait for", and D-149 puts the ABI first precisely so the waiting
shape is fixed before a filesystem depends on it. Slice 1 owes the first
operation whose honest answer is *not yet*.

Everything this kernel served through the ring until now answered immediately.
`perform` was a pure function of the submission: echo returns its argument,
uptime reads the clock. There was no in-flight state, and `Queue`'s wait queue
had been built in Phase 5 and never used.

**The obstacle is that a ring-3 thread had no way to stop running.** The
syscall arrives through an interrupt gate (D-96) and is served in trap context,
where `sleep_ns` cannot be used: it takes a `ThreadContext`, and there is none
inside a handler (D-55). A "wait" that could not block would be a spin, and a
program spinning is a program holding a CPU it is not using.

**Options.** Return "nothing yet" and let the program spin; run the ring from a
kernel worker thread that can block; or give the trap path its own way to yield.

**Decision.** All three of the pieces, each in its own place.

**An operation can be deferred.** `Opcode::Timer` is admitted, recorded in a
small pending table, and *not* completed. Its completion slot was reserved at
admission and stays reserved, which is what makes the later completion certain
rather than hopeful — the reservation is the difference between deferring an
operation and dropping one.

**A thread can sleep from a trap.** `thread::sleep_from_trap` is
`retire_from_trap`'s shape without the retirement: mark the caller `Sleeping`
with its deadline, hand the CPU on, and switch **through the frame**, which is
the mechanism preemption already uses and the only place a switch is legal
(D-54). The thread resumes inside its own handler and returns to ring 3 from
there.

**`RingWait` is the call that uses both.** Nothing in flight returns zero at
once — blocking on nothing is a hang. Something already due completes without
sleeping. Otherwise the thread sleeps to the soonest deadline and completes
what it finds on waking.

Not the worker thread. `012` §8 is emphatic that io_uring's history is a march
*away* from its pool, that its bounded pool alone would be `4 × nr_cpus`, and
that this kernel has sixteen thread slots in total. A state machine driven from
the waiting thread needs zero — the direction Linux was travelling and could
not finish because it had to keep every existing filesystem working. This
kernel has none yet, which is the one moment the cheap answer is also the right
one.

**Rationale.** The completion is posted by the kernel *after* the deadline, on
the waiting thread's behalf, so a woken program finds the entry already in its
queue. That is what makes it a wait rather than a poll dressed up, and it is
the shape storage was intended to reuse without changing the ABI. D-169 and
D-181 record the delivered limit: Read has the ring entry but still polls NVMe
inline while `RingEnter` serves it.

`Queue`'s `WaitQueue` is still not what a ring-3 waiter uses, and this entry
says so rather than leaving it looking like an oversight: `WaitQueue::wait`
takes a `ThreadContext`, so it is the mechanism for a *kernel* thread waiting
on a ring. A ring-3 thread waits through the scheduler, because that is where a
trap-context switch is legal.

**Cancellation is an operation, not a substitute.** `Opcode::Cancel` has its
own reservation and its own completion. The operation it ends still gets
exactly one terminal completion, carrying `STATUS_CANCELLED` rather than a
result. `012` §8 names the double-completion class behind two io_uring CVEs,
and this is the shape that has no room for it. A cancellation naming nothing is
`STATUS_NOT_FOUND` and **not an error** — io_uring's `-EALREADY` — because an
operation that completed between the submission of the cancel and its service
is a race the program cannot avoid.

**Consequences.**

- Three opcodes: `Duplicate`, `Timer`, `Cancel`. Seven statuses where there
  were two.
- Six machine checks, seven deliberate breaks, four mutations.
- `sleep_from_trap` returns whether it actually yielded. A machine with no
  other thread to run, or a clock that never started, gets an honest zero
  rather than a hang — a wait nobody can end is worse than no wait.
- The pending table holds one entry per submission slot, which is not
  arbitrary: an in-flight operation occupies a submission's worth of admission
  until it completes, so a larger table could never be filled.
- `Pending` is a timer and nothing more. The original expectation was that a
  block-device operation would become its second kind; it did not. D-41 still
  forbids generalising the table before deferred device service actually
  supplies that caller.
