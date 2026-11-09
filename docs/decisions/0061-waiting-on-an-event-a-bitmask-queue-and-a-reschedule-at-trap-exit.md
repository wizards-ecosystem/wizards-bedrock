# D-61 Waiting on an event: a bitmask queue, and a reschedule at trap exit

Status: Accepted, 2026-08-23.

**Context.** D-58 gave the kernel a wait for a *time*. The keyboard needs a
wait for an *event*: D-49 routed the line and retired polling, and the debt it
left was that nothing could block on a key.

**Decision.** `WaitQueue` is a bitmask of thread slots in the queue itself.
`wait` registers, blocks, and hands the CPU on; `wake_all` takes the mask and
makes those slots ready. No allocation anywhere on the path, because the waker
is an interrupt handler and D-55 forbids it the heap. A woken thread is placed
back at the fair-share point offset by the lag it parked with — the same rule
sleepers get.

The keyboard queues decoded events in a fixed ring and wakes the readers. The
kernel's own last act is to become one: the boot thread blocks on `read_key`
and draws what arrives.

**A wake needs a reschedule, and trap exit is where one is legal.** A thread
made runnable by a keystroke has no appointment to be woken at — there may be
no next expiry at all — so the keyboard handler asks the scheduler after its
end-of-interrupt, exactly as the timer does, and only when something woke.

**Rationale.** The lost-wakeup race is the classic bug here, and the fix is
that checking the queue and registering as a waiter happen together under a
clear interrupt flag. A queue's lock and `THREADS` are never held at once, so
the lock order gains a second leaf rather than an edge.

**Consequences.**

- A kernel at a prompt takes exactly one interrupt per keystroke. Nothing is
  armed for a thread with no deadline, so the machine is genuinely idle
  between them — which is the whole of Phase 4 in one sentence.
- Two checks. A blocked reader is off the run queue *and* the machine is quiet
  while it waits; then it wakes with the key the harness pressed. D-49's lesson
  shaped both: the first version of the second check claimed the reader had
  not polled and could not tell, and a deliberately polling `read_key` passed
  it. It now records whether it blocked, across its own call.
- The dropped-event counter should stay at zero and is checked. Sixteen
  events is generous against a reader that blocks; a burst larger than that
  means nobody is reading.
- Still owed: nothing can wait on a queue with a timeout, and nothing can wait
  on two. Both wait for a reason to exist.
