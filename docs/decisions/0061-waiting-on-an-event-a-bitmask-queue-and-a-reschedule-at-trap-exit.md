# D-61 Waiting on an event: a bitmask queue, and a reschedule at trap exit

Status: Accepted, 2026-08-23.

**Context.** D-58 gave the kernel a wait for a *time*. The keyboard needs a
wait for an *event*: D-49 routed the line and retired polling, leaving the
debt that nothing could block on a key.

**Decision.** `WaitQueue` is a bitmask of thread slots in the queue itself.
`wait` registers, blocks, and hands the CPU on; `wake_all` takes the mask
and readies those slots. The path allocates nothing: the waker is an
interrupt handler, and D-55 forbids it the heap. A woken thread returns at
the fair-share point offset by its parked lag, as sleepers do.

The keyboard queues decoded events in a fixed ring and wakes the readers.
The kernel's last act is to become one: the boot thread blocks on
`read_key` and draws what arrives.

**A wake needs a reschedule, and trap exit is where one is legal.** A thread
made runnable by a keystroke has no appointed wake time, and there may be
no next expiry. So the keyboard handler asks the scheduler after its
end-of-interrupt, as the timer does, and only when something woke.

**Rationale.** The lost-wakeup race is the classic bug here; the fix is
checking the queue and registering as a waiter together under a clear
interrupt flag. A queue's lock and `THREADS` are never held at once, so the
lock order gains a second leaf, not an edge.

**Consequences.**

- A kernel at a prompt takes exactly one interrupt per keystroke. Nothing is
  armed for a thread with no deadline, so the machine is idle between them,
  which is Phase 4 in one sentence.
- Two checks. A blocked reader is off the run queue *and* the machine is
  quiet while it waits; then it wakes with the key the harness pressed.
  D-49's lesson shaped both. The second check's first version claimed the
  reader had not polled but could not tell, and a polling `read_key` passed
  it. It now records whether it blocked, across its own call.
- The dropped-event counter should stay at zero and is checked. Sixteen
  events is generous for a blocking reader; a larger burst means nobody is
  reading.
- Still owed: nothing can wait on a queue with a timeout, and nothing can
  wait on two. Both wait for a reason to exist.
