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
| 100 Hz periodic tick, "explicitly disposable when D-15 lands" | D-15, D-36; `006-cpu-fundamentals.md` ("D-34's latency cost wearing a clock face") | Slice 3 retires it: one-shot APIC programming, no timer armed when nothing needs waking |
| A critical section longer than a tick swallows one; uptime undercounts 10 ms | D-36 consequences | Dies with the periodic tick; uptime moves off tick-interpolation (slice 3, with its own ADR) |
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
| 1 | Kernel threads, cooperative: stack arena with guard gaps, TCB table, `spawn`/`yield_now`/`exit`, naked-function context switch in `wost`, round-robin ready list (deliberately policy-free) | |
| 2 | The scheduler: `libs/eevdf` arithmetic host-tested, EEVDF policy behind a separable seam with round-robin fallback, timer preemption at trap exit, the lock order and the context token | |
| 3 | Tickless: one-shot APIC programming, sleep queue and `sleep_ms`, the 100 Hz tick retired, uptime re-based | |
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
  machinery D-36 already built.

## The lock order

This section is the authority; module docs restate it, the ADRs cite it.

D-47 established `HEAP → FRAMES`: the heap takes the frame allocator's lock
during a slab refill, never the reverse. The scheduler adds `SCHED`, and it
enters as a **leaf**: while holding `SCHED`, take nothing — not `HEAP`, not
`FRAMES`, not a console lock. Spawn does its allocating before it touches
`SCHED`; wake takes `SCHED` alone, which is what makes it legal from trap
context, where D-31's no-logging rule still binds.

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
- `libs/stack-layout`, `libs/eevdf` — new host-tested crates; each joins
  `members`, `default-members`, `[workspace.dependencies]`, and
  `HOST_PACKAGES` in `xtask/src/paths.rs`.
- `kernel/src/thread.rs`, `kernel/src/sched.rs` — new, safe code: the TCB
  table, spawn/yield/exit, the ready list, the policy seam. Scheduling
  policy is arithmetic, and arithmetic is the kernel's (D-43's pattern).
- `kernel/src/mem.rs` — `init_paging` maps the arena; the one place that
  knows how many threads the boot-time decision bought.
- `kernel/src/arch/x86_64/trap.rs` — the TIMER arm grows the preemption
  hook; EOI stays first.
- `kernel/src/main.rs` — the idle loop is the scheduler's to claim
  (slice 4), not before.
- `kernel/src/time.rs` — slice 3 re-bases uptime; `input::await_key`'s
  tick-counted timeout and every `time::ticks()` consumer are the
  pre-named casualties.
