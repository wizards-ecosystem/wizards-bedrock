# D-80 A signal shares the completion plane

Status: Accepted, 2026-08-24. Amended by D-181.

**Decision.** A signal is an entry posted to the completion queue. A blocking
wait is interruptible because a wakeup is indistinguishable from a completion.
The kernel has no signal frame, no signal mask, and no restart-after-interrupt
semantics. D-74 named this the question hardest to answer late.

**Narrowed by D-181.** This decides the event plane, not a version-1 wire
record. Today's completion has no kind or opcode and every token belongs to a
submission, so no unsolicited signal entry exists yet. Phase 9 must version that
record and prove dispatch, correlation and interruptible waits without adding a
second plane.

**Rationale.** D-79 unified ports and completion queues on the same argument. A
second delivery mechanism means two event planes and a join problem, where the
bugs are. As an ordinary entry, the signal makes interruptible waits a property
of the queue, which already has to be correct.

**Alternatives.** A kernel-delivered signal frame on the user stack (rejected: a
second event plane with its own reentrancy rules, writing kernel data on a stack
the process controls). A signal-specific wakeup that aborts a queue wait
(rejected: the abort has no completion, against D-79's one-terminal-completion
rule).

**Consequences.**

- mlibc's future `Sigaction` requires a userspace dispatch loop over CQ entries.
- Some POSIX behaviours assume a kernel-delivered signal interrupting an
  arbitrary instruction: `EINTR` on a blocking call, signal-safe reentrancy, a
  `SIGSEGV` handler that resumes. They require explicit emulation or remain
  unsupported. Plan 9's APE and Fuchsia POSIX Lite show the cost, not an
  impossibility.
- `002`'s event-plane question closes; the concrete record remains open.
