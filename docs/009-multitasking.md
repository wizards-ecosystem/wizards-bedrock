# 009 — Multitasking (Phase 4)

The phase that gives the kernel more than one thing to do: kernel threads
with guarded stacks, a context switch the type system can vouch for, an
EEVDF-style scheduler behind a separable policy seam, and — once the
scheduler knows when it next needs the clock — the retirement of the 100 Hz
tick that D-36 called explicitly disposable.

Phase 3 left a kernel that owns its memory but not its time. Everything after
this phase (userspace, channels, the executor) assumes something can block and
something else can run, so multitasking is the last phase that can be built
single-threaded. This file was the phase's charter, written before its code per
D-19, and records each slice as it was built.

References: `008-research-notes-2.md` §1 (naked functions, verified against
the pinned toolchain), §4.2 (Klint and the context-token idea), §4.3
(pin-init and address-stable queues), §4.4 (EEVDF mechanics, Redox's 250 ns
datapoint, sched_ext's separability lesson); Linux's EEVDF scheduler docs
and LWN 969062; Asterinas OSTD's task/scheduler split (the framekernel
precedent D-13 follows); Intel SDM Vol. 3A ch. 7 (hardware task management,
for what is deliberately not used) and Vol. 3A ch. 11 (APIC timer modes).

Quantities below are dated proposal or slice-close observations. The
phase-close total is the total at the end of Phase 4, not a promise that later
phases retain it; `014-storage.md` owns the checked current gate totals.

## What earlier phases left owing

| Debt | Where recorded | What Phase 4 does |
|---|---|---|
| 100 Hz periodic tick, "explicitly disposable when D-15 lands" | D-15, D-36; `006-cpu-fundamentals.md` ("D-34's latency cost wearing a clock face") | ✅ Retired by D-57: one-shot programming, and no timer armed when nothing needs waking |
| A critical section longer than a tick swallows one; uptime undercounts 10 ms | D-36 consequences | ✅ Discharged by D-57: a late deadline is delivered, not lost, and uptime is the timestamp counter rather than an interpolated countdown |
| Interrupt latency bounded by the longest critical section — "a scheduler's problem and waits for one" | D-34; `wost/src/sync.rs` module doc | ✅ Fixed by D-63: measured at seventy milliseconds of console scroll, then taken off the latency path by a lock that keeps interrupts on |
| Lock guard-discipline parameterisation "earns its keep once there is a scheduler" | D-34 rationale (Asterinas `PreemptDisabled`) | ✅ Declined for preemption in D-54, then adopted for latency in D-63: two lock kinds, not a type parameter |
| No allocation in IRQ handlers, enforced by nobody; "worth an ADR when the heap lands" | `003`; `008` §4.2 | ✅ D-55 made it a count the heap asserts on; D-62 made it a token the compiler keeps |
| `HEAP → FRAMES` is the one lock order; "the inversion is the one deadlock this pair can produce" | D-47 | The scheduler adds a lock; this file's lock-order section becomes the total order's authority |
| XSAVE "needs a scheduler; the kernel is built `+soft-float`" | D-29 | Still deferred: kernel threads are soft-float too, so there is no FPU state to switch until ring 3 (Phase 5) |
| Per-CPU stacks add layout assumptions — interest on the KASLR debt | D-46 | The stack arena is one more hardcoded window; the ADR that places it adds the entry to D-46's ledger |
| The address space cannot grow after boot; Phase 5's address-space work "should take that decision on purpose, not by accident" | D-45, D-49 | Phase 4 lives *within* the constraint, with stacks provisioned at table-build time, and does not pre-empt Phase 5's decision |
| SMP TLB shootdown, owed if anything ever unmaps | D-45 | Stays dormant: the arena maps at boot and unmaps nothing |
| Keyboard input drains to the log; nothing can block on a key | D-49 retired polling, not blocking | ✅ D-61: a wait queue, a blocking read, and the interrupt wakes the reader |

Not this phase's debt: the handle-table and rights-taxonomy doc and the
queue-layout doc were owed **before Phase 5** (`002`, open questions), and
KASLR waits for old Phase 12 (D-46).

