# 009 — Multitasking (Phase 4)

The phase that gives the kernel more than one thing to do: kernel threads
with guarded stacks, a context switch the type system can vouch for, an
EEVDF-style scheduler behind a separable policy seam, and — once the
scheduler knows when it next needs the clock — the retirement of the 100 Hz
tick that D-36 called explicitly disposable.

Phase 3 left a kernel that owns its memory but not its time: one flow of
control, interrupted but never preempted, idling in a halt loop that a
scheduler will claim. Everything after this phase (userspace, channels, the
executor) assumes something can block and something else can run, so
multitasking is the last phase that can be built single-threaded.

This file is the phase's charter, written before its code per D-19. As with
`007-memory.md`, each slice's design is recorded below **as it is built**,
so the file ends the phase as the description of what exists; the slice
table is the dependency-forced proposal, refined when the phase starts.

References: `008-research-notes-2.md` §1 (naked functions, verified against
the pinned toolchain), §4.2 (Klint and the context-token idea), §4.3
(pin-init and address-stable queues), §4.4 (EEVDF mechanics, Redox's 250 ns
datapoint, sched_ext's separability lesson); Linux's EEVDF scheduler docs
and LWN 969062; Asterinas OSTD's task/scheduler split (the framekernel
precedent D-13 follows); Intel SDM Vol. 3A ch. 7 (hardware task management,
for what is deliberately not used) and Vol. 3A ch. 11 (APIC timer modes).

## What earlier phases left owing

Every debt below is on the record elsewhere; this table is where a future
session finds them collected.

| Debt | Where recorded | What Phase 4 does |
|---|---|---|
| 100 Hz periodic tick, "explicitly disposable when D-15 lands" | D-15, D-36; `006-cpu-fundamentals.md` ("D-34's latency cost wearing a clock face") | ✅ Retired by D-57: one-shot programming, and no timer armed when nothing needs waking |
| A critical section longer than a tick swallows one; uptime undercounts 10 ms | D-36 consequences | ✅ Discharged by D-57: a late deadline is delivered, not lost, and uptime is the timestamp counter rather than an interpolated countdown |
| Interrupt latency bounded by the longest critical section — "a scheduler's problem and waits for one" | D-34; `wost/src/sync.rs` module doc | The phase takes a position in slice 4: fix, or accept with the bound named |
| Lock guard-discipline parameterisation "earns its keep once there is a scheduler" | D-34 rationale (Asterinas `PreemptDisabled`) | Revisited when preemption lands (slice 2); adopted or declined on the record |
| No allocation in IRQ handlers, enforced by nobody; "worth an ADR when the heap lands" | `003`; `008` §4.2 | The heap landed in D-47, so the ADR is due: a context token in the `PanicToken` idiom (slice 2) |
| `HEAP → FRAMES` is the one lock order; "the inversion is the one deadlock this pair can produce" | D-47 | The scheduler adds a lock; this file's lock-order section becomes the total order's authority |
| XSAVE "needs a scheduler; the kernel is built `+soft-float`" | D-29 | Still deferred: kernel threads are soft-float too, so there is no FPU state to switch until ring 3 (Phase 5) |
| Per-CPU stacks add layout assumptions — interest on the KASLR debt | D-46 | The stack arena is one more hardcoded window; the ADR that places it adds the entry to D-46's ledger |
| The address space cannot grow after boot; how it grows is "a decision Phase 5's address-space work should take deliberately" | D-45, D-49 | Phase 4 lives *within* the constraint — stacks are provisioned at table-build time — and does not pre-empt Phase 5's decision |
| SMP TLB shootdown, owed if anything ever unmaps | D-45 | Stays dormant: the arena maps at boot and unmaps nothing |
| Keyboard input drains to the log; nothing can block on a key | D-49 retired polling, not blocking | Slice 4: a wait queue, a blocking read, the interrupt wakes the reader |

