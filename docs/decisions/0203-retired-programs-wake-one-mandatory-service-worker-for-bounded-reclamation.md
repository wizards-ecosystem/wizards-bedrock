# D-203 Retired programs wake one mandatory service worker for bounded reclamation

Status: Accepted, 2026-08-28. Amends D-117, D-133 and D-200; closes PW-307.

**Decision.** The bounded file worker becomes the kernel service worker. It
continues to own every deferred Read and additionally consumes one reclamation
request per retired program. `orphan` moves ownership into `ZOMBIES`, increments
a counted request generation and wakes the worker through its existing wait
queue. That wake is legal from trap context and uses D-133's ordinary unblock
path, which makes the service thread runnable and arms the scheduler. The
retirement generation is published after `orphan` records the program's last
rites, so an acquiring observer never sees retirement without its promised
evidence.

The worker performs at most one collection and one file operation per loop,
so sustained process death cannot starve reads and sustained reads cannot
starve reclamation. It collects only after the dying thread has switched away;
the retirement path still frees nothing on its own stack or address space.
Idle and pre-spawn collection remain safe opportunistic fallbacks. Because
this progress guarantee depends on the worker, process construction refuses
before allocation unless the worker is ready; a boot which could not reserve
that kernel thread does not open ring 3.

**Rationale.** Only idle or a later spawn had called `collect`, so a busy
machine that stopped creating processes could keep the last dead program's
space, ring, handles and objects for ever; the future Wizzy process manager
needs exit to imply eventual resource return. One existing preemptible thread
is enough. A dedicated reaper would permanently spend another of sixteen slots
and duplicate wait/wake machinery, while collecting at trap exit would reclaim
the stack and tables currently executing. Polling from the timer would move
allocation and object destruction into handler context. A counted wake retains
every event even if several programs retire before the worker runs, unlike a
boolean hint.

**Alternatives.** Leaving the next-spawn fallback as the progress contract was
rejected because workloads may stop spawning after any exit. Scheduling the
idle class periodically was rejected because it would no longer mean “only
when no normal work exists” and would reintroduce idle timer traffic. Giving
file work unconditional priority was rejected because a full producer could
starve cleanup; giving cleanup unconditional priority was rejected for the
same reason in reverse.

**Consequences.** Release QEMU injects missing worker readiness and observes
typed `NoServiceWorker` refusal before any resource or syscall-count change.
With the real worker, a normal spinner keeps the machine busy while a program
retires without a later spawn or direct collection; the service worker reclaims
its pages and tables, restores the complete spawn-resource ledger, and advances
its own reap count while the idle halt count stays unchanged. Bypassing the
launch prerequisite and separately removing the retirement wake each redden
only `a busy machine reaps a program without another spawn`, and both have
persistent mutations. D-204's final gate exposed the publication-order race;
the existing bootstrap-ownership check caught it before the repair.
