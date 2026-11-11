# D-109 The thread table and the run queue are separate structures

Status: Accepted, 2026-08-24.

**Decision.** Split the data and keep one lock. `Thread` became `Identity`
(name, entry) and `Scheduled` (state, class, entity, timings). The scheduling
methods live in `impl RunQueue`, which holds the `Scheduled` array and nothing
else. `Threads` owns both halves under the one lock it always had.

Slot lifetime spans both halves, so it gets one call: `Threads::release` clears
both. A slot free in one half only is a thread with no name or a name with no
thread.

**Rationale.** On one CPU, splitting the lock changes correctness for no
benefit: two locks would add an order to get right and a deadlock to find.
Splitting the data is free at runtime and gives the property that matters:
**`RunQueue` cannot name a thread**, because `Scheduled` has no field to name it
with, so swapping the policy changes that impl and that struct, and the
compiler says so.

**Consequences.** There is no check. The property is *structural*, enforced by
`Scheduled` having no name field, so breaking it means adding one, a visible
edit.

Left owed: `STACK_SLOTS = 16` is still a compile-time default, and the arena is
still mapped at boot rather than per thread. D-86 and D-88 made per-thread
mapping possible; nothing does it yet.