Not this phase's debt, recorded so nobody mistakes it: the handle-table and
rights-taxonomy doc and the queue-layout doc are owed **before Phase 5**
(`002`, open questions), and KASLR waits for Phase 12 (D-46).

## Order of work (proposal)

The order is forced: nothing can schedule without a context switch, nothing
should preempt a switch that has not been proven cooperatively, tickless
needs a scheduler that knows the next event, and blocking needs
wake-from-interrupt, which needs the preemption-era lock discipline settled.

| Slice | Contents | Status |
|---|---|---|
| 1 | Kernel threads, cooperative: stack arena with guard gaps, TCB table, `spawn`/`yield_now`/`exit`, naked-function context switch in `wost`, round-robin ready list (deliberately policy-free) | ✅ 2026-08-22 |
| 2 | The scheduler: `libs/eevdf` arithmetic host-tested, EEVDF policy behind a separable seam with round-robin fallback, timer preemption at trap exit, the lock order and the context token | ✅ 2026-08-22 |
| 3 | Tickless: one-shot APIC programming, sleep queue and `sleep_ms`, the 100 Hz tick retired, uptime re-based | ✅ 2026-08-23 |
| 4 | Blocking: `WaitQueue`, the keyboard read blocks a thread and the interrupt wakes it, idle becomes the idle thread, D-34's latency position taken | |
| 5 | Phase close: VMware boot (D-18), roadmap and docs updated, this file's closing section | |

## Decisions the phase must make (with their tensions)

Held open deliberately; each becomes an ADR when taken.

- **The context-switch mechanism.** `#[unsafe(naked)]` + `naked_asm!` is
  stable on the pinned toolchain and `008` §1 calls this switch its natural
  adoption point — but the existing `global_asm!` interrupt trampoline is
  the most battle-tested assembly in the repo, and converting it is churn
  with no behavioural gain. Take the pattern for the new code; decide about
  the old on the record.
- **Stack provisioning under one-way mapping.** D-45 made `activate` consume
  the address space, so a stack cannot be mapped at spawn time. Boot-time
  arena, direct-map stacks without hardware guards, or a post-activation
  mapping API that weakens D-45? The guard question rides along: `008` §4.1
  warns a frame larger than the guard can jump it.
- **Preemption model.** Preemptive from the first commit couples the two
  hardest new mechanisms; cooperative-first proves the switch before it runs
  under interrupts, but means slice 1's checks exercise a gentler kernel
  than the one slice 2 ships.
- **The policy boundary.** D-15 wants pluggable policy as "a cheap future
  door, not a v1 feature"; sched_ext's lesson is *separable and observable,
  with a fallback path*. What is the smallest seam that honours both?
- **The allocation-safety context token.** `008` §4.2 says the ADR is due
  when the heap lands; it has. Adopt now — while the wake-from-interrupt
  path that makes violations writable is being built — or defer with
  reasons.
- **Per-CPU state.** One CPU today; SMP is a later phase. What shape does
  "current thread" take so that going SMP is a widening rather than a
  rewrite, and when does GS finally get loaded (D-29 left `swapgs` for
  ring 3)?
- **The tickless timebase.** Uptime currently interpolates a periodic
  countdown (D-36); one-shot mode deletes its denominator. TSC, accumulated
  deadlines, or something else — decided in slice 3 with the calibration
  machinery D-36 already built. *Taken in D-57: the timestamp counter, on the
  same window, with accumulated deadlines rejected because an idle machine's
  clock would stop.*

## The lock order

This section is the authority; module docs restate it, the ADRs cite it.

D-47 established `HEAP → FRAMES`: the heap takes the frame allocator's lock
during a slab refill, never the reverse. The kernel's thread table, `THREADS`, enters
as a **leaf**: while holding it, take nothing — not `HEAP`, not `FRAMES`, not
a console lock, not even `wost`'s own task-table lock, which `switch_to` and
`fabricate` take on their own and which is itself a leaf. Spawn does its
allocating before it touches `THREADS`; every switch happens after its guard
has dropped; a wake (slice 4) takes `THREADS` alone, which is what makes it
legal from trap context, where D-31's no-logging rule still binds.