## Slices

The order is forced: nothing can schedule without a context switch, nothing
should preempt a switch that has not been proven cooperatively, tickless
needs a scheduler that knows the next event, and blocking needs
wake-from-interrupt, which needs the preemption-era lock discipline settled.

| Slice | Contents | Status |
|---|---|---|
| 1 | Kernel threads, cooperative: stack arena with guard gaps, TCB table, `spawn`/`yield_now`/`exit`, naked-function context switch in `wost`, round-robin ready list (deliberately policy-free) | ✅ 2026-08-22 |
| 2 | The scheduler: `libs/eevdf` arithmetic host-tested, EEVDF policy behind a separable seam with round-robin fallback, timer preemption at trap exit, the lock order and the context token | ✅ 2026-08-22 |
| 3 | Tickless: one-shot APIC programming, sleep queue and `sleep_ms`, the 100 Hz tick retired, uptime re-based | ✅ 2026-08-23 |
| 4 | Blocking: `WaitQueue`, the keyboard read blocks a thread and the interrupt wakes it, idle becomes the idle thread, D-34's latency position taken | ✅ 2026-08-23 |
| 5 | Phase close: VMware boot (D-18), the soundness audit (D-76), roadmap and docs updated, this file's closing section | ✅ 2026-08-23 |

## The phase's decisions

- **The context switch** is fifteen instructions in a `#[unsafe(naked)]`
  function (D-51); the existing `global_asm!` interrupt trampoline stays as it
  is.
- **Stacks under one-way mapping** come from a boot-time arena (D-52). The
  constraint itself was retired in Phase 5 by D-86.
- **Preemption** is cooperative first, then the timer at trap exit (D-54).
- **The policy seam** is `libs/eevdf` with a round-robin fallback (D-53),
  D-15's "a cheap future door, not a v1 feature" made separable and
  observable.
- **The allocation-safety context token** was deferred in D-55 to a count,
  then taken in D-62 when the third blocking API made a token cheaper than the
  asserts.
- **Per-CPU state** is one word shaped to widen (D-56), which names the three
  statics that become per-CPU when SMP arrives; D-29 left `swapgs` for
  ring 3.
- **The tickless timebase** is the timestamp counter (D-57), on the same
  window, with accumulated deadlines rejected because an idle machine's clock
  would stop. D-225 later adds CPUID 0x15 selection and retains the PIT as an
  APIC-delivery-sampled validator; two conclusive disagreements revoke public
  time and unwind sleepers/Timer promises. PIT samples then advance a private
  scheduler epoch so already-running programs remain preemptible, while a
  machine with no deadline still arms nothing.

## The lock order

**Superseded by `011-userspace.md`'s section of the same name.** What is below
describes Phase 4 as it was built and is kept for that; two things in it are no
longer true. D-85 replaced the prose discipline with the borrow checker and
made `lock_leaf` the acquisition a `Drop` can take, and it retired
`HEAP → FRAMES` altogether by moving the heap's refill out from under its own
guard. What remains inside `wost` are the closure-taking accessors —
`mm::with_memory_map` is the one that matters — where the obligation is on the
caller's closure rather than on the type.

D-47 established `HEAP → FRAMES`: the heap takes the frame allocator's lock
during a slab refill, never the reverse. The kernel's thread table, `THREADS`, enters
as a **leaf**: while holding it, take nothing — not `HEAP`, not `FRAMES`, not
a console lock, not even `wost`'s own task-table lock, which `switch_to` and
`fabricate` take on their own and which is itself a leaf. Spawn does its
allocating before it touches `THREADS`; every switch happens after its guard
has dropped; a wake takes `THREADS` alone, which is what makes it legal from
trap context, where D-31's no-logging rule still binds.

A wait queue's lock is a **second leaf**, not an edge: registering as a
waiter and ceasing to be runnable are separate holds, and what makes them one
step is the interrupt flag, clear across both. On one CPU that is exclusion;
SMP owes the pair a second look.

