# 011 — Userspace (Phase 5)

The phase that gives the kernel someone to serve. Ring 3, a handle table that
is a process's external-object authority, `spawn` in place of `fork`, ELF
images loaded from an initrd, channels that carry authority, and the
submission/completion rings the native ABI is made of — closing with a stub
mlibc sysdep whose only job is to find out whether a POSIX libc can sit on this
ABI while the ABI is still cheap to change (D-74).

Phase 4 left a kernel that can run more than one thing but has nobody to run
it for: every thread is the kernel's own, every pointer is trusted, and the
only authority anything holds is the authority to be compiled in. Two
interfaces get fixed here that this project will live with longer than any
other — the handle table and the queue ABI — and both are the kind that get
frozen by their first caller. That is why this file exists before the code
does, and why the ring protocol gets a model before it gets an implementation.

This file is the phase's charter, written before its code per D-19. As with
`007-memory.md` and `009-multitasking.md`, each slice's design is recorded
below **as it is built**, so the file ends the phase as the description of what
exists; the slice table is the dependency-forced proposal, refined when the
phase starts.

References: `010-research-notes-3.md` throughout — §2 (the async ABI, its four
hard problems, and the rationale that had to be rewritten), §3 (capabilities
and the handle table, which answers `002`'s open question), §4 (the SMP
discipline that must not be deferred past this phase), §5 (the POSIX bill), §8
(where the time goes, and the extended-state decision ring 3 forces), §9 (the
verification tier, and the two version pins that reach it), §10 (FRED, CET, and
the security features that must be shaped now), §12 (the ranked list of what
must be decided before this phase writes code). Also Zircon's handle and port
documentation, Fuchsia RFC-0028 and RFC-0261, io_uring's man pages and its CVE
record, Windows RIO's reservation model, Clements et al., *The Scalable
Commutativity Rule* (SOSP'13), Intel SDM Vol. 3A ch. 6 (trap delivery) and
Vol. 1 ch. 13 (XSAVE), and mlibc's porting book.

Check totals below are dated slice-close snapshots. The phase-close total is
the current total at the end of Phase 5, not a promise that later phases retain
the same number.

## What earlier phases left owing

Every debt below is on the record elsewhere; this table is where a future
session finds them collected.

| Debt | Where recorded | What Phase 5 does |
|---|---|---|
| The boot thread's stack and the double-fault IST stack are unguarded | `002` §Security baseline; `007` close | Slice 3's stack story guards both, alongside the second region CET will want |
| The address space cannot grow after boot; how it grows is "a decision Phase 5's address-space work should take deliberately" | D-45, D-49; `009` | Slice 3: `activate` stops consuming the space, and `map` becomes reachable on a live one |
| Memory objects, moved out of Phase 3 when D-44 answered the untyped question | D-44; `007` slice 6 note | Slice 7, on slice 3's owned frames |
| `AddressSpace::map` accepts any physical address without ownership evidence | D-76, D-77 | Slice 3: a mapping takes an owned frame, which is D-44's discipline applied to the mapping API |
| The untyped split `UFrame`, "absent on purpose" until something can map a frame into a second address space | D-44; `wost/src/mm/mod.rs` | Slice 3 gives it its first real caller |
| Port claims prove exclusivity but not device entitlement | D-77 | **Not this phase.** The user-memory half is slice 3's; the device half is Phase 6's, with PCI discovery |
| XSAVE, deferred until "there is a scheduler" | D-29 | Slice 5: eager, compacted, boot-sized, with the offset arithmetic in `libs/` |
| `swapgs` and the GS base, left for ring 3 | D-29 | Slice 5, which is where `arch::percpu` gets its first real caller |
| There are no ring-3 descriptors | `006` | Slice 5 |
| The lock discipline belongs in the borrow checker — 13 statics, 73 call sites | D-69 | ✅ Slice 2, first, so nothing in this phase was written twice — and D-85 records where its shape was wrong |
| There is no preempt-disable mechanism; `SpinLock`'s `cli` is doing that job | D-70 rule 2 | A preempt-only lock, with its first caller: every lock a handler can reach must keep disabling interrupts, and today that is all of them |
| The per-CPU word widens; `SPINLOCKS_HELD` is still a global | D-56, D-70 | Slice 5: `arch::percpu`, reached through `gs:`, alongside `swapgs` |
| Per-process quota accounting for kernel-memory DoS — "Phase 5's handle-table doc inherits the quota obligation" | D-44 | Slice 6, at the handle layer, which is where the ADR said it belongs |
| A wait with a timeout, and a wait on two queues | `009` slice 4 | Slice 9: both fall out of the completion queue, which is why neither was built early |
| The thread cap of sixteen is a real cap | D-52; `kernel/src/thread.rs` | Slice 3, when stacks stop being committed at table-build time |
| Every new hardcoded window is interest on the KASLR debt | D-46 | Slices 3 and 7 each add an entry to the ledger |
| `002` states that channels are the sole handle-delegation mechanism, which is false even of Zircon | `010` §3 | Slice 7 replaces it with the checkable invariant and a self-test that holds us to it |

Not this phase's debt, recorded so nobody mistakes it: KASLR waits for Phase 12
(D-46), SMP bring-up waits for Phase 12 (D-70), the device-entitlement half of
D-77 waits for Phase 6's PCI discovery, and compat-fork mechanics wait for
Phase 9 (D-07, D-74).

## Order of work (proposal)

The order is forced. The borrow-checked lock discipline goes first or every
line this phase writes gets written twice. Nothing can build a second address
space while `activate` is one-way. The entry path must be decided before
anything enters it from ring 3. Handles need objects to name, spawn needs
handles, channels need spawn, and the queue needs channels to have proved the
message shape. mlibc needs an ABI to sit on.

| Slice | Contents | Status |
|---|---|---|
| 1 | This charter: the handle-table layout and lifetime rules, the queue layout and wakeup semantics, the signal answer, and the ADRs that cost no code | ✅ 2026-08-24 |
| 2 | The lock discipline in the borrow checker (D-69): `SpinLock::lock` borrows the context, and the leaf acquisition a `Drop` can take | ✅ 2026-08-24 |
| 3 | The address space that grows: two-way mapping, mappings that carry their evidence, unmap returning a flush token, the double-fault stack guarded | ✅ 2026-08-24 |
| 4 | Trap entry, FRED-shaped: `libs/trap-cause` and table dispatch, the boot thread's own guarded stack, eIBRS and the speculation report, x2APIC as a checked precondition | ✅ 2026-08-24 |
| 5 | Ring 3: user descriptors, one gate at DPL 3, the SMAP copy guard, `VERW` on the way out, the XSAVE layout in `libs/x86-cpuid` | ✅ 2026-08-24 |
| 6 | The handle table: `libs/handle-space`, rights in one `u32`, a refcounted table, the object header, per-process quota | ✅ 2026-08-24 |
| 7 | Memory objects, spawn, ELF loading, initrd: `libs/elf64`, a second address space, `UFrame`, per-thread `RSP0` and eager XSAVE, stacks that arrive with their thread, two processes, exactly one handle each at spawn, and `THREADS` split into a thread table and a run queue | ✅ 2026-08-24 |
| 8 | Channels: bytes and handles, attenuate-on-send, rights declared per field with no defaults | ✅ 2026-08-24 |
| 9 | The async queue ABI: the model before the protocol, `libs/ring-proto` under shuttle and proptest, reservation backpressure, exactly one terminal completion | ✅ 2026-08-24 |
| 10 | The stub mlibc sysdep (D-74): written and analysed, not compiled — the toolchain wall is recorded in D-114 | ✅ 2026-08-24 |
| 11 | Phase close: the VMware boot (D-18), the adversarial audit (D-77), the roadmap and this file's closing section | ✅ 2026-08-24 |

## The handle table

`002` listed "handle table representation and rights taxonomy" as owed before
this phase. `010` §3 answered the taxonomy half; this section owes the layout
and the lifetime rules, and takes them.

**One global arena, not a per-process array.** Zircon has no per-process handle
array: one arena, `base_value = generation ‖ arena_index`, and the handle value
a process sees is `(base_value << 2 | 0b11) ^ per_table_mixer`. Lookup is
arithmetic plus one ownership comparison. Two independent research angles
converged on copying this without sight of each other — one for security
(generation tagging kills use-after-free on handle reuse), one for scalability.

**The scalability half is the reason the low bits are reserved.** The Scalable
Commutativity Rule says that whenever interface operations commute, they can be
implemented so as to scale, and it is analysable before an implementation
exists — which is exactly the property this phase needs, because the hardware
that would show the difference is in Phase 12. Zircon's contract commutes *by
construction*, because reserved low bits forbid a dense encoding. POSIX's
"lowest free descriptor" does not: it is an ordering promise that serialises
allocation. sv6 measured the unordered alternative scaling linearly **with no
single-core penalty** — "at one core, both cases perform identically." So:
**the handle table does not promise the lowest free handle** (D-70 rule 7), and
the encoding makes the promise unmakeable rather than merely unmade.

**The encoding arithmetic goes to `libs/handle-space`**, beside `port_space`
and `frame_space` — D-30 and D-43's pattern applied a third time. The kernel
keeps the arena and the lock. Per D-28, the two checks most likely to be
silently unfailable here are *the invalid value never decodes* and *a reused
slot never yields the same value twice*; both get broken.

**Shape: `HandleTable { table: AtomicPtr<HandleArray> }`** — Linux's
`files_struct`/`fdtable` split, which Linux took nine years to reach. Readers
can go lock-free later without touching a call site.

**The table is a refcounted object, not an inline field of `Process`.**
Zircon's strict per-process invariant was the first casualty of its own
compatibility layer: RFC-0261 shares handle tables for Starnix. Free now, a
redesign later.

**Rights: 16–24 in one `u32`**, universal (`DUPLICATE`, `TRANSFER`, `WAIT`,
`INSPECT` — Zircon's `RIGHTS_BASIC`) split from per-type. The evidence for
"small" is one-directional: seL4 has 4, Zircon 25, CloudABI 31, Capsicum ~81 —
and Capsicum's taxonomy outgrew a machine word and broke its ABI, so
`cap_rights_t` is now a multi-word struct spending 7 of every 64 bits on
metadata, with an `init`/`set`/`is_set` API because two rights can no longer be
OR'd. WASI deleted its rights system entirely in Preview 2 for creating "a lot
of ambiguity about how it mapped to POSIX semantics." managarm, adding
fine-grained rights ten years in, landed on ~14. The rule that keeps the number
where it is:

> A right must correspond to a distinction the kernel actually makes about that
> object — never to a syscall name.

That is precisely the rule WASI's rights violated.

**Type encodings get headroom for 64 object types, not 8.** Zircon is at 26 and
still growing. Policy is **monotonic and frozen** — tighten-only, immutable
once a process has children, Zircon's `zx_job_set_policy` shape — with bits
encoded so that adding an object type never renumbers an existing one.

**No revocation.** Zircon has none; Capsicum has none; seL4 has real recursive
revocation and pays for it — operations are preemptible and resumable, progress
stored in a zombie capability, with a pre-mitigation worst-case execution time
of about one second, reduced to 436 µs computed and 81 µs observed. Object
death plus a strippable `TRANSFER` right covers the real cases. If revocation
is ever needed, a per-object **epoch bump is O(1)** — no traversal, no
preemption points, no zombies — and the epoch already exists as the generation
counter in the handle encoding.

**Lifetime.** A handle is a reference; the object dies when the last one goes.
Refcounts use `inc_not_zero` from day one (D-70's list) so that a lookup racing
a close cannot resurrect a corpse. Closing does **not** promise global
visibility on return (D-70 rule 9) — the same decision as Linux's `mmu_gather`
batching, and it is unmakeable later.

**The quota obligation D-44 assigned here** is answered at this layer, not the
frame layer: a process's kernel memory is bounded by what its handle table can
name, and the bound is a policy number that stays revisable. Making callers
donate memory would have been an ABI decision and permanent; this is not.

## The queue

`002` listed "queue memory layout and wakeup semantics" as owed before this
phase. This section owes it, and `010` §2 supplies the answers.

**The rationale first, because the recorded one was refuted by its own
source.** D-06 justified async-first with FlexSC, reading it as *batching and
cache locality, not trap avoidance*. FlexSC says the opposite: batching
"minimizes the frequency of switching between user and kernel execution,
eliminating some of the mode switch overhead … improves **both** the direct and
the indirect costs." The 2026 attribution ladder is cleaner and is what this
phase should hold in mind:

> Structural asynchrony ≈ 10×. Total trap elimination ≈ 1.3×, and it costs a
> dedicated core.

With the measurement that matters most for API design: **naive io_uring
adoption bought 1.06×; the 33× came from the caller becoming asynchronous.**
The design consequence is sharper than "use rings" — *the win is in making
async the natural way to write a service.* An ABI that lets callers stay
synchronous will deliver the 1.06×.

**The four hard problems, answered.**

- **Cancellation is advisory.** io_uring's own manual: `-EALREADY` means "the
  caller must wait for the canceled request to post its completion event." The
  rule this becomes: **every operation posts exactly one terminal completion,
  always.**
- **The kernel owns the buffer.** Blocking in `Drop` is unsound because leaking
  is safe, so a userspace-owned buffer has no sound lifetime. Windows RIO
  states the rule kernel-side — page-locked at registration, deregistration
  forbidden while requests are outstanding.
- **Backpressure is reservation at submission.** io_uring's `CQ_NODROP` took an
  unbounded kernel-memory liability for a misbehaving application; RIO's
  reservation-based admission control makes overflow *structurally impossible*.
  Steal RIO's. The CQ is a power of two and **at least twice the SQ size**.
- **Copy each entry out exactly once, then validate the copy.** The double
  fetch is the canonical hole and it is not theoretical: CVE-2025-39816 is
  literally "always use `READ_ONCE()` to read ring pointers." This is enforced
  by the shape of the API, not by discipline — the validated copy is the only
  thing the kernel can reach.

**Ports and completion queues are the same shape; build one.** A Zircon port
*is* a completion queue whose entries are events. Building both gives userspace
two event planes to join, which is exactly why io_uring grew `poll_add`,
`futex`, `waitid` and `msg_ring`. Zircon's four semantic wins move into the CQ
instead: one-shot arming by default, edge-versus-level as an explicit flag,
cancellation that purges already-queued entries, and exactly one waiter
released per entry.

**Austerity, from io_uring's security record.** ~60% of Google kernelCTF
submissions targeted io_uring and Google still disables it in 2026; NVD returns
143 io_uring CVEs, rising — 38 in 2024, 40 in 2025, 36 by partial 2026 — and by
class 50 race/TOCTOU, 23 use-after-free, 10 refcount. But the statistic is
frozen *because* they disabled it, and the 2025–26 cluster is in **newly added**
surface: `zcrx`, provided buffers, `ublk`, passthrough. The lesson is feature
velocity on a ring surface, not io_uring the name. Ring structures carry
explicit version fields (IoRing's improvement); the opcode surface stays
minimal; there are no privileged opcodes.

**No global FIFO across submitters** (D-70 rule 8). sv6's mail-server benchmark
is the evidence: an ordered socket forces one shared queue, an unordered one
allows per-core queues. Rings are single-owner and the pages are
kernel-allocated.

**One consequence nobody had written down for us.** If the native ABI is a
ring, then *"observe the syscalls" is not a valid basis for auditing,
sandboxing or introspection*. ARMO's 2025 "Curing" rootkit did command and
control without making a single system call, and Falco, Tetragon and Microsoft
Defender were all blind to it. Linux discovered this after the fact. Every
observability and security mechanism this kernel will ever want has to be
designed into the ring from the start, which means slice 9 owes at least the
hook, even if nothing consumes it until much later.

## How a signal interrupts a blocking queue wait

The question D-74 called the hardest to answer late, and `002` did not mention
signals at all.

**A signal must share the completion plane.** The target blocking wait is
interruptible through that same mechanism rather than a second event plane,
which is the argument that unified ports and completions above. There is no
in-kernel signal frame, kernel signal mask or restart-after-interrupt semantic.
D-181 later made the boundary explicit: version 1 has no unsolicited-event
record, so no signal wire format or interruptible libc dispatch exists yet.

What this costs, recorded now so Phase 9 is not surprised: mlibc's `Sigaction`
needs a userspace dispatch loop over CQ entries. POSIX behaviours that assume
a kernel-delivered signal interrupting an arbitrary instruction — `EINTR` on
a blocking call, signal-safe reentrancy, resumable `SIGSEGV` handlers — need
explicit libc/runtime support or remain unsupported. Plan 9's APE and Fuchsia
POSIX Lite show the cost; Phase 9 decides which behavior is worth emulating.

## The lock order

This section is the authority; module docs restate it, the ADRs cite it.
`009-multitasking.md`'s section is superseded by this one from slice 2 onward.

Phase 4 left `HEAP → FRAMES` (D-47) with the thread table and each wait queue
as leaves, and a rule between lock *kinds*: a `ThreadLock` may never be taken
while a `SpinLock` is held (D-63), checked by a count.

Slice 2 changed what enforces most of it, and D-85 records how the shape
differed from what D-69 predicted. `SpinLock::lock` takes
`&'a mut ThreadContext` and the guard borrows it, so **while one token is in
scope**, taking a second lock with it or blocking with the guard live are both
`E0499`. `lock_leaf` is the second level, for an acquisition that is the whole
operation — the only one a `Drop` impl can perform, since `drop` takes no
arguments. `libs/lock-rank` was not written: a rank with one non-trivial pair
is not an abstraction.

**The compile-time half is a strong default, not a proof, and the difference
matters.** `ThreadContext::claim` mints a token wherever the interrupt flag
says thread context, so a second one can be minted inside a live guard and the
borrow says nothing about it. That is one visible, greppable line — the same
shape as an `unsafe` block, and audited the same way — but it is not the
`E0499` this section used to claim. What still holds unconditionally is the
runtime side: D-66's assert refuses a context switch with a guard live, and
`ThreadLock` refuses to be taken while a spin lock is held. Two independent
checks is what OSTD ships, and here the second is the one that cannot be
side-stepped.

**A closure taken under a leaf guard is the live hole**, and there are two:
`mm::with_memory_map` and `dev::with_io_apic` run caller-supplied code with a
lock held. (`mem::with_active_space` was listed here and does not belong: it
takes the token — `SPACE.lock(context)` — and is the sanctioned two-level
nesting D-85 describes, not a leaf acquisition.) The obligation is on the closure and
nothing checks it — a closure that logs takes the console's `ThreadLock` and
trips D-63's assert, which is how this was found rather than reasoned. Closing
it properly means a guard type that forbids calls, and that is a decision this
phase has not taken.

The preempt-only lock is a third kind and sits below both: it disables
preemption and leaves interrupts on, so it may be taken while nothing else is
held and may not be held across a block. Never write "disable interrupts" where
"disable preemption" is meant — the single most-cited breakage across the SMP
survey, and the conflation this kernel currently has.

**`ARENA` is a leaf**, added by slice 6. Every acquisition is
take-mutate-return, and the two places that were not — growing the slot vector,
and collecting objects in `HandleTable::drop` — both called the allocator under
the guard, which rebuilt the `ARENA → HEAP → FRAMES` order D-85 had just
deleted. Growth is reserved outside the lock now and the drop buffer is sized
before it is taken (D-95). An object's last reference is always released *after*
the guard is gone, because dropping one can reach the frame allocator.

Slices 7 through 9 add the remaining objects and each ring. Their order relative
to the above is fixed when each lands, and the rule for all of them is the one
D-70 rule 5 already binds: **interrupt handlers never block**.

## How it will be checked

The pattern carries over whole: arithmetic host-tested before hardware sees it
(D-30), every new mechanism landing with an on-machine check in
`kernel/src/selftest.rs` (D-28), each broken once to prove it can go red, QEMU
as the loop and a VMware boot to end the phase (D-18). Phase 3's standing
lesson binds every check here — no identity comparisons, no dependence on
allocator or codegen ordering, no early return that makes the failure path
unreachable — and Phase 4 added two more: a break has to hit the path the check
watches, and a break that crashes proves less than one that fails.

**This phase adds a third check location and a fourth, and both are new kinds
of claim.**

The third is the **model**. The ring protocol is specified in TLA+ and checked
before it is written, because the Scalable Commutativity Rule's decisive
property is that it is analysable "before an implementation exists and before
the necessary hardware is available", and because the one bug class a single
CPU cannot show — Store→Load reordering — is exactly the shape of the
lost-wakeup re-check the ring depends on. Java 21 is already on the development
machine, so `tla2tools.jar` costs a vendored download and a `cargo xtask
model-check` task. The model gets its own break: a deliberately weakened
invariant must produce a counterexample, or the model is decoration.

The fourth is **host concurrency testing** of `libs/ring-proto` under shuttle
and proptest. `010` §9 measured the wall and the way through: shuttle 0.9 works
here unpinned, proptest works with `default-features = false` and
`getrandom = "=0.3.1"`, and anything reaching `getrandom >= 0.3.2` or
`windows-sys >= 0.61` fails because there is no assembler on this machine. The
pins are a recorded tax under D-16, and slice 10's vendored clang is what
eventually retires them. This tier also gets its break: a variant built with
`Relaxed` where the protocol needs `Acquire`/`Release` must **fail** under
shuttle. A concurrency harness that passes a knowingly wrong program is not a
harness.

Expected machine checks, refined per slice: a preempt-only lock is held across
a delivered interrupt; a mapping into a live address space is readable and
survives unmap only until its flush token is spent; a write into the boot
stack's guard faults and so does one into the double-fault stack's; a ring-3
thread runs, syscalls, and returns with ring 3 in the trap record; a ring-3
read of a kernel address faults; extended state survives a switch between
threads using it differently; a reused handle slot never yields the same value
twice; **a freshly spawned process's handle table holds exactly one entry**;
two processes cannot see each other's memory; a handle sent through a channel
arrives with exactly the declared rights and is gone from the sender; and every
ring operation posts exactly one terminal completion, cancelled ones included.

That spawn check is the one worth naming twice. `002` pillar 1 had claimed
channels were the sole delegation mechanism, which is false even of Zircon.
Slice 7 replaced it with the load-bearing part that remains checkable:

> A process receives exactly one handle at spawn — its bootstrap channel.

Slice 7 fixed `002` and landed the check in the same commit. D-151 later made
handle-producing completions an authorized second provenance path without
changing what a process holds at birth.

## Known holes, carried deliberately

Each was found by an adversarial pass (D-94) and each is a design decision
rather than a repair, so each is written down instead of patched over.

- **A closure taken under a leaf guard.** `mm::with_memory_map` and
  `dev::with_io_apic` run caller-supplied code with a lock held, which is
  exactly what `lock_leaf`'s obligation forbids. (`mem::with_active_space` is
  not one of them; it takes the token.) The
  obligation is on the closure and nothing checks it: a closure that logs takes
  the console's `ThreadLock` under a `SpinLock` and trips D-63's assert — which
  is how this was found, not by reasoning. Closing it means a guard type that
  forbids calls, and that shape wants a phase that has more callers to test it
  against.
- **`mem::forget` on a `SpinLockGuard` strands the interrupt flag.** Leaking is
  safe, so a safe caller can leave interrupts off for the life of the kernel and
  `SPINLOCKS_HELD` permanently non-zero. Not undefined behaviour, but
  `Unmapped`'s own doc states the principle this violates — *no soundness
  argument may rest on `Drop` running* — and the guard's interrupt invariant
  rests on exactly that.
- **`MappedStack` can be invalidated while it is still evidence.** `unmap` will
  clear a range `map_stack` mapped, since it carries `OWNED`, leaving a live
  token that claims a stack is there. The result is a fault rather than
  corruption, which is why it is here rather than in D-94's fixes.

## Decisions the phase must make (with their tensions)

Held open deliberately; each becomes an ADR when taken. The ones this charter
takes up front are those where writing code first would waste the most work.

- **Where `ThreadContext` lives.** D-69 needs `wost::sync` to name the token,
  and the token is `kernel/src/thread.rs`'s today. Moving it to `wost` is the
  smaller change and matches D-62's framing — but `wost` then exports a type
  whose whole meaning is a kernel scheduling fact, which is the sort of leak
  D-13's boundary exists to prevent. Trait or move, decided on the record.
- **What replaces one-way mapping as the safety argument.** D-45 made `map`
  safe by making it unreachable once the tables were live. A live-mappable
  space needs a different answer, and "the caller owns the frame" is the
  candidate — but ownership of a frame is not ownership of a *virtual range*,
  and the second half needs its own mechanism or `map` can still clobber.
- **Whether the arena is in `wost` or `kernel`.** The handle table holds no raw
  pointer and touches no hardware, so by `005`'s own test it belongs in
  `kernel` — but it is also the thing every capability check rests on, and
  `wost` is where the sound-wrapper standard is enforced. The rule says
  `kernel`; say so explicitly rather than by omission.
- **How large the ring pages are, and who counts them.** Kernel-allocated ring
  pages mean a process's rings are kernel memory, which is exactly what the
  quota is for — so the quota's unit has to be decided with the ring's, not
  after it.
- **Whether `syscall`/`sysret` or a trap gate carries the synchronous set.**
  `sysret`'s non-canonical `RCX` hazard is a real CVE class and the mitigation
  is a branch on every return; a trap gate has none of that and costs more per
  call. The measurement that would settle it is meaningless under TCG (`010`
  §8), so this one gets decided on safety and revisited on real hardware.
- **How much of the entry path is rebuilt in slice 4 versus grown into.** 256
  vector-aware stubs work today and the FRED-shaped table is strictly better —
  but the existing trampoline is the most battle-tested assembly in the repo,
  and Phase 4 already declined to convert it once for exactly that reason.
- **What the mlibc slice does if it cannot compile.** The vendored clang is the
  plan; the fallback is authored sources and a recorded wall. Decide the
  threshold before starting, not while overrunning.

## Slice 2 — the lock discipline in the borrow checker (2026-08-24)

Recorded as built, per the charter above.

**D-69's counts were exact and its shape was not** (D-85). Thirteen statics,
seventy-three call sites, as predicted. But a `Drop` impl cannot receive a
token — `Frame` and `Segment` free themselves back to the physical allocator
from wherever they die, and `drop` takes no arguments — so `FRAMES` could never
be a token-bearing lock whatever the discipline said. The same is true of
`GlobalAlloc::alloc`, whose signature is the language's.

So a lock has **two levels**, not a rank lattice. `lock(&mut ThreadContext)`
borrows the token for the guard's life. `lock_leaf()` is for an acquisition
that is the whole operation — take, mutate, return, calling nothing — and it is
the one a `Drop` can take. A leaf may be taken while a token-bearing lock is
held, which is what makes a nested acquisition sound rather than an exception.

**Sixty-eight of the seventy-three sites were already leaves.** Phase 4's
scoping, forced by D-66's assert refusing a switch with a guard live, had
already produced the property D-69 wanted to enforce — everywhere. The token
earns its keep on the five blocking paths, and `libs/lock-rank` went unwritten
because a rank with one non-trivial pair is not an abstraction. So did
`lock_from_trap`: every acquisition a handler makes is a leaf, so it would have
been a second name for `lock_leaf` with no caller.

**The heap stopped nesting, which deleted a lock order instead of proving it
safe.** `HEAP` was the one lock held across a call: `take_slot` refilled from
inside the guard, and refilling takes `FRAMES`. It now returns null on an empty
free list and the caller allocates the frame with the lock *dropped*,
re-acquires and carves. `HEAP → FRAMES` is no longer an order this kernel has,
and `wost` holds no lock across a call at all — every non-leaf lock in this
kernel is the kernel's own.

**The break was the counter-intuitive one, and it worked.** `let _a =
a.lock(&mut cx); let _b = b.lock(&mut cx)` and `let _g = a.lock(&mut cx);
sleep_ms(&mut cx, 1)` are both `E0499`, and the legal ordering compiles. Then,
per D-69's warning: **delete `Drop for SpinLockGuard` and both errors vanish**,
because without a `Drop` impl NLL ends the borrow at the call. The guarantee is
`Drop`-dependent. Breaking the lock instead would have proved nothing about it.

**Where a check goes was decided by which break tells the truth.** The
restructure earned a machine check — a refill costs a frame and the accounting
balances across the window the dropped guard opens. Its other half, that no
slot is handed out twice, went to `libs/slab-layout` as arithmetic, because
breaking it on the machine corrupts the allocator every later check runs on and
produces a general-protection fault rather than a red verdict. Phase 4 learned
twice that a break which crashes proves less than one that fails; this is the
first time that lesson chose a check's *address*. The machine check found a
flaw in itself on its first boot, too: the vector holding the allocations was
built inside the snapshot window, so its own buffer counted. Eighty checks.

**Four things this slice was scoped to carry now belong with their first
callers**, which is `005`'s no-speculative-API rule applied to a slice plan
rather than to an API:

- `arch::percpu` behind `gs:` → **slice 5**, where `swapgs` is its first real
  caller. `010` §4 said "provide the accessor when Phase 5 sets up `swapgs`",
  and that is slice 5, not this one.
- The preempt-only lock → its first caller. Every kernel lock reachable from a
  handler must keep disabling interrupts, and today that is all of them; the
  candidates arrive with the handle table and channels.
- The full barrier on the lost-wakeup re-check (D-70 rule 6) → **slice 9**. The
  wait queue is subsumed by the completion queue there, the fix is a
  restructure rather than a fence, and the TLA+ model is the only thing that
  can check it — a single CPU cannot, by the same argument that put the rule in
  D-70.
- `THREADS` split into a thread table and a run queue → **slice 7**, where
  processes give the table a second reason to change and the split happens
  once rather than twice.

**Still owed by this slice's neighbours.** The compile-time property has no
automated check and cannot have one where it lives: `wost` is cross-compiled,
so a `compile_fail` doctest cannot run against it (D-41), and a copy in `libs/`
would drift. It is verified by hand, recorded in D-85, and re-verifying it is a
manual step when the guard types change.

## Slice 3 — the address space that grows (2026-08-24)

Recorded as built, per the charter above.

**The constraint was already redundant, and D-77 is why we could tell** (D-86).
D-45 kept `AddressSpace::map` sound by making it unreachable: `activate` took
the space by value, so once the tables were live nothing held anything to map
into. Three phases were shaped by that. But D-77's adversarial pass had already
hardened `set` so that a present entry accepts only an identical rewrite — and
*that* is what stops a live mapping moving under the running kernel. Taking the
space by value was guarding a hazard the entry check had closed. So `activate`
borrows, the kernel keeps its space, and mapping into a running space is safe
for a reason that is checked rather than promised.

Two shapes were available and both are known failures. An
`ActiveSpace`/`InactiveSpace` typestate is the obvious Rust answer, and `010`
§4 names it: *Theseus's `ActivePageTable`/`InactivePageTable` typestate was
simply wrong once two cores existed.* A `wost`-held global behind a closure
would have put a lock where the kernel's policy belongs, against D-43.

**A mapping carries its evidence** (D-87), which is D-76's first authority debt
paid and the reason D-77 left it: *"Phase 5 must replace the first with owned
user-memory mappings."* There is no public entry taking a bare physical address
any more. `map_owned` takes frames the caller owns and moves them in;
`map_boot` takes a `BootRegion`, which exists only for a range the bootloader's
map covers, page-aligned, non-empty and non-overflowing; `map_device` was
already the narrowest.

The two cases genuinely differ and neither collapses into the other. The direct
map and the kernel image are nobody's frames — firmware placed them — so
demanding ownership there would be a fiction. What can honestly be demanded is
that the handoff described them, which is the bound D-64 gave physical reads.

**Unmap gives frames back, and the token owns them until it has flushed**
(D-88). A CPU may still hold a cached translation to a cleared entry, so the
allocator must not see the frame first; the frames therefore live *inside* the
token and leave only through `flush` or the `Drop` that does the same work.
That also survives Rust's leak rule, which is the part that decides the design:
`mem::forget` on a token costs the frames and not correctness, so no soundness
argument rests on `Drop` running. It is the same reasoning D-79 applies to ring
buffers, in the other direction.

Provenance is bit 9 of the page entry, one of the three the architecture leaves
to software. A frame the allocator handed out must come back and one the
firmware placed must not, and the entry is the only thing every path to an
unmap can see. Large pages are not split: the direct map is two-megabyte on
purpose (D-45), and `010` §8 measured that as right.

**Half of `002`'s stack debt is paid** (D-89). The double-fault handler stood
on a `.bss` array with nothing below it — the worst of the three to leave bare,
because an overrun there corrupts its neighbour *while the fault handler is
running*. It now stands on a guarded region whose geometry is the thread
arena's own, placed at its end, so there is no new hardcoded window and no
entry to D-46's ledger. That is also the first shipped caller of
`with_active_space`: an unsafe-boundary change whose only user is its own test
is one to be suspicious of.

**Five breaks, all red.** Dropping the space again as D-45 left it; making
`BootRegion::describe` assume rather than ask; releasing unmapped frames before
the flush; assuming provenance instead of reading it; and mapping the fault
stack starting at its guard. Eighty-five checks.

**Two of this slice's charter items moved, and one was assessed rather than
built.**

- `UFrame` → **slice 7**. D-44 is precise about when it arrives: *with the
  first caller that can map a frame into a second address space*. There is
  still only one, so building it now would be the type without the caller that
  gives it meaning.
- Stacks that arrive with their thread → **slice 7**. The cap is already a
  default rather than a ceiling — D-86 and D-88 between them make a spawn-time
  map and an exit-time unmap possible — and the remaining work is in `wost`'s
  task table, which slice 7 reshapes anyway for processes.
- The boot thread's stack → **slice 4**. It sits inside the direct map, which
  is deliberately unsplittable, so guarding it means moving the boot thread
  onto a stack of its own rather than splitting a page. That is a naked
  function that never returns, and it belongs with the trap-entry rework.
- **CET's second region is not foreclosed, and needed no code to keep open.**
  `010` §10's requirement is that the stack allocator not make a shadow stack
  impossible to add. `libs/stack-layout` computes every address from
  `(base, stack_len, guard_len, slots)` and hands back a `Slot`; a second
  region per thread is a field on `Slot` and a term in the stride, in a
  host-tested crate with no hardware in it. Building an unused slot now would
  be the speculative API `005` forbids; recording that the arithmetic is
  parameterised is the whole of what was owed.

**Still owed by this slice's neighbours.** The frame D-86's check maps is never
unmapped — that check is about the mapping being live, and it costs four pages
for the life of a boot. `unmap` frees no intermediate page tables, so a space
that maps and unmaps repeatedly keeps its tables; nothing does that yet, and the
first thing that will is slice 7.

## Slice 4 — trap entry, FRED-shaped (2026-08-24)

Recorded as built, per the charter above.

**The decode is one place now** (D-81's shape, `libs/trap-cause`). The handler
matched on a raw vector number, which spreads the architecture's meaning across
every arm that tests one — and it threw the page-fault error code away
entirely, so a report could say *where* a fault happened but never *why*. A
missing mapping, a refused one, an instruction fetch and a user access all
looked identical. They do not now.

The crate is host-tested over all 256 vectors, and its cross-check against
`x86-tables` found a real gap on its first run: vector 28, hypervisor
injection, had no name while 29 and 30 did. Two tables of the same thing had
drifted, and the test that noticed is what keeps them together.

**The gate boots a machine the kernel cannot fully use** (D-90). All three
kernels D-65 added run on `-cpu max`, where every feature is present — so every
precondition this kernel checks had only ever been met, which is
indistinguishable from never checking one. A fourth boot runs on
`-cpu qemu64,-x2apic` and requires the kernel to name the missing feature and
still reach its prompt. D-33 left no fallback to test, so the diagnosis *is* the
behaviour. The kernel already degraded correctly; what was missing was the
evidence, and breaking it proved the gap was real.

**QEMU decided where the speculation code lives** (D-91). D-81 wanted an eIBRS
write at boot and `RDCL_NO` recorded as the reason KPTI is declined. TCG does
not emulate `IA32_ARCH_CAPABILITIES` — asked directly, with two CPU models —
so the branch that enables enhanced IBRS cannot run here at all. An unexercised
branch inside the memory-safety TCB is the worst place to put one, so the
decisions went to `libs/x86-cpuid` where synthetic capability words drive them,
and `wost` kept a `cpuid` and two MSR accesses. Silence is not safety there:
a processor with no capability word is a different value from one reporting all
clear, because it is the majority case and the wrong answer for it.

**And the last stack got its guard** (D-92). It could not be guarded where it
was — the bootloader's stack lives inside the direct map, which is
two-megabyte on purpose and which D-88 refuses to split — so the boot thread
leaves it. `kernel_main` ends at the switch. The primitive is safe because its
argument is the proof: `map_stack` maps the region and then *verifies the guard
is absent*, and no caller could have told from the return value. **D-193 closes
the later fail-open edge:** if that mapping refuses, diagnostic kernel service
continues but process construction returns a typed refusal before allocation
and ring 3 remains closed.

The alignment had history. The continuation is entered by `jmp`, so nothing
pushes a return address, and the eight bytes the ABI expects come off at the
load — the same off-by-one word D-76 found in the fabricated first frame, where
it was symptomless without SSE and invisible to every register check.

**`002`'s security baseline now has no exceptions.** Every stack this kernel
runs on has an unmapped page beneath it, and each has a check that goes red
without it. `000`'s table says so too.

**Four breaks, all red**: a fault reporting a vector that is not its own, the
two vector tables drifting apart, the x2APIC refusal being swallowed, and the
boot thread declining to move. Eighty-six checks, and the gate boots four
kernels.

**Three of this slice's items moved, each because it has no caller yet.**

- **`VERW` on exit-to-user → slice 5.** D-81 paired it with eIBRS as a
  one-liner, but one is a boot-time MSR write and the other is an instruction
  on a return path this kernel does not have until ring 3. `libs/x86-cpuid`
  already decodes the `MDS_NO` bit that gates it.
- **Trap stacks as an enum → slice 5.** There is exactly one IST user, and an
  enum with one variant is not an abstraction (`005`). Ring 3 is what adds the
  second.
- **The Limine base-revision-5 bump → slice 11.** It retires the
  invariant-TSC warning and guarantees the EFI system table stays mapped, but
  it means replacing a checksum-pinned bootloader binary (D-25) — a deliberate
  change, and one whose only failure mode is not booting. It belongs with the
  phase close, where a VMware boot is already being taken.

**Still owed by this slice's neighbours.** `ENDBR64` on the indirect targets in
`global_asm!` is written down (D-81) and not written: nothing enables kernel
IBT, so today it would be an instruction with no effect and no way to tell if
it were wrong. It arrives with the first thing that turns IBT on.

## Slice 6 — the handle table (2026-08-24)

Recorded as built, per the charter above.

**A handle is arithmetic** (`libs/handle-space`, D-78). Reserved low bits, a
generation sharing the word with the index, a per-table mixer. Each property
earns its place: the reserved bits make the encoding non-dense, so no
implementation of this interface *can* promise the lowest free handle even if a
later author wanted to; the generation turns use-after-close into a decode
failure; the mixer means a value that leaked from one table is not a handle in
another.

The generation **wraps** rather than saturating, and that is the interesting
choice. A saturating counter would make one slot's last generation permanent,
so every later handle for it would decode as valid for ever — the failure the
counter exists to prevent, made permanent instead of transient.

**One arena, and one ownership comparison.** Lookup is decode, bounds,
generation, owner. A table is a refcounted object rather than a field of a
process, because Zircon's strict per-process invariant was the first casualty
of its own compatibility layer. `Arc` is the refcount: D-78 wants
`inc_not_zero` from day one, and `Weak::upgrade` is that operation with the
atomics already audited.

**`Event` is the first object** — no memory, no address space — so it exercises
the table, rights, refcounting and lifetime without waiting on ring 3.

**Building it found a bug in Phase 4** (D-93). `unblock` changed who was
runnable and armed no timer, which this kernel's own module doc forbids. Nobody
noticed because the only waker was an interrupt handler, where the trap exit
reschedules anyway; the first waker in *thread* context left the woken thread
Ready with nothing to run it, on a tickless kernel where there may be no timer
at all. The check that found it timed out rather than failed, twice, before it
was sharp enough to say so.

**And an adversarial pass found four more** (D-95), of which two matter most.
`Event::wait` checked its condition before registering, so a signal in the gap
woke an empty queue and the waiter slept for ever — and three doc comments said
that could not happen. `WaitQueue::wait_unless` does both under the lock a
waker must take. Separately, `ARENA` held its leaf guard across `Vec` growth,
quietly rebuilding the `ARENA → HEAP → FRAMES` order D-85 had just deleted.

**D-78's own rule was being broken by the first object.** *A right must
correspond to a distinction the kernel actually makes about that object* — and
an event shipped with `READ`, `WRITE`, `EXECUTE` and `MAP` granted, with the
check named for rights using that grant as its fixture. An event has no pages.
`Rights::are_meaningful_for` and a per-type mask make the rule enforceable, and
`insert` refuses the rest.

**Eight machine checks and fifteen host tests.** The two D-28 singles out as
most likely to be silently unfailable — *the invalid value never decodes* and
*a reused slot never yields the same value twice* — were broken first. So was
`attenuate`, whose clamping form reads like a convenience and turns "give me
more than I have" into a silent success, and `contains`, whose any-of form
reads identically to subset and grants everything.

**Still owed by this slice's neighbours.** `mod handle` and `mod object` are
`#[cfg(feature = "selftest")]`: they are built and proven and nothing ships
them, because the first shipped caller is `spawn`. The gate comes off in slice
7 with it. And `NotYours` versus `Stale` is an occupancy oracle over a global
arena — a table can learn which slots another holds. Zircon collapses both to
one error for that reason, and it is a decision for the slice that has a
syscall to surface them through.

## Slice 5 — ring 3 (2026-08-24)

Recorded as built, per the charter above.

**A gate of its own, and not for the reason D-81 expected.** It left the
`syscall`-versus-trap-gate question open and said the measurement that would
settle it is meaningless under TCG. It is settled on something else: `syscall`
needs `swapgs`, `swapgs` needs a per-CPU block, and that is two unproven
mechanisms in the commit that establishes a privilege boundary. A gate the
CPU reaches through the interrupt table needs neither — it loads `RSP0` from
the task-state segment itself. (An *interrupt* gate at DPL 3, not a trap gate:
the flag is clear on entry, and `libs/x86-tables` offers no trap gate at all.) So the
boundary goes up on the simplest thing that can hold it, and `syscall` becomes
an optimisation with something working to compare against, on hardware, where
the comparison means something.

Exactly one gate admits ring 3, and it is named rather than defaulted. Every
other vector faults when user code says `int`, because a device's line is not a
service.

**`enter_user` is safe because it checks** (D-96). `iretq` will send the CPU to
an unmapped address in a privilege level that cannot report it, so the tables,
the entry's user-executability, the stack's user-writability, the alignment and
`RSP0` are all walked first. Breaking the code page's `USER` bit produces a
sentence and a red check where it would otherwise produce a triple fault.

**The evidence is the selector the CPU pushed.** A ring-3 `CS` in the trap
frame cannot be manufactured by a kernel thread saying `int 0x80` — the number
is the hardware's account of where the caller was. And when the gate is closed,
the machine confirms the whole boundary at once: *general protection fault,
error 0x402* — IDT entry `0x80` refused — *at cs 0x33, rsp 0x300000001000*.

**The user copy is the boundary** (D-97), and the release build found the bug
the debug build could not. `stac` and `clac` were given `nomem` and
`preserves_flags`; both are lies. `stac` changes whether a *later* access
faults, so `nomem` let the optimiser move the copy out of the window — and the
flag it sets is `EFLAGS.AC`. Debug passed. Release took a protection violation
reading the page it had just been given permission for. D-65 made the gate boot
both profiles for overflow checks; this is the same lesson through a different
door.

**`VERW`'s gate moved so that it could be reached** (D-98). D-81 wanted it
gated on `MD_CLEAR`, which QEMU does not report — so gated that way the
instruction would have shipped unexecuted, in the state eIBRS is still in. It
is armed unless the processor claims `MDS_NO` instead, because `verw` exists on
every x86-64 and `MD_CLEAR` says only whether it also flushes. A machine whose
capability word cannot be read gets the instruction rather than the assumption.

**Three items moved, each with a reason rather than a shrug.**

- **`syscall`/`sysret`, `arch::percpu` and `swapgs`** → the slice that measures
  them. The descriptor order is fixed for `sysret` already: `USER_DATA` sits
  *before* `USER_CODE`, which looks backwards and is what `STAR + 8` and
  `STAR + 16` require. Moving a descriptor later means moving every selector
  that names it, so the order is settled now and the mechanism is not.
- **Trap stacks as an enum** → still one IST user. An enum with one variant is
  not an abstraction (`005`).
- **Eager XSAVE save and restore** → **slice 7**. D-82's arithmetic is done and
  host-tested, including the APX case that broke Linux's monotonicity
  assumption. What is missing is per-thread buffers and a save on every switch,
  and **D-99 made it reachable**: `RSP0` follows the thread now, so two can be
  in ring 3 and two can have floating-point state to lose. Nothing does yet —
  the program is twenty-two bytes and touches no SSE — but the gap is real from
  D-99 onward rather than hypothetical, and it arrives with the processes in
  the rest of this slice.

**A ring-3 fault used to stop the machine, and D-102 closed it.** D-31 made a
fault terminal when a fault was necessarily a kernel bug. After D-96 it can be
a *program* bug: a `ud2`, a division by zero, or a touch of an unmapped page in
ring 3 would panic the kernel. The trap path now distinguishes —
`is_the_programs` in `trap.rs` refuses `#DF`, `#MC`, NMI and anything from
ring 0, and everything else from CPL 3 retires the caller — and
`a_faulting_program_dies_alone` is in the gate. What remains owed is the
neighbour below: a fault *during a user copy* is still terminal.

**Still owed by this slice's neighbours.** A fault *during* a user copy is
still terminal, where a real kernel would recover and return an error — that needs a
fixup table keyed on the faulting instruction pointer, and the trap dispatch
D-81 built is what would consult one. And the retired ring-3 thread keeps its
slot and its two pages for the life of the boot; nothing reclaims a retired
thread until `spawn` has a counterpart.

## The phase closes (2026-08-24)

### The debts, settled

Every row of the table at the top of this file, with what actually happened
rather than what was proposed. Where the answer is "not paid", it says so and
says where it went.

| Debt | Settled |
|---|---|
| Boot and double-fault stacks unguarded | **Paid.** D-89 guards the double-fault stack; D-92 moves the boot thread onto a guarded one, since the bootloader's could not be guarded where it sat |
| The address space cannot grow after boot | **Paid.** D-86: `activate` borrows rather than consumes, and mapping is reachable on a live space |
| Memory objects, moved out of Phase 3 | **Paid in the part that had a caller.** Owned frames, `map_user`, `map_shared` and the untyped split are here; a first-class `MemoryObject` is still absent and waits for the later zero-copy/runtime caller that can determine its shape |
| `map` accepts any physical address without evidence | **Paid.** D-87: every mapping entry takes evidence — an owned run, a `BootRegion`, or a `USegment` — and D-105 closed the two doors that let the evidence be forged |
| The untyped split, absent on purpose | **Paid.** D-107, with the caller it was waiting for |
| Port claims prove exclusivity, not entitlement | **Not this phase, as planned.** Phase 6, with PCI discovery |
| XSAVE | **Paid.** D-108: eager, boot-sized, out of line, and the compacted form is live — the VMware boot reports `compacted`, which QEMU never exercised |
| `swapgs` and the GS base | **Not paid, and the reason changed.** Nothing needs a per-CPU word yet: there is one CPU, `SPINLOCKS_HELD` is a global, and the thing `swapgs` exists to protect — a kernel entry finding the wrong `GS` — cannot happen until there is a second CPU or a `syscall` instruction. It moves to Phase 12 with SMP, where it has a reason rather than a slot |
| No ring-3 descriptors | **Paid.** D-96 |
| The lock discipline in the borrow checker | **Paid.** D-85, first, so nothing here was written twice |
| No preempt-disable mechanism | **Not paid, and it should be said plainly.** Every lock a trap handler can reach still disables interrupts, which is correct and is not what D-70 rule 2 asked for. A preempt-only lock has no caller that would benefit on one CPU, and building one without a caller is how an abstraction gets the wrong shape. Phase 12 |
| The per-CPU word widens | **Not paid**, with `swapgs`, and for the same reason |
| Per-process quota | **Paid at the handle layer** (D-111: 64 handles per process). **Not paid** for frames or for per-message allocation, which is D-44's other half and needs an accounting owner |
| A wait with a timeout, and a wait on two queues | **Not paid.** Both were to fall out of the completion queue, and the ring is built but nothing blocks on it yet. The queue's wait exists and is unused |
| The thread cap of sixteen is a real cap | **Paid in principle, not in fact.** D-86 and D-88 made a stack mappable at spawn and unmappable at exit; the arena is still committed at table-build time, so sixteen is still sixteen. What changed is that it is now a default rather than a ceiling |
| Every new hardcoded window is KASLR interest | **Paid as interest.** Two more windows: the ring at `0x0000_2000_0000_0000` and the boundary probe's pages. The ledger is D-46's |
| `002`'s false claim about delegation | **Paid.** D-111 makes the corrected sentence a check that counts the table |

### What the phase cost, measured

- **124 machine checks**, from 79 at the Phase 4 close.
- **343 lines of `unsafe` in `wost`**, in 143 blocks, against a budget of 380
  that `cargo test` now enforces. `kernel/` and every `libs/` crate have none,
  and the counter checks that too rather than trusting the lint.
- **Fifteen new decisions**, D-100 through D-114, and five amendments.
- **Four adversarial audits** during the phase (D-94, D-95, D-101, D-105) plus
  the closing one, D-115.

### What the VMware boot showed that QEMU cannot

Recorded the day it happened (D-18), and it earned its place rather than
confirming a green already had:

```
cpu: protections smep smap umip nx
cpu: extended state 832 bytes, xcr0 0x7, compacted
cpu: capabilities 0x44 rsba
cpu: buffers cleared on every return to ring 3
apic: x2apic id 0 version 0x15 (bootstrap)
time: tickless: counter at 3412636204 hz (invariant), apic timer at 65939442 hz
kernel: reading the keyboard, uptime 4 ms
```

Four things there are not reachable under the gate's QEMU: SMEP, SMAP and UMIP
actually enabled; the **compacted** `XSAVEC` path, where QEMU takes the
uncompacted one; a real x2APIC, where the gate's degraded boot has none by
design; and a processor that reports `MD_CLEAR`, so the `VERW` on the way out
of ring 3 is doing something rather than merely executing. D-91 recorded that
QEMU reports no capability word; this is the machine that does.

### What the closing audit found

D-115, in D-77's pattern, held to D-105's two rules. Five reachable defects:
a boot check reading numbers ring 3 owns (one store panicked the debug
kernel); `xstate::init` enabling SSE before three paths that refuse, leaving
two processes sharing the register file with nothing saving it; a safe public
walk taking a bare physical address; `map_shared` and `map_user` able to alias
one run; and three `lock_leaf` obligations breached by this phase's own code.

Nine checks could not fail and three failed for the wrong reason. Two are worth
carrying forward as patterns rather than as incidents:

- **A check compared two places the thing under test never touched.**
  `a_rewritten_submission_cannot_change_what_was_served` read a completion slot
  128 bytes from the submission it rewrote, so it reduced to `posted == posted`
  — *no line in `queue.rs` could falsify it*. The question that finds this is
  "which deletion turns this red", asked before the check is trusted.
- **A check read a Rust early return where it meant to read a register.**
  `extended_control()` began `if !is_enabled() { return 0 }`. This is D-105's
  own `VERW`-arming finding, three decisions later, in a fresh costume. The
  guard must be the hardware, not a flag remembering the hardware.

And one finding did not reproduce as described. The audit rated `reap_up_to`'s
unvalidated head as defeating the no-dropped-completion claim; `Ring::reap`
already refuses on an empty queue, so a lying head gains nothing. The bound was
added anyway, and then *neither* bound could be shown load-bearing because each
covers every input the other does. D-115 says so rather than shipping a
confident comment on a redundant guard.

### Still owed, and where it went

Nothing here is a surprise; each is named in the ADR that created it.

- ~~A program cannot drive its own ring.~~ **Closed by D-116**, after the
  phase's other work was done: the gate dispatches on `rax` and only `exit`
  retires, `serve` reserves at the doorbell rather than in an earlier call, and
  the initrd's program posts an operation, rings the doorbell and reads its own
  answer. The shipped kernel runs it — a kernel that has userspace and never
  enters it has not shipped userspace. 125 checks.
- ~~**`FS_BASE` is a fourth thing that must follow a thread**, found by
  writing the sysdeps (D-114).~~ **Closed by D-118.** It lives in
  `task::prepare` with `RSP0`, `CR3` and the extended state. Under
  D-114's former vocabulary that made it one of four MAPS entries; D-181's
  exact register calls its body WIRED and its platform remainder WORKS. The
  kernel range-checks the base against `USER_END`: a program asking to point
  `fs:` at kernel memory has no reason it could be given.
- ~~Nothing is reclaimed.~~ **Closed by D-117.** A retired program's resources
  become a zombie and are collected by the idle thread — which *is* "the
  machine has nothing better to do" — or by the next spawn, on a machine busy
  enough never to idle. An address space now remembers its own page tables so
  it can give them back, `bind` borrows rather than consumes so the space
  survives the thread, and the check runs forty programs through sixteen slots
  with the frame count steady. The program's *stack* is still the boot-time
  arena, so sixteen remains a cap on concurrency — no longer on programs ever
  run. **D-190 removes D-117's former 32-table tracking ceiling:** ownership
  grows fallibly before a table frame is taken, every table returns, and a
  refused table cleanup is a typed, logged and counted result rather than a
  zero-valued success record. **D-191 closes construction as well as
  destruction:** a never-run process remains one rollback transaction through
  the reserved-thread commit point, and loader placements are per-call data
  with the executable entry validated before allocation. **D-192 makes the
  final ring settlement fail-atomic:** a terminal post retains its pending
  operation until publication succeeds, and collection retains the complete
  zombie until the ring is observably quiet.
- ~~A fault during a user copy is still terminal.~~ **Closed by D-120.** The
  copy is assembly with its faulting instructions between named labels, a page
  fault in that range resumes at a fixup, and the caller gets a count of what
  did not move. The trap dispatch D-81 built is what consults it. What it buys
  beyond the hole: dereferencing an arbitrary *user* address is now memory-safe,
  so `copy_from_user_unwalked` exists and is what Linux's `copy_from_user`
  has always been.
- ~~`xtask/src/elf.rs` carries its own transcription of the ELF header
  offsets.~~ **Closed by D-158**, which moved the builder to `libs/elf64`.
- ~~No completion carries a handle.~~ **Closed by D-163** for same-object
  Duplicate; D-173 later added OpenAt's cross-type derivation.