## How it will be checked

The Phase 2/3 pattern carries over whole: arithmetic host-tested before
hardware sees it (D-30), every new mechanism landing with an on-machine
check in `kernel/src/selftest.rs` (D-28), each broken once to prove it can
go red, QEMU as the loop and a VMware boot to end the phase (D-18). Phase
3's standing lesson binds every check here: no identity comparisons, no
dependence on allocator or codegen ordering, no early return that makes the
failure path unreachable. **A check that has never failed is not evidence;
it is an untested claim.**

Expected machine checks, refined per slice: two threads alternate strictly,
counted rather than compared; callee-saved state survives a switch, verified
against recomputation; a thread stack's guard is absent from the tables
while the stack itself is writable and NX (structural read-back — the fault
itself is terminal); an exited thread's slot is reused, by count; a spinning
thread cannot starve another (preemption's whole claim); runtime divides in
proportion to weight, within bounds coarse enough to survive a hypervisor;
and, once something blocks, the D-38 keystroke harness watches a blocked
reader wake — not a poll, which is the shape D-49 caught lying.

## Where the code changes (breadcrumbs for the next session)

- `wost/src/arch/x86_64/task.rs` — new: the saved context, the naked
  switch, the entry trampoline. Raw `rsp` manipulation is TCB (D-13).
- `wost/src/arch/x86_64/paging.rs` — grows the stack-arena mapping,
  built before `activate` like everything else; the `KernelStack` handle
  follows `DeviceMapping`'s not-`Copy`-not-`Clone` precedent (D-49).
- `libs/stack-layout`, `libs/eevdf`, `libs/timebase` — new host-tested
  crates; each joins `members`, `default-members`,
  `[workspace.dependencies]`, and `HOST_PACKAGES` in `xtask/src/paths.rs`.
- `kernel/src/thread.rs`, `kernel/src/sched.rs` — new, safe code: the TCB
  table, spawn/yield/exit, the ready list, the policy seam. Scheduling
  policy is arithmetic, and arithmetic is the kernel's (D-43's pattern).
- `kernel/src/mem.rs` — `init_paging` maps the arena; the one place that
  knows how many threads the boot-time decision bought.
- `kernel/src/arch/x86_64/trap.rs` — the TIMER arm grows the preemption
  hook; EOI stays first.
- `kernel/src/main.rs` — the idle loop is the scheduler's to claim
  (slice 4), not before.
- `kernel/src/time.rs` — slice 3 re-based uptime on the timestamp counter
  and turned the timer into an alarm; `input::await_key`'s tick-counted
  timeout and every `time::ticks()` consumer were the pre-named casualties,
  and all of them went.

## Slice 1 — threads and the switch (2026-08-22)

Recorded as built, per the charter above.

**The boundary fell where D-13 said it would, and the kernel never holds a
context.** `wost::task` owns a fixed table of slots, each with a stack from
the arena and a saved stack pointer; the kernel names slots by index and asks
for transitions — `fabricate`, `switch_to`, `exit_to` — every one of which is
checked before it happens. That is D-51's substance: a `Context` value the
kernel could copy would make the sound-wrapper question unanswerable, and an
index into a table that refuses dead slots answers it. The switch itself is
fifteen instructions in a naked function (D-51); the interrupt trampoline
stays as it is.

**The stacks were forced into an arena by D-45**, not chosen (D-52). Sixteen
slots of 16 KiB above a 4 KiB gap that is never mapped, committed while the
tables are being built, with the arithmetic in `libs/stack-layout` and six
host tests for the one property that matters — no two slots touch. `task::init`
walks every stack in the live tables before it will fabricate on one. The boot
thread stays on the bootloader's stack this slice; slice 4 decides whether it
moves.