Slice 4 adds an ordering between lock *kinds* as well as locks (D-63). A
`ThreadLock` may never be taken while a `SpinLock` is held — the holder has
interrupts off, so a `ThreadLock` whose owner has been preempted can never be
released. This one is not on trust: `SpinLock` counts its holders and
`ThreadLock` asserts the count is zero. It is also why the two consoles are
`ThreadLock`s and everything a handler touches is a `SpinLock`.

## How it is checked

Arithmetic is host-tested before hardware sees it (D-30), every new mechanism
lands with an on-machine check in `kernel/src/selftest.rs` (D-28), each broken
once to prove it can go red, with QEMU as the loop and a VMware boot to end the
phase (D-18). No identity comparisons, no dependence on allocator or codegen
ordering, no early return that makes the failure path unreachable. **A check
that has never failed is not evidence; it is an untested claim.** A break has
to reach the path the check watches and fail it: a break that crashes or hangs
the machine proves nothing about the check. A wake check measures from inside
the thing it is about, because an outcome cannot distinguish a working wake
from a working poll (D-49's lesson).

## Slice 1 — threads and the switch (2026-08-22)

The kernel never holds a context. `wost::task` owns a fixed table of slots, each with a stack from the arena and
a saved stack pointer; the kernel names slots by index and asks for
transitions (`fabricate`, `switch_to`, `exit_to`), every one of which is
checked before it happens. That is D-51's substance: a `Context` value the
kernel could copy would make the sound-wrapper question unanswerable, and an
index into a table that refuses dead slots answers it.

The stacks were forced into an arena by D-45, not chosen (D-52): sixteen slots
of 16 KiB above a 4 KiB gap that is never mapped, committed while the tables are
being built, with the arithmetic in `libs/stack-layout` host-tested for the one
property that matters, that no two slots touch. `task::init` walks every stack
in the live tables before it will fabricate on one. `008` §4.1 warns that a
frame larger than the guard can jump it.

`thread.rs` is a table of names and states over the same indices and a fixed
ring of who runs next, with no allocation and a leaf lock. `spawn` reserves a
slot under the lock, drops it, then asks `wost` to fabricate; `yield_now` and
`exit` choose the next slot under the lock and switch after the guard drops.
Nothing at this slice enforced that a thread holds no lock when it yields; that
cannot corrupt memory, only hang, and D-55's context token is where the type
system takes that over.

## Slice 2 — the scheduler (2026-08-22)

`libs/eevdf` is the whole of EEVDF as pure functions: wrapping virtual time,
weighted charging, the fair-share average, eligibility, the earliest-deadline
pick, and lag-clamped placement (D-53). `RESOLUTION` exists because one unit of
real time at twice the base weight truncated to zero virtual time. The kernel's
side of the seam feeds the policy the runnable entities in rotation from just
after the running thread, so equals take turns, and counts the one answer the
policy must never give.

Preemption is the timer calling the scheduler after its end-of-interrupt, and
the gate is the flag the CPU pushed (D-54). Code that was not accepting
interrupts is not switched out, and since D-34 clears the flag for every lock,
that single test makes every critical section unpreemptible with no count to
maintain. `switch_from_trap` in `wost` refuses a frame whose flag is clear, so
the rule is checked where the switch happens rather than trusted from where it
is called. Asterinas's `PreemptDisabled` discipline is declined on the record:
a lock already is one.

Trap context became a number (D-55). The heap refuses to allocate while a
handler is on the stack and the scheduler refuses to yield; a switch out of a
handler parks the count with the thread that owns it.

The round-robin fallback belongs only to the preemption path, where everyone is
considered and the policy's own tests promise an answer. The yield path
excludes the running thread from the candidates but not from the average, so
"nobody else is eligible" is a correct answer there, and the yielder keeps the
CPU.

## Slice 3 — tickless (2026-08-23)

**Two clocks doing two jobs** (D-57). The timestamp counter free-runs, so it
is the clock: `now_ns` is a subtraction and a multiply, with nothing to
interpolate and no handler to keep it moving. The APIC timer is the alarm:
one-shot, armed for the running thread's slice ending or the earliest sleeper's
deadline, whichever is sooner, and stopped when there is neither. Both rates
come out of D-36's twenty-millisecond window; `libs/timebase` holds a rounded
multiply-and-shift held to thirty-two bits so a whole conversion packs into
one word and can be published to a reader inside an interrupt handler.

Accumulated deadlines were rejected: while nothing is armed, nothing accrues,
and an idle machine's clock stops. Closing that hole means a housekeeping
timer, which is the tick wearing a different name.

D-36's monotonic clamp went with the countdown it guarded; monotonicity is now
the counter's claim, and "uptime never runs backwards" went red on its first
break.

**Sleeping** (D-58) is a deadline, a saved lag, and a state. Every scheduling
path wakes what is due before deciding anything, and a woken thread comes back
at the fair-share point offset by the lag it left with. A thread that sleeps
with nothing else runnable halts in place on one `sti`-`hlt` pair, whose shadow
is what stops the wakeup landing in the gap.

**A thread's first interrupt flag comes from its creator** (D-59). Taking it
from the context that switched to it meant every thread first started from a
yield ran uninterruptible, because that context is a scheduler, which clears
the flag around every decision.

An idle kernel takes no timer interrupts: none, against the fifty the retired
tick would have taken in the same half second.

## Slice 4 — blocking (2026-08-23)

`WaitQueue` is a bitmask of thread slots in the queue itself, with no
allocation, because the waker is an interrupt handler and D-55 forbids it the
heap (D-61). The keyboard queues decoded events in a fixed ring and wakes the
readers; the boot thread, having finished booting, blocks on `read_key`. A
kernel idle at a prompt takes exactly one interrupt per keystroke and arms
nothing between them.

A wake needs a reschedule, and trap exit is the only place one is legal, so the
keyboard handler asks the scheduler after its end-of-interrupt, exactly as the
timer does (D-54).

The idle thread is outside the policy, not at the bottom of it (D-60): a
weight small enough to ignore is still a weight, and eligibility would hand an
idle-weighted thread the CPU now and then. So it has a class the policy never
sees, and the tickless claim stays exact.

D-55's token arrived with the API that needed it (D-62): three calls can
block, which makes a `ThreadContext` argument cheaper than three run-time
asserts.

D-34's latency debt was measured and fixed (D-63). The longest critical section
was a console scroll: seventy milliseconds of framebuffer under one lock, with
interrupts off for all of it. The console keeps its lock but not the interrupt
flag, and refuses trap context instead, enforcing D-31's prose rule with D-55's
trap depth. The keyboard handler's log line and echo moved to the reading
thread. Putting the console back on a `SpinLock` hung the machine: an
interrupts-on lock taken inside an interrupts-off one, whose preempted holder
could never be scheduled. Hence the lock-kind rule above.

**Still owed at this phase close.** A wait with a timeout, and a wait on two
queues; neither had a caller yet. D-164 later built the timed `RingWait` once
the completion queue supplied that caller. A wait on multiple queues remains
absent because no caller requires it.

## Slice 5 — the phase close (2026-08-23)

**The VMware boot settled** (D-18): the shipped kernel reached its keyboard
prompt on VMware Workstation at 5 ms of uptime, with the TSC advertised
invariant.

**The close-out audit became D-76.** Asking D-13's question adversarially —
could *any* safe caller break `wost`, not just this kernel — found five holes,
one of them this phase's own: the fabricated first frame was a word short, so
every spawned thread ran with `rsp` shifted eight from the ABI's promise. The
frame grew its padding word, and `a thread starts aligned` now stands where the
gap was. The other four, and the residue deliberately left for Phase 5's memory
objects, are D-76's record. That checkpoint had seventy-five machine checks;
D-77's completed-phase audit added four and closed Phase 4 at seventy-nine.

**What the phase left open at this close**, besides D-76's residue: a timed
wait and a wait on multiple queues; unguarded boot and double-fault stacks; and
the weight plumbing beyond `spawn_weighted`, which waited for a caller with an
opinion. D-248 later removed that uncalled production seam; weight injection is
self-test-only until policy has a real owner. Later work paid the timed wait in
D-164 and guarded the double-fault and boot stacks in D-89 and D-92
respectively. The multi-queue wait remains uncalled; the historical close is
not a claim that those paid debts are open now.
