# D-109 The thread table and the run queue are separate structures

Status: Accepted, 2026-08-24.

**Context.** `static THREADS` held one `Thread` per slot, carrying what a thread
*is* (name, entry point) and everything the policy needs about it. Phase 4 was
right to start there: one array, one lock, no boundary to keep honest. Phase 5
gives a slot a second reason to change. `libs/eevdf` is already domain-agnostic
and host-tested, so the seam that would let it be replaced is worth cutting
before more code is written across it.

**Options.** Leave it; split the lock; or split the data.

**Decision.** Split the data and keep one lock. `Thread` became `Identity`
(name, entry) and `Scheduled` (state, class, entity, timings). The ten
scheduling methods moved to `impl RunQueue`, which holds the `Scheduled` array
and nothing else. `Threads` owns both halves under the one lock it always had.

**Rationale.** On one CPU, splitting the lock changes correctness for no
benefit. Every runnability change happens with the other half settled, so two
locks would add an order to get right and a deadlock to find. Splitting the data
is free at runtime and gives the property that matters: **`RunQueue` cannot name
a thread**, because `Scheduled` has no field to name it with. Swapping the
policy changes that impl and that struct, and the compiler says so.

Slot lifetime spans both halves, so it got one call: `Threads::release` clears
both. A slot free in one half only is a thread with no name or a name with no
thread, and that used to be two assignments a caller could half-perform.

**Consequences.** There is no check. The property is *structural*, enforced by
`Scheduled` having no name field, so breaking it means adding one, a visible
edit. The 112 machine checks all pass unchanged, as a refactor with no behaviour
change should.

Left owed: `STACK_SLOTS = 16` is still a compile-time default, and the arena is
still mapped at boot rather than per thread. D-86 and D-88 made per-thread
mapping possible; nothing does it yet.
