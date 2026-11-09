# D-200 File reads leave trap context through one bounded worker, and NVMe waits use monotonic deadlines

Status: Accepted, 2026-08-27. Amends D-153, D-164, D-169 and D-181; closes PW-205 and PW-209.

**Context.** `Read` had the asynchronous ring entry but performed the complete
VFS and polled NVMe path inside `RingEnter`. The syscall gate has interrupts
masked, and `nvme::with_controller` held a `SpinLock`, so one delayed command
could suppress the timer and keyboard for its whole five-million-read bound.
The same driver converted `CAP.TO` and command waits to iteration counts even
after the monotonic counter had been calibrated; debug, release and different
processors therefore assigned different real durations to the same policy.
Wizzy's filesystem client is the concrete future caller of this boundary.

**Decision.** `RingEnter` copies, validates and authorizes a Read once, reserves
its terminal completion as before, records an immutable read request, and
offers it to one kernel file worker. The global work queue has eight fixed
slots and allocates nothing at admission. Full means an immediate
submission-time `STATUS_NO_ROOM`, before the device or destination is touched.
The work item holds an `Arc` to the ring, so its three shared pages and data
area remain pinned even if the program retires.

Pending operations distinguish timers, queued reads and running reads. A
queued read may be cancelled without reaching the device; a running read is
too late to cancel and still owns its promised target completion. Program
retirement cancels what has not started but retains the complete zombie while
a running read exists. Only the worker's terminal post makes that ring quiet.
`RingWait` registers on the ring's existing wait queue before rechecking its
completion generation, then switches through the live trap frame. A timer
deadline or a worker completion may wake the same wait; there is no polling
door and no second event plane.

The controller moves to a `ThreadLock`. Only the file worker performs runtime
reads, so it may be preempted while owning the controller without another
thread or interrupt handler waiting for that lock. NVMe I/O queues remain
polled with `IEN = 0`; an MSI-X path would add interrupt allocation, sharing
and controller state without removing the need for deferred VFS service.

`libs/nvme` owns a clock-or-attempt poll limit. Once the monotonic clock exists,
controller-ready waits expire at `CAP.TO` (with the existing 500 ms minimum)
and commands at the kernel's documented five-second service limit, independent
of code generation or CPU rate. Before calibration, bring-up uses the explicit
old fallback: five million command polls, and 20,000 ready polls per millisecond
capped at twenty million. A poll chooses its mode once; a missing boot clock
cannot become an accidental unbounded wait.

**Rationale.** A single worker is the smallest design which moves filesystem
and device service off the trap stack while preserving the read-only driver's
one-command discipline. The fixed queue makes pressure an admission result,
not late allocation. Keeping the existing polled hardware queue avoids
inventing interrupt routing for one controller, while the preemptible worker
removes polling from interrupt latency. Consumed completion reservations and
the queued/running distinction preserve exactly-once settlement through
cancel, completion and death.

**Alternatives.** Polling for a measured short interval inside `RingEnter` was
rejected because the syscall gate still masks interrupts and any acceptable
interval would be a latency policy disguised as an iteration count. Enabling
interrupts inside the handler was rejected because preemption cannot carry a
live trap frame away through an ordinary lock path. A worker per request or an
unbounded pool was rejected by the sixteen-slot thread budget and because
queue pressure would become a completion-time resource failure. MSI-X was
rejected for this slice because the queue was deliberately created with
interrupts disabled and no second controller exists to shape sharing.

**Consequences.** Release QEMU holds a real NVMe command after its doorbell and
observes `RingEnter` return while timer preemption and an injected keyboard IRQ
both progress. Retirement retains the ring until that running read posts one
exact completion. A second check fills all eight worker slots while the worker
is paused, sees the ninth return `STATUS_NO_ROOM`, then observes every admitted
request settle. The shipped program submits a Timer and reaches `RingWait`
through its trap frame rather than polling.

Host tests give a controlled clock arbitrarily many polls without extending
its deadline, exhaust the clockless limit at the exact attempt count, and
cover saturating deadline arithmetic. The six new witnesses were each broken
once; their persistent mutations bring the register to 136. The complete gate
passes 206 release and debug machine checks plus shipped and degraded boots.
