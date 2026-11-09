# D-80 A signal shares the completion plane

Status: Accepted (2026-08-24) (amended by D-181)

**Context.** D-74 named this the question hardest to answer late, and the
reason mlibc moved forward to Phase 5. `002` does not mention signals at all.
The native ABI would never raise it; a POSIX libc raises it immediately.

**Decision.** A signal is an entry posted to the completion queue. A blocking
wait is interruptible because a wakeup is indistinguishable from a completion.
There is no in-kernel signal frame, no kernel signal mask, and no
restart-after-interrupt semantics in the kernel.

**Narrowed by D-181.** This decides the event plane, not a version-1 wire
record. The current completion has no kind or opcode and every token belongs to
a submission, so no unsolicited signal entry exists yet. Phase 9 must version
that record and prove dispatch, correlation and interruptible waits without
adding a second plane.

**Rationale.** It is the same argument that unified ports and completion queues
in D-79: a second delivery mechanism means two event planes and a join problem,
and the join problem is where the bugs are. Making the signal an ordinary entry
means the interruptible-wait behaviour is a property of the queue that already
has to be correct, rather than a second path that has to be kept correct
alongside it.

**Alternatives.** A kernel-delivered signal frame on the user stack (rejected:
it is a second event plane, it needs its own reentrancy rules, and it puts
kernel-written data on a stack the process controls); a signal-specific wakeup
that aborts a queue wait (rejected: the abort has no completion, which
contradicts D-79's one-terminal-completion rule).

**Consequences.**

- mlibc's future `Sigaction` requires a userspace dispatch loop over CQ entries.
- The POSIX behaviours that assume a kernel-delivered signal interrupting an
  arbitrary instruction — `EINTR` on a blocking call, signal-safe reentrancy,
  a `SIGSEGV` handler that resumes — require explicit emulation or remain
  unsupported. Plan 9's APE and Fuchsia POSIX Lite are evidence of the cost,
  not an impossibility proof.
- `002`'s event-plane question closes; the concrete record remains open.
