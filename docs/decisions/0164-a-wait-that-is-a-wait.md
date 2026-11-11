# D-164 A wait that is a wait

Status: Accepted, 2026-08-25. Amended by D-181.

**Decision.** A ring-3 thread can wait for an operation whose answer is *not
yet*, through three pieces:

- **An operation can be deferred.** `Opcode::Timer` is admitted, recorded in a
  small pending table, and not completed. Its completion slot was reserved at
  admission and stays reserved, which makes the later completion certain rather
  than hopeful: the reservation is the difference between deferring an
  operation and dropping one.
- **A thread can sleep from a trap.** The syscall is served in trap context
  through an interrupt gate (D-96), where `sleep_ns` cannot be used because
  there is no `ThreadContext` inside a handler (D-55). `thread::sleep_from_trap`
  is `retire_from_trap`'s shape without the retirement: mark the caller
  `Sleeping` with its deadline, hand the CPU on, and switch **through the
  frame**, the mechanism preemption uses and the only place a switch is legal
  (D-54). The thread resumes inside its own handler and returns to ring 3 from
  there.
- **`RingWait` uses both.** Nothing in flight returns zero at once, because
  blocking on nothing is a hang. Something already due completes without
  sleeping. Otherwise the thread sleeps to the soonest deadline and completes
  what it finds on waking.

The ring is not served from a kernel worker thread. `012` §8 shows io_uring's
history is a march away from its pool, whose bounded size alone would be
`4 × nr_cpus` against this kernel's sixteen thread slots; a state machine driven
from the waiting thread needs zero, and with no filesystems to keep working this
is the moment the cheap answer is also the right one.

**Cancellation is an operation, not a substitute.** `Opcode::Cancel` has its
own reservation and its own completion. The operation it ends still gets
exactly one terminal completion, carrying `STATUS_CANCELLED` rather than a
result, which leaves no room for the double-completion class behind two io_uring
CVEs (`012` §8). A cancellation naming nothing is `STATUS_NOT_FOUND` and **not
an error** (io_uring's `-EALREADY`), because an operation completing between the
cancel's submission and its service is a race the program cannot avoid.

**Rationale.** The kernel posts the completion after the deadline on the waiting
thread's behalf, so a woken program finds the entry already in its queue. That
makes it a wait rather than a poll dressed up, and it is the shape storage was
intended to reuse without changing the ABI. D-169 and D-181 record the
delivered limit: Read has the ring entry but still polls NVMe inline while
`RingEnter` serves it.

**Consequences.**

- Three opcodes: `Duplicate`, `Timer`, `Cancel`. Seven statuses where there
  were two.
- `Queue`'s `WaitQueue` is not what a ring-3 waiter uses: `WaitQueue::wait`
  takes a `ThreadContext`, so it is for a *kernel* thread waiting on a ring. A
  ring-3 thread waits through the scheduler, because that is where a
  trap-context switch is legal.
- `sleep_from_trap` returns whether it actually yielded. A machine with no other
  thread to run, or a clock that never started, gets an honest zero rather than
  a hang: a wait nobody can end is worse than no wait.
- The pending table holds one entry per submission slot, because an in-flight
  operation occupies a submission's worth of admission until it completes, so a
  larger table could never be filled.
- `Pending` is a timer and nothing more. The original expectation was that a
  block-device operation would become its second kind; it did not. D-41 still
  forbids generalising the table before deferred device service actually
  supplies that caller.
