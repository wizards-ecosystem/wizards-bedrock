# D-203 Retired programs wake one mandatory service worker for bounded reclamation

Status: Accepted, 2026-08-28. Amends D-117, D-133 and D-200; closes PW-307.

**Context.** A retired program gave its thread slot back immediately but left
its address space, mappings, ring, handles and objects in `ZOMBIES`. Only the
idle loop or the beginning of a later spawn called `collect`. Both bounded the
leak, but neither guaranteed progress: a continuously busy machine which
stopped creating processes could retain the last dead program forever. The
future Wizzy process manager is the concrete caller which needs exit to imply
eventual resource return without manufacturing another spawn.

**Decision.** The bounded file worker becomes the kernel service worker. It
continues to own every deferred Read and additionally consumes one reclamation
request per retired program. `orphan` moves ownership into `ZOMBIES`, increments
a counted request generation and wakes the worker through its existing wait
queue. That wake is legal from trap context and uses D-133's ordinary unblock
path, which makes the service thread runnable and arms the scheduler.

The worker performs at most one collection and one file operation per loop,
so sustained process death cannot starve reads and sustained reads cannot
starve reclamation. It collects only after the dying thread has switched away;
the retirement path still frees nothing on its own stack or address space.
Idle and pre-spawn collection remain safe opportunistic fallbacks. Because
this progress guarantee depends on the worker, process construction refuses
before allocation unless the worker is ready; a boot which could not reserve
that kernel thread does not open ring 3.

**Rationale.** One existing preemptible thread is enough. A dedicated reaper
would permanently spend another of sixteen slots and duplicate wait/wake
machinery, while collecting at trap exit would reclaim the stack and tables
currently executing. Polling from the timer would move allocation and object
destruction into handler context. A counted wake retains every event even if
several programs retire before the worker runs, unlike a boolean hint.

**Alternatives.** Leaving the next-spawn fallback as the progress contract was
rejected because workloads may stop spawning after any exit. Scheduling the
idle class periodically was rejected because it would no longer mean “only
when no normal work exists” and would reintroduce idle timer traffic. Giving
file work unconditional priority was rejected because a full producer could
starve cleanup; giving cleanup unconditional priority was rejected for the
same reason in reverse.

**Consequences.** Release QEMU first injected missing worker readiness and
observed typed `NoServiceWorker` refusal before any resource or syscall-count
change. With the real worker, a normal spinner kept the machine busy while a
program retired without a later spawn or direct collection; the service worker
reclaimed its pages and tables, restored the complete spawn-resource ledger,
and advanced its own reap count while the idle halt count remained unchanged.
Bypassing the launch prerequisite and separately removing the retirement wake
each reddened only `a busy machine reaps a program without another spawn` with
209/210. The two persistent mutations bring the register to 142 entries and
the complete gate carries 210 machine checks. D-204's final gate exposed one
publication-order race in this older protocol: the retirement generation
advanced before `orphan` recorded the program's last rites, so an acquiring
observer could see retirement without its promised evidence. Publishing the
generation after `orphan` closes that window; the existing bootstrap-ownership
check caught the real race with 210/211 before the repair.
