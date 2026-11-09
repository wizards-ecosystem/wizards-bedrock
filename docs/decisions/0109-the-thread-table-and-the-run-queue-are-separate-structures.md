# D-109 The thread table and the run queue are separate structures

Status: Accepted (2026-08-24)

**Context.** `static THREADS` held one `Thread` per slot carrying both what a
thread *is* — its name and entry point — and everything the policy needs about
it. Phase 4 was right to start there: one array, one lock, no boundary to keep
honest. Phase 5 gives a slot a second reason to change, and `libs/eevdf` is
already domain-agnostic and host-tested, so the seam that would let it be
replaced is worth cutting before more code is written across it.

**Options.** Leave it; split the lock; or split the data.

**Decision.** Split the data, not the lock. `Thread` became `Identity` (name,
entry) and `Scheduled` (state, class, entity, timings), and the ten scheduling
methods moved to `impl RunQueue`, which holds the `Scheduled` array and nothing
else. `Threads` owns both halves under the one lock it always had.

**Rationale.** Splitting the lock would be a change to correctness for no
benefit on one CPU: every decision that changes runnability happens with the
other half settled, so two locks would mean an order to get right and a
deadlock to find. Splitting the data costs nothing at runtime and buys the
property that matters — **`RunQueue` cannot name a thread**, because
`Scheduled` has no field to name it with. Swapping the policy is a change to
that impl and to that struct, and the compiler says so.

Slot lifetime is the one thing that spans the halves, so it got one call:
`Threads::release` clears both. A slot free in one and not the other is a
thread with no name or a name with no thread, and that used to be two
assignments a caller could half-perform.

**Consequences.** No check, and this is the honest version rather than an
omission: the property is *structural*, enforced by `Scheduled` having no name
field, so the way to break it is to add one — a visible edit, not a silent
regression. The 112 machine checks all pass unchanged, which is what a refactor
with no behaviour change should look like.

Left owed: `STACK_SLOTS = 16` is still a compile-time default, and the arena is
still mapped at boot rather than per thread. D-86 and D-88 made the second one
possible; nothing yet does it.