**Kernel-side policy is a ring.** `thread.rs` is a table of names and states
over the same indices and a fixed ring of who runs next — round-robin, no
allocation, and a leaf lock per this file's lock-order section. `spawn`
reserves a slot under the lock, drops it, then asks `wost` to fabricate;
`yield_now` and `exit` choose the next slot under the lock and switch after the
guard drops. Nothing outside the self-checks calls any of it yet, and the
module says so in the one `allow` it carries: the idle thread of slice 4 is
its first shipped caller, and the allow leaves with it.

**Four checks, each broken once, and one break that taught something.** Two
threads alternate strictly (yield made a no-op: red); callee-saved registers
survive a switch; every stack is guarded at both ends and present at both
(guard mapped: red); an exited slot is reused, by count (exit left the slot
taken: red). The register check's first break — deleting `r12`'s push and pop
— crashed the machine instead of failing the check, because it changed the
frame size and the fabricated frame's `ret` popped a zero. Saving `rax` in
`r12`'s slot is the break that tests what the check claims, and it went red.
A break that crashes proves less than one that fails: the harness cannot tell a
crash from a hang, and it says nothing about whether the check would have
noticed. Fifty-eight checks on the machine.

**Still owed by this slice's neighbours.** Nothing preempts: a thread that
never yields runs forever, which is slice 2's claim to test. Nothing enforces
that a thread holds no lock when it yields — it cannot corrupt memory, only
hang — and D-55's context token is where the type system takes that over.

## Slice 2 — the scheduler (2026-08-22)

**The policy is a crate, and the crate found the first bug.** `libs/eevdf`
is the whole of EEVDF as pure functions — wrapping virtual time, weighted
charging, the fair-share average, eligibility, the earliest-deadline pick,
and lag-clamped placement — with thirteen host tests (D-53). One of them
drives the policy the way the kernel will, a thousand steps at 1:2 weight,
and its first run reported a ratio of infinity: the heavy thread's clock
never moved, because one unit of real time at twice the base weight
truncated to zero virtual time. `RESOLUTION` exists because of that test.
The kernel's side of the seam feeds the policy the runnable entities in
rotation from just after the running thread, so equals take turns, and
counts the one answer the policy must never give.

**Preemption is the timer calling the scheduler after its end-of-interrupt,
and the gate is the flag the CPU pushed** (D-54). Code that was not accepting
interrupts is not switched out — and since D-34 clears the flag for every
lock, that single test makes every critical section unpreemptible with no
count to maintain. `switch_from_trap` in `wost` refuses a frame whose flag
is clear, so the rule is checked where the switch happens rather than
trusted from where it is called. Asterinas's `PreemptDisabled` discipline is
declined on the record: a lock already is one.

**Trap context became a number** (D-55). The heap refuses to allocate while
a handler is on the stack and the scheduler refuses to yield; a switch out of
a handler parks the count with the thread that owns it. The token type
`008` §4.2 proposed waits for the first API that can sleep, which is slice
4's, and D-56 names the three statics that become per-CPU when SMP arrives.

**What the counter caught on the first boot.** Sixty checks passed and the
sixty-first — "the policy never needed its fallback" — failed. Not a policy
fault: the yield path excludes the running thread from the candidates but
not from the average, so when the yielder is the one behind the average,
"nobody else is eligible" is the honest answer, and the thread should keep
the CPU rather than hand it to round-robin. The fallback now belongs only to
the preemption path, where everyone is considered and the policy's own tests
promise an answer. A counter that exists to make an inconsistency visible
made one visible in its first hour — in the caller, not the policy.

**Three more checks, each broken once.** A spinner cannot starve the boot
thread, which also never yields (preemption disabled: red). Runtime divides
1:2 by weight within bounds coarse enough for a hypervisor (weight ignored:
red). The fallback counter stays at zero (the preempt path denied its pick:
red). The first attempt at that last break — handing the policy a wrong
average everywhere — crashed in `exit` instead, the second time this phase
that a break reached the wrong code; a break has to hit the path the check
watches, or it proves nothing about the check. Sixty-one checks on the
machine, and every slice-1 check ran under preemption unchanged.

