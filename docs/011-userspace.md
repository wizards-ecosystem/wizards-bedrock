# 011 — Userspace (Phase 5)

The phase that gives the kernel someone to serve. Ring 3, a handle table that
*is* a process's power, `spawn` in place of `fork`, ELF images loaded from an
initrd, channels that carry authority, and the submission/completion rings the
native ABI is made of — closing with a stub mlibc sysdep whose only job is to
find out whether a POSIX libc can sit on this ABI while the ABI is still cheap
to change (D-74).

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
| 4 | Trap entry, FRED-shaped: `libs/trap-cause` and table dispatch, trap stacks as an enum, the boot thread's own guarded stack, `VERW` and eIBRS, x2APIC as a checked precondition, the Limine base-revision-5 bump | |
| 5 | Ring 3: user descriptors, `syscall`/`sysret`, `arch::percpu` and `swapgs`, the SMAP copy guard, `libs/xstate` and eager compacted `XSAVES` | |
| 6 | The handle table: `libs/handle-space`, rights in one `u32`, a refcounted table, the object header, per-process quota | |
| 7 | Memory objects, spawn, ELF loading, initrd: `libs/elf64`, a second address space, `UFrame`, stacks that arrive with their thread, two processes, exactly one handle each at spawn, and `THREADS` split into a thread table and a run queue | |
| 8 | Channels: bytes and handles, attenuate-on-send, rights declared per field with no defaults | |
| 9 | The async queue ABI: the model before the protocol, `libs/ring-proto` under shuttle and proptest, reservation backpressure, exactly one terminal completion | |
| 10 | The stub mlibc sysdep (D-74), on a vendored clang | |
| 11 | Phase close: the VMware boot (D-18), the adversarial audit (D-77), the roadmap and this file's closing section | |

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

**A signal is a completion-queue entry.** The blocking wait is interruptible
because a wakeup is indistinguishable from a completion — the same mechanism,
not a second event plane, which is the same argument that unified ports and
completion queues above. There is no in-kernel signal frame, no signal mask in
the kernel, and no restart-after-interrupt semantics to get wrong.

What this costs, recorded now so slice 10 is not surprised: mlibc's `Sigaction`
becomes a userspace dispatch loop over CQ entries, and the POSIX behaviours
that assume a kernel-delivered signal interrupting an arbitrary instruction —
`EINTR` on a blocking call, signal-safe reentrancy, `SIGSEGV` handlers that
resume — are either emulated in the libc or unimplementable. That is the same
cluster Plan 9's APE could not implement and Fuchsia excluded from POSIX Lite;
slice 10's job is to find out which half this falls in, and to record the count.

## The lock order

This section is the authority; module docs restate it, the ADRs cite it.
`009-multitasking.md`'s section is superseded by this one from slice 2 onward.

Phase 4 left `HEAP → FRAMES` (D-47) with the thread table and each wait queue
as leaves, and a rule between lock *kinds*: a `ThreadLock` may never be taken
while a `SpinLock` is held (D-63), checked by a count.

Slice 2 changes what enforces all of it. `SpinLock::lock` takes
`&'a mut ThreadContext` and the guard borrows it, so taking two spin locks is
`E0499` and blocking with one held is `E0499` — the ordering stops being a
document and becomes a compile error (D-69). Where nesting is genuinely needed
the second acquisition takes the *guard*, and a type-level rank in
`libs/lock-rank` says which nesting is legal. The runtime assert is not
removed: two independent checks is what OSTD ships, and the assert covers the
paths the borrow checker cannot see.

The preempt-only lock is a third kind and sits below both: it disables
preemption and leaves interrupts on, so it may be taken while nothing else is
held and may not be held across a block. Never write "disable interrupts" where
"disable preemption" is meant — the single most-cited breakage across the SMP
survey, and the conflation this kernel currently has.

Slices 6 through 9 add the handle table, the object arena, and each ring. Their
order relative to the above is fixed when each lands, and the rule for all of
them is the one D-70 rule 5 already binds: **interrupt handlers never block**.

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

That spawn check is the one worth naming twice. `002` pillar 1 claimed channels
were the sole delegation mechanism, which is false even of Zircon — it also
passes handles through `zx_process_start`. The replacement is better than the
claim because it is checkable:

> A process receives exactly one handle at spawn — its bootstrap channel — and
> every subsequent handle arrived through a channel.

Slice 7 fixes `002` and lands the check in the same commit.

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
