# D-59 A thread's first interrupt flag comes from its creator

Status: Accepted (2026-08-23)

**Context.** A thread resuming after a switch restores its own flag from its
own stack. A thread running for the first time has no such record, and D-51's
`wost::task` gave it the switcher's, "the same rule a lock guard follows: put
back what was found". The switcher is a scheduler, and a scheduler clears the
flag around every decision it makes.

**What that cost.** Every thread whose first run came from `switch_to` — the
yield path, and now the sleep path — started with interrupts disabled and
could never get them back. Nothing noticed from slice 1 until slice 3, because
the checks that used the yield path had their threads yielding constantly, and
the check that needed preemption got its thread started by the preemption path
instead, where the flag handed over is the interrupted code's and is set. The
symptom, when it came, was a hang: a thread slept, handed the CPU to a spinner
that could not be interrupted, and the machine stopped.

**Decision.** The flag is captured in `fabricate`, from the thread's creator,
and restored by the entry trampoline. It cannot be captured at the switch: by
then the only flag left to read is the scheduler's own.

**Rationale.** A new thread holds no lock and has nothing to protect, so there
is no state it inherits that would justify running uninterruptible. The one
thing worth inheriting is whether the machine was accepting interrupts at all
when the thread was made — which is what its creator's flag says, and which is
correctly `false` on a machine where interrupts never came on, where
unconditionally enabling them would panic.

**Consequences.**

- `HANDOFF_INTERRUPTS` is gone, and with it the only entry on D-56's per-CPU
  list that was not really per-CPU: this belongs to a slot, not a processor.
- A thread spawned from inside a critical section still starts
  uninterruptible. That is the inheritance rule applied honestly rather than
  an oversight, and it is deterministic and documented; nothing in the kernel
  spawns from one.
- One check, broken once: a spawned thread records the flag on its first
  instruction, and it must be set. Spawned from a yield, because that is the
  path that got it wrong — the preemption path would have passed either way.
  Breaking it reproduced the original hang exactly.