**Still owed by this slice's neighbours.** The tick costs a lock and a pick a
hundred times a second; slice 3 retires it. Time is charged in milliseconds,
conserved but coarsely attributed; the finer clock is slice 3's too. Nothing
can block, so lag's clamp has no sleeper to act on yet; slice 4's wait queue
is its first.

## Slice 3 — tickless (2026-08-23)

**The tick is gone, and what replaced it is two clocks doing two jobs**
(D-57). The processor's timestamp counter free-runs, so it is the clock:
`now_ns` is a subtraction and a multiply, with nothing to interpolate and no
handler to keep it moving. The APIC timer counts down and interrupts, so it
is the alarm: one-shot, armed for the running thread's slice ending or the
earliest sleeper's deadline, whichever is sooner, and stopped when there is
neither. Both rates come out of the same twenty-millisecond window D-36 built,
with two more reads inside it — the arithmetic that turns them into a clock is
`libs/timebase`, thirteen host tests, a rounded multiply-and-shift held to
thirty-two bits so a whole conversion packs into one word and can be published
to a reader inside an interrupt handler.

**One question the charter held open answered itself in the negative.**
Accumulated deadlines — summing each one-shot interval as it expires — needs
no new hardware and has a hole exactly where tickless puts one: while nothing
is armed, nothing accrues, and an idle machine's clock stops. Closing it means
arming a housekeeping timer, which is the tick wearing a different name.

**The clamp went, and its check started meaning something.** D-36's uptime was
clamped monotonic against a high-water mark, because a reader with the flag
clear could watch the countdown reload while the tick it announced stayed
pending. There is no countdown to reload now, so monotonicity is the counter's
claim rather than a clamp's — and "the clock never runs backwards", which
could not have failed before, went red on its first break.

**Sleeping is the first thing that waits** (D-58). A deadline, a saved lag,
and a state; every scheduling path wakes what is due before deciding anything,
and a woken thread comes back at the fair-share point offset by the lag it
left with, which is D-53's clamp finally acting on a real sleeper. A thread
that sleeps with nothing else runnable keeps the CPU and halts on it, on one
`sti`-`hlt` pair whose shadow is what stops the wakeup landing in the gap.
Slice 4's idle thread deletes that branch; inventing it here would have taken
slice 4's decision.

**The bug the sleep found had been there since slice 1** (D-59). A thread's
first run took its interrupt flag from the context that switched to it — and
that context is a scheduler, which clears the flag around every decision. So
every thread first started from a yield had run uninterruptible since slice 1,
and nothing noticed, because the checks that used that path yielded constantly
and the check that needed preemption got its thread started by the preemption
path, where the flag handed over is set. The symptom was a hang: a thread
slept, handed the CPU to a spinner that could not be interrupted, and the
machine stopped. The flag now comes from the thread's creator, captured where
there is still a real one to read.

**Nine checks broken, and two of the breaks taught the same lesson a third
time.** The one worth quoting is *an idle kernel takes no timer interrupts* —
not fewer, none, against the fifty the retired tick would have taken in the
same half second — with a counterpart under load that bounds the same counter
from below and is also what now proves the handler's end-of-interrupt. Making
the measured counter rate a thousand times too fast turned the machine into an
interrupt storm before the check that reads the rate could run; breaking the
expiry handler could not redden a check about an idle kernel, which never
enters it. A break has to reach the path the check watches.

**One flake fixed on the way past.** The editing-keys check measured the
console cursor while the harness's second and third injected keystrokes were
still echoing into it — a red check with nothing wrong, about one boot in
three. It now waits for the last injected key to be seen released, which is
also a check that the key arrived. Sixty-four checks on the machine, and
sixteen more on the host.

**Still owed by this slice's neighbours.** Nothing can be woken by an *event*
— only by a deadline — so `WaitQueue` and the blocking keyboard read are
slice 4's, along with the idle thread and D-34's latency position.
