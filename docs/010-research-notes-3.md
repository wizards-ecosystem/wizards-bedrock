# 010 — Deep Research III: Verifying the Foundation

Produced 2026-08-23, at the close of Phase 4 and before Phase 5 fixes the two
interfaces this project will live with longest: the handle table and the async
queue ABI.

Where `003` surveyed the state of the art before the code existed, and `008`
closed `003`'s gaps and armed Phase 3, **this pass is verification-first**. Its
question is not "what should we build" but "is what we already decided true".
Eleven parallel research angles, every one instructed to try to *refute* the
project's own recorded claims, plus a local audit of the docs against the code
and a set of claims compiled and run against the pinned toolchain.

**Method and its honest limits.** Each angle fetched primary sources and
returned an explicit verdict per claim. Several angles exhausted their search
budgets and say so; where a finding rests on a single source or on inference,
this document says which. Marks used below:

- **✓** — verified locally, by compiling or running it against this repo's
  pinned toolchain (`rustc 1.97.1`) or by reading this repo's own code.
- **◆** — a primary source was fetched directly.
- **⊕** — two independent angles reached it without sight of each other.

The three debts `008` §7 left open are now closed or answered: the Tier-2
re-verification, the search for independent replication of the Asterinas
numbers, and framekernel SMP scalability. Two of the three answers are not the
ones the project was expecting.

---

## 0. What held, what moved

| Decision | Verdict |
|---|---|
| D-13 framekernel discipline | **Holds** — the property we rely on is *verified*; the number we quote is *wrong* |
| D-06 async submission/completion queues | **Holds — rationale must be rewritten**; the recorded reason is refuted by its own source |
| D-06 capability handles + rights | **Holds**, and `002`'s open rights question is now answered |
| D-04 monolithic-modular | **Holds**, unchallenged by the evidence |
| D-15 EEVDF, tickless | **Holds** |
| D-45 direct map, 2 MiB | **Holds** — confirmed by measurement |
| D-07 spawn-only | **Holds as a kernel design**; the compatibility bill is unbudgeted |
| D-10 POSIX as a translation layer | **Holds, but the sequencing is wrong** and the surface is undocumented |
| D-08 WizFS CoW B-tree | **Design holds; the schedule does not** ⊕ |
| D-09 WASI 0.3 components first-class | **Should change** — the named runtime cannot run the named standard |
| SMP at Phase 12 | **Bring-up: correct. Design discipline: must move to Phase 5** |
| D-16 self-contained Windows build | **Holds, with a tax to record** ✓ — a toolchain gap gates host-side crates; passable with pins, fixed by vendored binutils |

Five things this pass found that no prior pass did:

1. **`002-architecture.md` contains a factual error** about Zircon that a
   selftest can replace with something checkable.
2. **Our lock guards are `Send`** ✓ — latent on one CPU, a real hazard on SMP,
   one line each to fix.
3. **The borrow checker can enforce our lock discipline** ✓ — the runtime
   `SPINLOCKS_HELD` counter has a compile-time form, verified working on the
   pinned toolchain.
4. **There is no preempt-disable mechanism at all** ✓ — `SpinLock`'s `cli` is
   doing that job, which is the exact conflation the SMP literature names as
   its most-cited breakage.
5. **Verus runs on this machine, on stable, today** ✓ — and `008` recorded its
   cost 2.26x too high. The verification tier is reachable; a missing `as.exe`
   gates part of it, and two version pins get past that today.

---

## 1. The framekernel — keep the discipline, drop the metric

`002` pillar 5 and D-13 rest on Asterinas (ATC'25). `008` §7 flagged the numbers
as author-measured and unreplicated. Both halves of that flag are now resolved,
in opposite directions.

**The containment property is verified, and it is the one we actually rely on.**
An agent went to the live repository to break the boundary and could not: there
are **zero occurrences of `#[allow(unsafe_code)]` in the entire Asterinas
repository** ◆ — checked by whole-repo code search, with every non-`lib.rs` hit
read individually. Real drivers — virtio, NVMe, PCI, i8042, framebuffer, uart,
block, network — all live in `deny(unsafe_code)` crates. This holds at ~100k
lines, 90 contributors, four years. **The discipline scales.** That is the claim
D-13 depends on, and it is now empirical rather than aspirational.

**The 14.0% TCB figure does not survive scrutiny, and it is the figure we
quote.** From the paper's own Table 9: Asterinas TCB = 10,571 lines; Tock =
2,903. **Asterinas's TCB is 3.6× larger than Tock's in absolute terms**, and the
paper reports the comparison as a 3× improvement. A ratio improves for free as
safe features accumulate. For the question that matters to this project — *how
much unsafe must one person audit* — Tock wins outright. Two further problems:
the numerator is very likely OSTD-only (the workspace labels `hashbrown`,
`bitvec`, `lru`, `itertools`, `spin`, `zerocopy` as "external dependencies only
for safe crates", where *safe* means authored without writing unsafe, not
linking none), and the line-counting method is blind to the highest-risk unsafe
— `unsafe impl Send for Foo {}` contributes approximately zero.

**Consequence for us: track absolute unsafe lines in `wost` against a budget,
and never a percentage.** The percentage is the metric on which Asterinas loses
to Tock while claiming to win.

**The performance caveats are worse than `003` recorded.** `mitigations=off` is
not the problem — it makes Linux faster, so it is conservative. But
`hugepages=0` disables transparent huge pages on the Linux baseline, and Redis
— their best result at 1.31 — is a THP-sensitive workload. And smoltcp's
missing congestion control is a pure win over a lossless tap, so **both network
results measure a simpler TCP stack, not a faster kernel**. The defensible
summary is *"single-core syscall dispatch is at parity"* (LMbench 1.08), and
nothing broader.

**Independent replication: none exists.** Not weak replication — none. LWN's
coverage is descriptive, running no benchmarks and auditing no measurement.

**The project itself is alive and accelerating**: 4,817 stars, 90 contributors,
1,111 commits in 2026 alone, v0.18.1 released 2026-08-18, and a distro running
Firefox and QEMU. The bet is not on a dead project.

**The strongest counterargument, and it is legitimate.** Containment and
soundness are separable properties. *Containment* — all unsafe in one crate, an
audit surface that does not grow with features — costs one lint attribute.
*Soundness* — safe code cannot cause UB however hostile — is where all the
difficulty lives: the typed/untyped frame model, `!Send` guards, sensitivity
types, and a framework that re-validates untrusted policy output. It exists to
defend against your own service code being wrong, and in a solo project you are
the only author of that code. Two things sharpen it: **Converos (ATC'25)
model-checked 12 Asterinas concurrency modules and found 20 bugs in the safe
code** — races, deadlocks, livelocks, panics — so property 2 buys less than it
appears; and memory safety is not reliability, since a misplaced `unwrap()` in
safe Rust produces the identical outage.

The answer is to **take containment unconditionally and soundness selectively**,
and to name that 80/20 in the ADR as the fallback position rather than discover
it under pressure.

**Patterns worth stealing**, all of which fit conventions this repo already has:
capability tokens as arguments (`fn alloc(&mut self, guard: &DisabledLocalIrqGuard)`
makes "I am pinned to this CPU" a compile-time fact); `!Send` guards; and
sensitivity in the type (`IoMem<Sensitive>` vs `IoMem<Insensitive>`, with the
allocator pre-removing privileged ranges) — which is the mechanism that makes
safe-Rust drivers actually work.

---

## 2. The async ABI — keep the decision, rewrite the reason

D-06 and `003` justify async-first with FlexSC: *"the win comes from batching +
cache locality (FlexSC), not from avoiding the trap."* **That sentence is
refuted by FlexSC itself.** ◆

The paper's numbers are exact as recorded — 150 cycles round trip, up to 65%
IPC degradation, 44 d-TLB and 3,160 L3 entries evicted by a single `pwrite`.
But §2.3 says the *direct* cost dominates below ~2,000 instructions per syscall
and the indirect cost only in the 2,000–100,000 band — and two of the paper's
own three motivating workloads sit on that crossover. And §3 says batching
*"minimizes the frequency of switching between user and kernel execution,
**eliminating some of the mode switch overhead** … improves **both** the direct
and the indirect costs."* Batching and trap-avoidance are the same lever, not
alternatives.

**The 2026 evidence gives a cleaner ordering.** A July 2026 study of io_uring
for database systems provides an attribution ladder from an already-async
batched design: +registered buffers +11%, +NVMe passthrough +20%, +IOPOLL +21%,
+SQPOLL — eliminating the trap entirely — +32%. Against *blocking* I/O the same
design is **up to 12.5×**. So:

> **Structural asynchrony ≈ 10×. Total trap elimination ≈ 1.3×, and it costs a
> dedicated core.**

Meanwhile the trap itself became 5–10× *more* expensive than in 2010 (76 ns
unmitigated versus 231–560 ns mitigated), while caches grew 4–16×, making the
pollution term relatively smaller. A second measurement from the same paper is
the one to internalise: **naive io_uring adoption bought only 1.06×**; the 33×
came from the *caller* becoming asynchronous.

**What this changes.** The decision stands and is well-founded. The ADR's
rationale should be replaced with the measured ordering, and the design
consequence is sharper than "use rings": *the win is in making async the natural
way to write a service, not in the ring itself.* An ABI that lets callers stay
synchronous will deliver the 1.06×.

**The four hard problems, with answers the evidence supports:**

- **Cancellation is advisory.** io_uring's own manual: `-EALREADY` means *"the
  caller must wait for the canceled request to post its completion event."*
  Rule: **every operation posts exactly one terminal completion, always.**
- **Buffer lifetime: the kernel must own the buffer.** Blocking in `Drop` is
  unsound because leaking is safe. Windows RIO states the rule kernel-side —
  page-locked at registration, deregistration forbidden while requests are
  outstanding.
- **Backpressure: reserve completion-queue space at submission.** io_uring's
  `CQ_NODROP` took an unbounded kernel memory liability for a misbehaving
  application; **RIO's reservation-based admission control makes overflow
  structurally impossible.** Steal RIO's, not Linux's. Also steal Windows'
  requirement that the CQ be a power of two **≥ 2× the SQ size**.
- **Shared-ring security: copy each entry out exactly once, then validate the
  copy.** The double-fetch is the canonical hole, and it is not theoretical —
  CVE-2025-39816 is literally *"always use `READ_ONCE()` to read ring
  pointers."*

**Ports and completion queues are the same shape — build one, not both.** A
Zircon port *is* a completion queue whose entries are events. Building both
gives userspace two event planes to join, which is exactly why io_uring grew
`poll_add`, `futex`, `waitid` and `msg_ring`. Lift Zircon's four semantic wins
into the CQ instead: one-shot arming by default, edge-versus-level as an
explicit flag, **cancellation that purges already-queued entries**, and exactly
one waiter released per entry.

**On io_uring's security record**, which `003` cites as a warning: the ~60%
kernelCTF figure is confirmed ◆ and Google still disables io_uring in 2026. But
the statistic is frozen and unreproducible *because* they disabled it, and the
2025–26 CVEs cluster in **newly added** surface — `zcrx`, provided buffers,
`ublk`, passthrough. NVD returns **143 io_uring CVEs**, rising, not falling:
2024:38, 2025:40, 2026:36 by partial year. By class: **50 race/TOCTOU, 23
use-after-free, 10 refcount** — the signature of asynchronous shared-memory
deferred completion. **The lesson is feature velocity on a ring surface, not
io_uring the name**, and it argues for the austerity `003` already adopted.

One consequence nobody has written down for us. ARMO's 2025 "Curing" rootkit did
command-and-control **without making any system calls**; Falco, Tetragon and
Microsoft Defender were all blind to it. **If the native ABI is a ring, then
"observe the syscalls" is not a valid basis for auditing, sandboxing or
introspection.** Linux discovered this after the fact. Every observability and
security mechanism this kernel will ever want must be designed into the ring
from the start.

---

## 3. Capabilities and the handle table — `002`'s open question, answered

### The error to fix first

`002` states that **channels are the sole handle-delegation mechanism**. This is
**false, even of Zircon** ◆, which also passes handles via `zx_process_start()`.
Fuchsia's own documentation says so in as many words.

The replacement is better than the claim, because it is checkable:

> A process receives exactly one handle at spawn — the bootstrap channel — and
> every subsequent handle arrived through a channel.

A selftest can assert that a freshly spawned process's table holds exactly one
entry. That converts a doc sentence into a D-28 check.

### Rights: small, and the rule matters more than the number

The evidence is decisive and one-directional. seL4 has **4** rights; Zircon
**25**; CloudABI **31**; Capsicum **~81** (not the ~60 `003` recorded). Two
outcomes settle it:

- **Capsicum's taxonomy outgrew a machine word and broke its ABI.**
  `cap_rights_t` is now a multi-word struct spending **7 of every 64 bits on
  metadata**, with an `init`/`set`/`is_set` API because you can no longer OR two
  rights together. ◆
- **WASI deleted its rights system entirely in Preview 2**, on the grounds that
  it *"created a lot of ambiguity about how it mapped to POSIX semantics."* ◆

Independently, managarm — which added fine-grained rights in August 2026, ten
years in — landed on **~14**. ⊕

**Recommendation: 16–24 rights in a single `u32`**, splitting universal
(`DUPLICATE`, `TRANSFER`, `WAIT`, `INSPECT` — Zircon's `RIGHTS_BASIC`) from
per-type. And the rule that keeps it there:

> A right must correspond to a distinction the kernel actually makes about that
> object — never to a syscall name.

That is precisely the rule WASI's rights violated.

Two mechanisms to adopt from day one. **Attenuate-on-send**: Zircon's
`zx_channel_write_etc` can strip rights during transfer, including stripping
`TRANSFER` itself to yield a non-delegatable handle for one bit. It costs ~4%
and makes least privilege the default. And **rights declared per handle-field
with no defaults** (Fuchsia RFC-0028: *"Defaults have unclear behavior that can
be inconsistent across object types"*), validated fail-fast at transfer, with
both ends declaring — managarm's `PushDescriptor`/`PullDescriptor`. ⊕

### The handle table: one encoding decision, reached twice

**Zircon has no per-process handle array** ◆. One global arena, `base_value =
generation ‖ arena_index`, and the handle value is
`(base_value << 2 | 0b11) ^ per_table_mixer`. Lookup is pure arithmetic plus a
single ownership comparison.

Two independent angles converged on copying this, for different reasons. From
the security side: generation tagging kills use-after-free on handle reuse. From
the scalability side: the **Scalable Commutativity Rule** (Clements et al.,
SOSP'13) — *"whenever interface operations commute, they can be implemented in a
way that scales"* — and Zircon's contract is commutative **by construction**,
because the reserved low bits forbid a dense encoding. POSIX and Capsicum
descriptors are not: "lowest free descriptor" is an ordering promise that
serialises allocation. sv6 measured the alternative (`O_ANYFD`) scaling linearly
**with no single-core penalty**: *"at one core, both cases perform
identically."*

So, concretely:

- **The handle table must not promise "lowest free handle."** ⊕
- **Put the encode/decode arithmetic in a `libs/handle-space` crate**, beside
  `port_space` and `frame_space`, host-tested with doctests; `wost` keeps only
  the arena and the lock. This is D-30 and D-43's pattern applied again.
- Per D-28, the two checks most likely to be silently unfailable are *"the
  invalid value never decodes"* and *"a reused slot never yields the same value
  twice."* Break both.
- **Shape it as `HandleTable { table: AtomicPtr<HandleArray> }`** — Linux's
  `files_struct`/`fdtable` split, which Linux took nine years to reach — so
  readers can go lock-free later without touching call sites.
- **Make the handle table a refcounted object, not an inline field of
  `Process`.** Zircon's strict per-process invariant was the first casualty of
  its own compatibility layer (RFC-0261 shares handle tables for Starnix). Free
  now; a redesign later.

### Revocation: don't

Zircon has none. Capsicum has none. seL4 has real recursive revocation and paid
for it: operations are **preemptible and resumable** (not "abortable", as `003`
recorded), progress stored in a zombie capability, with a pre-mitigation
worst-case execution time of **about one second**, reduced to 436 µs computed /
81 µs observed.

**No revocation in Phase 5.** Object death plus a strippable `TRANSFER` right
covers the real cases. If it is ever needed, a **per-object epoch bump is
O(1)** — no traversal, no preemption points, no zombies — and the epoch is
already there, as the generation counter in the handle encoding. Put seL4's
numbers in the ADR as the reason.

### Two sizing notes

Zircon is at **26 object types and still growing**; give type encodings headroom
for 64, not 8. And policy should be **monotonic and frozen** — Zircon's
`zx_job_set_policy` is tighten-only and immutable once a job has children —
with policy bits encoded so that adding an object type does not renumber
existing ones.

### Numbers to record with their qualifiers

`003` records seL4 IPC at "~720 cycles". The real figure ◆ is **718 ± 29 cycles
on x86_64 Skylake, same-core fastpath, hot cache, *without Meltdown
mitigation*** — versus ARMv7 367, ARMv8 413, RISC-V 694, Haswell 775, and 468 →
687 (+47%) when mitigations are enabled. The number is meaningless without those
qualifiers, and the mitigated figure is the honest one to compare against.

Similarly, Capsicum's "few percent" fast-path claim ◆ is measured **against
standard UNIX syscalls, not against message passing** — the paper contains no
message-passing measurement at all. Worst case is `fstat` at **10.2%**, and
`cap_new` versus `dup` at **50.7%**.

---

## 4. SMP — the bring-up can wait; the discipline cannot

`008` §7 listed framekernel SMP scalability as unquantified. It is worse than
unquantified: **Asterinas's own scalability work became a separate SOSP'25 paper
(CortenMM) that deletes the VMA layer entirely** in favour of a transactional
MMU interface. The authors changed the design rather than measuring the old one.

But the broader question — *is deferring SMP to Phase 12 a foundational
mistake?* — has a more encouraging answer than expected.

**Deferring SMP bring-up is correct. Deferring SMP design discipline past Phase
5 is the mistake.**

The decade-long retrofits everyone cites were retrofits of *large kernels with
frozen ABIs*. Small kernels do it in months: **Maestro, 11 weeks** (87 files,
+4,592/−1,851); **Theseus, 24 days** from AP detection to eight cores; **seL4,
one release** with a big kernel lock. Quoting Linux's fifteen years overstates
the case.

Three findings sharpen it:

1. **Our locks are already SMP-shaped** ✓. `wost/src/sync.rs` uses real
   `compare_exchange(Acquire, Relaxed)` and `store(Release)`, carries
   `unsafe impl<T: Send> Sync`, and disables interrupts *before* the exchange
   with the reason written down. Three phases of correct SMP reasoning are
   already banked.
2. **Two of the three "untestable" objections are false.** Lock *ordering* is
   observable with one CPU — lockdep's own documentation says complex multi-CPU
   deadlocks *"can be detected on a plain, lightly loaded single-CPU system as
   well!"* And atomic protocols are host-model-checkable in principle (see §9
   for why that is currently blocked here).
3. **The one bug class one CPU hides completely is already named in our own
   plan.** x86's only permitted reordering is **Store→Load**, unobservable to a
   single core through store-buffer forwarding. The shape that hits it is
   store-then-load — which is exactly the **lost-wakeup re-check pattern**
   `002` already lists as owed before Phase 5. Message-passing shapes do not
   break when cores are added; this one does, silently.

### Cheap now, expensive later

| Item | Verdict |
|---|---|
| Lock granularity | Retrofit fine — Linux's fifteen years went on *recovering what the BKL protected*, not on splitting locks |
| **Lock ordering** | **Design now — highest return.** Single-CPU observation *proves* multi-CPU deadlock freedom. Redox's `src/sync/ordered.rs` (MIT) is `L0..L6` plus a `compile_fail` doctest — pure type-level, so by our own rule it belongs in `libs/` |
| Per-CPU data | Mostly fine; D-56's rationale holds. But provide the `gs:`-based accessor when Phase 5 sets up `swapgs` for ring 3, or Phases 5–11 will each invent their own global |
| Memory ordering | **Design now, free.** `Acquire`/`Release` is *identical codegen* to `Relaxed` on x86-64. Already required today: `compiler_fence`'s docs call interrupt handlers UB-prone *"despite everything happening in a single thread"* |
| RCU | Contract now, mechanism later — retrofitting "readers cannot block, publish via release" means rewriting every reader |
| Refcounts | Design now: `inc_not_zero` from day one |
| **Run queue** | **Design now — measured.** Redox ran EEVDF on a *global* run queue and **four cores were 3× slower than one** (1.367 → 4.253 µs switch latency). Per-CPU run queues, July 2026: 2 µs → 350 ns |
| TLB shootdown | Design now: unmap must *return a flush token* rather than flushing, and the shootdown IPI must not be maskable by `SpinLock` |
| Interrupt threading | Foreclose by ADR — handlers never block. FreeBSD's opposite answer touched every driver |
| **Handle table** | **Design now** — §3 |

### What to write down now, at zero code cost

- **The target model**: one big kernel lock at first bring-up, per-CPU run
  queues, correct shootdown. seL4 measured a BKL at +10% on x86 IPC and noted
  *"for 8-way parallelism … the choice of lock is essentially irrelevant"*;
  Ariel OS chose a BKL in 2025 with full hindsight. **This makes today's coarse
  locking the plan rather than debt.**
- **Never write "disable interrupts" where you mean "disable preemption."** The
  single most-cited breakage across the entire survey — Maestro, Theseus #603,
  Tock #4544. Our `SpinLock` conflates them today, and a preempt-only variant is
  much cheaper than `cli`/`sti` and is the common case.
- **Make `SpinLockGuard` and `ThreadLockGuard` `!Send`.** Verified ✓: they are
  `Send` today, and `PhantomData<*const ()>` fixes it. One line each. The idiom
  is already in the codebase — `ThreadContext` uses it.
- **No lock guard live across a context switch.** Redox's September-2025
  multi-core hang fix was **292 lines of `{ }` scoping** because Rust
  temporaries held guards across `context::switch()`. **We already enforce this
  at runtime** ✓ — D-66's assert in `prepare()` — but it was reached from a UP
  deadlock, so the ADR should name the SMP rationale too, or someone will
  weaken it later thinking it only guards a UP bug.
- **`Relaxed` requires justification the way `unsafe` does.**
- **Split `static THREADS: SpinLock<Threads>` into a thread table and a run
  queue.** `libs/eevdf` is already domain-agnostic, and
  `Entity::place(average, lag, slice)` *is* the migration hook. A struct
  boundary today, a scheduler rewrite avoided later. Asterinas makes per-CPU run
  queues an *API requirement* of its scheduler interface — and Asterinas is our
  template.

### The Phase 5 lever

The **Scalable Commutativity Rule** is the tool, and its decisive property is
that it is analysable *"before an implementation exists and before the necessary
hardware is available."* Linux scales for 68% of 13,664 POSIX tests; sv6 for
99%. **The difference is interface, not effort.** Three interface rules follow
directly:

- The handle table must not promise lowest-free (§3).
- **The async queue ABI must not promise global FIFO across submitters.** sv6's
  mail-server benchmark: an ordered socket forces one shared queue; unordered
  allows per-core queues.
- **Unmap, close and release must not promise global visibility on return** —
  the same decision as Linux's `mmu_gather` batching.

And one runtime rule: **a full barrier on both sides of the lost-wakeup
re-check.** It costs one locked instruction on the block/wake slow path, and it
is the difference between a Phase 12 that boots and one spent bisecting an
hourly hang.

**A caveat to internalise: Rust does not save you here.** Converos found 20
concurrency bugs in 12 Asterinas modules. Theseus's `ActivePageTable` /
`InactivePageTable` typestate was simply wrong once two cores existed.

---

## 5. The POSIX bet — sound, unbudgeted, and sequenced wrong

D-07 (spawn-only) and D-10 (POSIX as a translation layer) are the decisions this
pass hit hardest, and the finding is not that they are wrong. It is that the
project has priced them at zero.

**Fuchsia ran this exact experiment and reversed.** ◆ Capability handles with
POSIX confined to a userspace layer — the same shape as mlibc-over-handles.
RFC-0082 states why it was insufficient: POSIX Lite omits *"parts of POSIX that
imply mutable global state (e.g., the `kill` function) because Fuchsia is
designed around an object-capability discipline."* Their answer was **Starnix**,
a Linux kernel in userspace — **≥15 MB of Rust, ≥972 files, 493 syscall-table
entries** — followed by RFC-0261 (2024) **modifying Zircon itself** (restricted
mode, `zx_process_create_shared`, bifurcated address spaces) to make it fast,
then RFC-0264 (2025) adding a second ISA. Still committed to daily in August
2026, now with SELinux inside it. **And Starnix implements `fork`.**

**The same wall appears thirty years earlier.** Plan 9's APE could not implement
uid/gid, umask, controlling tty, `link`, or
`sigprocmask`/`sigpending`/`sigsuspend` ◆ — the identical cluster: POSIX's
ambient-authority and mutable-global-state surface. **Signals are on both
lists.** Starnix's answer is a 96 KB signals implementation. And Pike's own
estimate of the tax: *"90–95% of the work in Plan 9 was directly or indirectly
to honor externally imposed standards."*

**The mlibc budget, measured** ◆: 296 sysdep tags; **16 mandatory**, which the
porting book says is *"enough for running a basic statically-linked hello world
program"*; and real ports implement **147 (ironclad), 163 (astral), 202
(managarm), 280 (linux)**. Every one of them implements `Fork`, `Sigaction`,
`Ioctl`, `Termios`, `Poll` and `Waitpid`. Adopting mlibc also means adopting
`abis/linux` — the userspace-visible ABI becomes Linux's, pinned to a moving
upstream, and mlibc's own documentation says to pin a release rather than track
`master`.

**A concrete trap for the CPython milestone**: `subprocess`'s `posix_spawn`
fast path is guarded by a conjunction that includes **`cwd is None`** ◆. So
`subprocess.run(..., cwd=...)` — extremely common — falls back to fork+exec, as
does anything setting `uid`, `gid`, `umask`, `pass_fds`, `start_new_session` or
`preexec_fn`. A spawn-only OS must either implement a `posix_spawn` far richer
than the POSIX baseline or accept that a large fraction of real Python
subprocess calls fail. CPython itself builds and runs without fork (WASI is
Tier 2; iOS ships with no subprocessing at all), so this is a feature-surface
question rather than a build blocker — but it is not free.

**What the alternative branch bought.** Redox chose POSIX-first in 2015 and has
**2,665 port recipes** against managarm's 84 and CloudABI's 131. It implements
`fork` in *userspace* relibc — ~181 lines of deeply unsafe, ordering-sensitive
code plus an assembly trampoline, doing CoW address-space duplication through
per-grant file descriptors. That is proof a compat-fork is achievable on a
capability kernel, and proof of its price. Meanwhile CloudABI — strictly more
radical, and requiring *zero* kernel work since it ran on FreeBSD and Linux —
**died of adoption**, with its author writing *"never got enough traction to be
sustainable."* Even CloudABI kept `proc_fork`.

### The recommendation, and it is the largest in this pass

**Move mlibc from Phase 9 to immediately after the Phase 5 ABI stabilises.** ⊕

The current plan builds a userland in Phase 7 against a bespoke ABI and
introduces mlibc in Phase 9. **That means writing the userland twice.** mlibc
ships `sysdeps/demo`, a five-file ~9.4 KB skeleton. The goal is not to run
GNOME; it is to answer *"can a POSIX libc sit on this ABI?"* while the ABI is
still cheap to change — and to discover which of `openat`, `dup`, `O_CLOEXEC`
and descriptor inheritance across `posix_spawn` the handle model has made
awkward, before 73 call sites depend on the answer.

The evidence for this is the strongest single correlation in the retrospective
data. **Zinnia** — one person, started 2024-01, 604 of 631 commits — reached a
**Wayland/X11 desktop in about 2.5 years** on mlibc, with a sysdep of **13
files, ~45 KB**. Asterinas reached a Linux-compatible userland in ~3 years by
adopting the Linux ABI outright. Against that: ToaruOS, **15 years solo with a
strict no-third-party-dependencies rule, still not self-hosting**; HelenOS,
**20+ years, elegant, and no path to third-party software**; Redox, own libc,
CPython at 10.4 years and self-hosting at 10.75.

> **Conform at the boundary, innovate inside.** That single variable explains
> the timelines better than team size, funding, or language.

It does not weaken the capability ABI — Redox is eleven years in and
*retrofitting* capabilities underneath POSIX, which is strictly harder. It says
the translation layer should arrive early enough to shape the ABI rather than
inherit it.

**Also owed**: `002`'s POSIX section is 19 lines, and the words `dlopen`,
dynamic linking, signals, `termios`, `ioctl` and pthreads appear **zero times in
6,063 lines of docs** — though CPython needs all of them. The single hardest
question to answer late is: **how does a signal interrupt a blocking queue
wait?** Answer it before the queue ABI is fixed.

---

## 6. WASI — the one decision that should change

D-09 makes WASI 0.3 components a first-class app format, with **wasmi** as the
runtime. Both halves of that are now untenable.

**WASI 0.3.0 did ship, on 2026-06-11** ◆ — 29 months after 0.2, with `wasi:io`
deleted and absorbed into the Canonical ABI. So `008`'s note that ratification
is "a fact rather than a projection" is right. Everything else is against us:

- **The Component Model is still a Phase 1 proposal** in the W3C WebAssembly CG
  — the lowest active phase of five ◆, verified against
  `WebAssembly/proposals/README.md`.
- **"Component Model 1.0" is a roadmap article dated 2026-06-08** whose open
  workstreams include *"ABI improvements — transitioning from eager allocation
  … to lazy value handles"* and *"Easier implementation — simplifying the spec
  … to reduce implementation burden."* **The Bytecode Alliance says its own
  spec is too hard to implement and its ABI is going to change.**
- **WASI 0.3 is implemented by Wasmtime and jco only** — and D-09 already
  rejects Wasmtime as tokio/std-coupled and unusable in-kernel. Rust's
  `wasm32-wasip3` target is **Tier 3**; `wasm32-wasip2` is Tier 2.
- **wasmi has no Component Model and no plans for one.** Its maintainer, in
  September 2025: *"an absolutely huge proposal and therefore an enormous
  undertaking for any Wasm runtime to implement … it remains uncertain if a Wasm
  interpreter will ever even profit … the whole project is kinda controversial
  within the Wasm developer community due to its sheer size and complexity."*
  The tracking issue has been open since 2024-01-16 and was removed from the
  planned list in February 2025. WasmEdge — funded, with a full-time team — has
  been implementing it since early 2024 and is still not done.
- **Cost, measured**: the Component Model spec is **~12,200 lines / ~66,000
  words** on top of core Wasm (`CanonicalABI.md` alone is 5,436 lines).
  Wasmtime's implementation is **40,902 lines of Rust — larger than wasmi's
  entire core interpreter.**
- **The ecosystem is ten components.** The canonical curated list, maintained by
  a Bytecode Alliance / Microsoft engineer: three sample HTTP servers and seven
  micro-utilities (fetch a URL, get the weather, generate a QR code).
  Microsoft's registry has 12, all MCP-tool-shaped. `wa.dev`, the public
  component registry, returns **"No packages available."** Everything
  general-purpose — CPython, Ruby, Go — targets `wasm32-wasip1` core modules.
  **There is no component build of a shell, an editor, coreutils, SQLite, or a
  compiler.**
- The Component Model's actual production niche is **a typed plugin ABI for a
  host application** — VS Code, Zed, Grafbase, MSBuild — not an application
  distribution format. **No component-capable runtime is an interpreter.**

The steelman is real and worth keeping: WASI's capability model maps onto our
handles with no impedance mismatch (a WASI descriptor *is* a handle grant), and
components survive an AArch64 or RISC-V port. **But both of those are delivered
by core Wasm plus `wasip1`**, which is a fraction of the work and where all the
actual software is. The Component Model buys cross-language typed *composition*
— an ecosystem feature that pays off when there are components to compose.

**Restate D-09**: core Wasm + `wasip1` on wasmi, first-class. The Component
Model becomes a watch item with a named re-evaluation trigger — *it reaches
Phase 4, or an interpreter ships support*. Keep the capability-mapping
rationale; drop the components-first-class claim; and drop the wasmi 1.0 pin,
which `008` §4.5 already asked for and which is now doubly stale (wasmi is at
2.0.0-beta).

One ecosystem correction, so nobody repeats a wrong story: **Fermyon was
acquired by Akamai (2025-12-01)** and Spin remains a CNCF project; Cosmonic is
small but shipping. The real 2026 event is that **WAMR left the Bytecode
Alliance (2026-07-21)** — the small-footprint interpreter, the class a hobby OS
would embed, decoupling from the organisation that owns the Component Model.

---

## 7. WizFS — the design holds; the schedule does not

Every rule `003` adopted for WizFS is **confirmed** against primary sources ◆,
with two exceptions and one omission that matters more than either.

### The omission

`003`'s rules describe a CoW B-tree. bcachefs's own Principles of Operation
says: *"unlike other COW btrees, btree updates are journalled. This greatly
improves our write efficiency on random update workloads."*

**bcachefs is not a pure CoW B-tree — it is a journalled B-tree over CoW bucket
storage.** That single decision buys prefix consistency, removes any need for a
btrfs-style log tree, makes small random metadata updates affordable, and makes
crash testing tractable. It should be the spine of the design, and it is not in
our rules.

### The two corrections

**The blind-upsert ABI rule is refuted by its own source.** `003` says POSIX
preconditions force read-before-write, so our non-POSIX ABI should expose blind
upserts. BetrFS (FAST '15) states the precondition problem and then dissolves
it: *"In our experiments, the OS cache of file and directory information was
able to answer these queries, enabling file creation etc., to run at the full
speed of upserts."* The underlying technique — no read-modify-write, a sub-block
write is a self-contained record — is sound and **ABI-independent**. Keep the
technique; **strike the ABI rule**, which solves a problem the primary source
measured as already solved.

**The parity-RAID reason is wrong, though the conclusion survives.** ZFS's
RAID-Z *eliminates* the write hole via variable stripe width; it is btrfs's
RAID56 and classic RAID5 that carry it — btrfs's own docs still mark RAID56
**unstable** with *"write journal is not implemented"* in 2026, 17 years in. We
conflated them. The correct reason to skip parity RAID is that **every correct
solution needs buckets, copying GC and backpointers first.**

Also worth adjusting: full-path keying's rename cost was real in BetrFS 0.1, but
FAST '18's answer was to *keep* full-path indexing and add a range-rename
primitive. Choose inode indirection because range-rename is a research
contribution we do not want to invent — not because full-path keying is wrong.

### The schedule is the problem ⊕

Two angles reached this independently from different evidence.

- **bcachefs: announced "more or less feature complete" 2015-08-21; experimental
  label dropped 2026-06-19. Ten years and ten months**, one exceptional
  full-time funded engineer. In between: merged to Linux 6.7; Torvalds in April
  2024, *"if you thought bcachefs was stable already, I have a bridge to sell
  you"*; **all 117,000 lines removed from Linux 6.18 on 2025-09-30**. It ships
  as an out-of-tree DKMS module today.
- **btrfs: mainline 2009, on-disk format stable November 2013** — and Red Hat
  *removed* it from RHEL 8 in 2019, ten years after mainline.
- **ZFS: ~5 years and a team growing past twelve paid senior engineers**, inside
  a company that owned the OS, the hardware and the QA lab.

And the empirical rebuttal to "CoW is easier to make crash-consistent" —
OSDI '18 ◆: *"btrfs maintains its metadata … in the form of various
copy-on-write B+ trees. **This makes achieving crash consistency tricky, as the
updates have to be propagated to several trees.**"* Of 26 unique
crash-consistency bugs studied, **btrfs accounted for 24**. One went **seven
years** undiscovered.

The honest split: **CoW makes the crash-consistency *argument* simpler and the
*implementation* harder.** ZFS's "no fsck" rests on a single tree root and an
atomic uberblock swap; the elegance stops once there are several interdependent
trees. And you still need a log — ZFS has the ZIL, because `fsync` must be
durable before the next transaction group commits.

For calibration: SerenityOS ships **ext2 and FAT**; Haiku ships a *journaling*
filesystem that came from Be Inc. with paid engineers; Redox has been committing
to `redoxfs` since 2016 — ten years on something simpler than WizFS.

### Recommendation: demote WizFS from a phase to a destination, and stage it

Run Phases 7–11 on something boring, and give WizFS its own later phase whose
definition of done includes crash injection and a repair tool. Within that:

- **6a — a read-only FAT32 or ext2 driver.** This makes virtio-blk, the VFS and
  the buffer plumbing honest at **zero on-disk-write risk**, and a real image
  the project did not generate is an excellent independent parser test. FAT32's
  real argument is *the oracle already exists*: mount the image on Windows,
  Linux and macOS and let three mature implementations judge the writer. For a
  project whose rule is "every change lands with the check that proves it," that
  is categorically stronger than anything buildable for WizFS in one phase.
- **6b — the crash harness, before WizFS has a format** (see §9).
- **6c — WizFS v0**: one btree, 4K nodes, journalled updates, no snapshots, no
  checksums, no GC, and *"simply fails when it runs out of blocks"*. But the v0
  *format* carries the expensive-to-retrofit bits.
- **Free, do it now**: put the `snapshot` field in the key from v0, always zero.
  Adding a key field later is a format break; reserving four bytes is not.
- **Not yet**: 128–256K log-structured nodes (they cost bset merge-sorting,
  whiteout deletion, packed keys and auxiliary search trees, and bcachefs
  justifies them by petabyte scale on spinning disks); and no copying collector
  in v0, which is four subsystems on its own.

Two hazards to record now. **Free-space deadlock** — btrfs's docs, verbatim:
*"The balance operation needs enough work space … otherwise this may lead to
ENOSPC reports."* bcachefs's answer is a **seven-level watermark ladder** plus a
reserve normal writes cannot touch; that is pure arithmetic and belongs in a
`libs/` crate, the `port_space`/`frame_space` pattern again. And **devices that
lie about flushes**, which bcachefs calls *"historically a major (potentially
disastrous) problem for COW filesystems"* — answered by journal scrub at mount
plus a bounded journal rewind.

Finally, a piece of good news: bcachefs's Principles of Operation is now a
current, free, ~117-page architecture document (revision 1.39.2, dated
**2026-08-22**) for exactly the system WizFS wants to be. It did not exist in
this form when our rules were written. **Read it end to end before designing.**

---

## 8. Performance — the axis never researched

`000` promises "fast" and no prior pass examined it. The most useful output is
not a list of optimisations but an ordering of which decisions foreclose speed.

**Where the time actually goes in 2026.** A null syscall costs **78–649 ns**
across a range of Intel hardware, while the **vDSO path is 13–24 ns** — 5–30×
cheaper. That is the single most important structural fact: *a read-mostly
kernel value should be mapped, not called for.* Full mitigations cost **+33%**
on a syscall-bound server; KPTI's cost scales with syscall rate (<0.5% below
10k/s/CPU, ~5% at 75k) and PCID cut it from 2.1% to 0.5%. seL4's IPC goes
**468 → 687 cycles (+47%)** with mitigations — the cleanest isolation of the
cost on a hot path. Notably, **Zen 5 gains essentially nothing from
`mitigations=off`**, the first AMD architecture natively immune; but the list
keeps growing, with 18 hardware-vulnerability documents in the kernel today.

**Context switch and extended state.** Direct cost is 1,000–2,000 cycles; total
including cache and TLB damage is **10k–1M**. **AMX `TILE_DATA` is 8 KiB today
and up to 64 KiB architecturally.** Linux's answer is **XFD** — first-touch
`#NM`, then allocate — and emphatically **not lazy FPU restore**, which was
killed as a security vulnerability (CVE-2018-3665). Since D-29 defers XSAVE to
"when there is a scheduler" and Phase 5 brings ring 3, the concrete
recommendation is: **compute the compacted XSAVE size at boot, use
`XSAVES`/`XRSTORS`, gate with XFD, never lazy.** The size leaks into
stack-guard arithmetic, so it is a Phase 5 decision rather than a later one.

**Data path.** A shared-memory SPSC ring versus a Unix socket at 32 bytes:
**270 ns versus 5,910 ns** — because "the data path contains zero system
calls". This is the quantitative case for D-06. And the io_uring study's
attribution again: **naive adoption 1.06×, restructured caller 33×**, with a
silent `io_worker` fallback costing **7.3 µs** — hence *no silent worker
fallback; make the completion state explicit.*

**Memory system, and a confirmation.** **Direct-map fragmentation does not
matter for data** — measured within noise; only kernel *code* huge mappings
matter. **D-45's 2 MiB direct map is confirmed correct.** Keep it unsplittable,
carve pre-split regions at boot for permission-changing allocations, and export
a counter for 4 KiB splits so the invariant is observable. For scale: ~20% of
Meta's datacentre cycles stall on TLB misses.

**Filesystem and network, at the level of decisions.** **`fsync` spans 1.6 µs
(enterprise, power-loss-protected) to 2,974 µs (consumer)** — three orders of
magnitude, and the on-disk format decides how many flushes a durable operation
costs. For networking, **`MSG_ZEROCOPY` gives 39% on netperf but only 5–8% on
real workloads, while zero-copy *receive* gives 1.85×** — send is overrated, rx
is the prize. smoltcp does 3.7–7.9 Gbps and its real limits are **no SACK, no
jumbo frames, single-threaded poll** — not copying. Worth knowing before D-20's
plan meets a real NIC.

**Rust's own cost**, since D-01 rests on it: **under 5% typically, 6–11% more
cycles per packet** in a network driver, concentrated in bounds checks.

**Measurement, which the project has none of.** Nothing above is checkable on
the current dev loop, because QEMU/TCG timings are meaningless. The answer is to
**measure counts, not times**: allocations per operation, lock acquisitions per
operation, traps per operation, and boot instruction count under `-icount`.
Counts are the only thing emulation reports truthfully, and five of the six
cheapest regressions to catch are counts. Instruction-count benchmarking of the
`libs/` crates would fit `cargo xtask ci` exactly — **but see §9: the tooling
for it may hit the same host-toolchain wall.** The durable recommendation is
therefore: **acquire a netboot-able bare-metal machine, ideally one AMD and one
Intel**, because Zen 5 and Intel no longer predict each other.

**The ranked list of what must be decided before Phase 5 closes**, which is the
practical output:

1. **How many ring transitions per unit of work** — this *is* the ABI.
2. **The buffer-ownership type** — it decides channels, I/O and networking
   simultaneously. Not zero-copy itself: the *type*. Pool provenance, DMA
   mapping, lifetime, `Drop` returning to the pool.
3. **Page- versus folio-oriented memory** — `(order, base-PFN)` runs with tagged
   descriptors. Linux is spending years retrofitting this.
4. **Per-CPU data and the interrupt-state token** — free on one CPU, expensive
   later.
5. **Extended state** — it sizes the TCB.
6. **Entry-path architecture** — keep it patchable per attack vector and
   FRED-shaped.
7. **A measurement capability.**

If only three fit: **the buffer type, the async ABI shape, and the measurement
loop.**

---

## 9. Verification — cheaper and closer than the project assumed

This section was written twice. The first version reported that the tooling was
unreachable on this machine, because I tried the obvious thing and it failed.
That conclusion was wrong, and the way it was wrong is worth keeping: **the
wall is real, and it is passable with two version pins.**

### The wall, and the way through ✓

`loom`, `shuttle` and `proptest` all fail to build here at their default
versions. The root cause, diagnosed precisely: rustup's GNU toolchain ships a
`self-contained` directory holding `dlltool.exe`, `ld.exe`, a mingw gcc and
`libwinpthread-1.dll` — and **there is no `as.exe` anywhere on the machine**.
Put `self-contained` on `PATH` and `dlltool` gets one step further before dying
with `CreateProcess`, because it cannot spawn the assembler it needs.

The threshold is specific: **any crate that reaches `getrandom >= 0.3.2` or
`windows-sys >= 0.61`** — the versions that adopted raw-dylib — fails. Below
those versions everything builds. Measured:

| Crate | Default | With pins |
|---|---|---|
| `proptest` 1 | fails | **works** ✓ — `default-features = false` + `getrandom = "=0.3.1"` |
| `shuttle` 0.9 | **works** ✓ — no pins needed | — |
| `loom` 0.7.2 | fails | works with two lockfile pins |
| `cargo-mutants` | fails | still blocked |
| `cargo-fuzz` | Unix / MSVC only | out |

The durable fix is a **project-local MinGW binutils drop in `tools/`**, which
clears all of it and keeps D-16's self-contained posture — it is vendored
tooling, the same category as `boot/limine/`, not a system install. That is the
highest value-per-minute item in this document.

`Cargo.toml` already documents the symptom for one crate — *"fatfs drops
default features to avoid chrono, whose Windows bindings need dlltool.exe"* —
and treats it as a one-off. **It is a standing tax on every future host-side
dependency**, and it should be recorded as such.

### Verus runs here, on stable, today ✓ — and the recorded cost was pessimistic

This is the finding that changes the most.

`008` §5 recorded Atmosphere's proof-to-code ratio as **7.5:1**. The SOSP'25
paper says **3.32:1** — 20,098 proof lines over 6,048 executable lines, with
*"verification in less than 20 seconds on a modern laptop."* The 7.5:1 number
comes from the earlier KISV'23 workshop paper, and the project page still
repeats it. **Our own note understates Verus by 2.26×.** For context: seL4
20:1, CertiKOS 14.9:1, NrOS 10:1, SeKVM 6.9:1, VeriSMo 2:1.

Two asterisks belong with it, though: Atmosphere took **three clean-slate
rewrites**, and leaves a ~3,000-line trusted Rust TCB plus 172 lines of assembly
and the bootloader unverified.

And the decisive practical question — *does any of this run on a stable,
Windows-GNU, no-nightly machine?* — is answered **yes, measured**: the same
crate, carrying `#![cfg_attr(not(test), no_std)]` and `#![forbid(unsafe_code)]`,
went green under both `cargo test` on `1.97.1-x86_64-pc-windows-gnu` and
`verus.exe --crate-type=lib` (6 verified, 0 errors). **Verus pins stable
1.97.1 — this project's exact pinned version.** No nightly, no WSL.

The other two recorded claims check out. **Asterinas × CertiK is correct
verbatim** — *"a race condition that might have allowed a page table node to be
freed prematurely"* — and the follow-on CortenMM won SOSP'25 Best Paper. **The
TASE 2026 buddy-allocator result is correct on every element.**

The measured learning curve is the useful part: four failed iterations before a
proof went through, because spec arithmetic promotes to `int`, bitmasks need
`by(bit_vector)`, and **multiplication and division need
`by(nonlinear_arith)`** — which is *exactly* the shape of EEVDF's weighting and
of ring-index arithmetic. Roughly **15 proof lines per 4 executable lines**,
matching Atmosphere's page-table ratio of 4.3:1.

**Kani, by contrast, is out**: Linux and macOS only, with zero Windows
artifacts. It was `008`'s proposal #10 and the one overdue item — so the reason
it never landed turns out to be better than "not done".

### What the tools actually catch, measured

`shuttle` 0.9 builds cleanly here, and I ran a deliberately-broken SPSC handoff
through it — a `Relaxed` publish where `Release` is required, 2,000 iterations.
**It passed.** That is not a defect; it is the tool's design. Shuttle is a
*scheduler-interleaving* checker and does not model weak memory, so it cannot
see an ordering bug at all. The SMP angle listed "whether Shuttle models weak
memory" as genuinely open; this answers it. **Loom, on the same shape, reports
`FAILED — stale payload observed`.**

So the two are complements, not alternatives: **shuttle for interleavings and
deadlock, loom for memory ordering.** And loom's own blind spots still belong in
the ADR, because D-28's argument cuts both ways — a check that cannot fail is
also a claim:

| Break | loom |
|---|---|
| `Release` → `Relaxed` on the publish store | **red** ✓ |
| drop the `Acquire` on the index load | **red** |
| publish the index before the data | **red** |
| `SeqCst` → `AcqRel` on a doorbell fence | **green — false pass** (loom models SeqCst as acquire-release) |
| relaxed intra-thread reordering | **green — false pass** |

The doorbell protocol — the store-then-load shape §4 identifies as the one bug
class a single CPU cannot show — **must therefore be reasoned about by hand, not
merely tested.**

The deeper reason to do this in Phase 5 rather than at the SMP phase:
**loom's two threads over-approximate a uniprocessor kernel with interrupts**,
since an interrupt is preemption at an arbitrary point — a subset of the
interleavings loom explores. A loom-clean primitive is correct on UP today *and*
pre-validated for SMP later, which is a direct answer to §4's "every lock is an
untested claim".

### TLA+ is the cheapest thing here, and it runs before any Rust exists

Measured: a 39-line TLA+ specification of a ring buffer checks in **under a
second** (35 states). Delete the producer's free-slot check and TLC reports
`Invariant NoOverrun is violated` in **0.72 s**, with a full counterexample
trace. JDK 21 is already present on this machine, so the cost is one vendored
JAR.

**Model the Phase 5 ring protocol before writing it**, and the Phase 6
crash-consistency protocol before freezing the on-disk format. Ten minutes to
write, sub-second to check.

### The crash harness for Phase 6

**Every academic filesystem crash-testing tool is abandoned** — CrashMonkey,
Hydra, FSCQ and Yggdrasil were all last touched in 2022, and all test a
*mounted kernel filesystem on a real device*. None can be pointed at a `no_std`
crate. The ideas transfer; the harness must be built, and it is small, ordinary,
host-side Rust.

The most actionable empirical fact available (OSDI '18): *"most reported bugs
can be reproduced using small workloads of three or fewer file-system operations
on a newly-created file system, and all reported bugs result from crashes after
fsync-related system calls."*

Staged, each stage a plausible commit:

0. **Write the persistence contract as an ADR** — prefix consistency
   recommended. Without it there is no oracle.
1. **`libs/blockdev-sim`** — a seeded deterministic device with a volatile
   cache and barrier semantics, yielding every crash prefix. ~400 lines.
2. **A reference model and `allowed_states()`** — ~300 lines.
3. **`fsck` as a library function** returning a structured report — a second,
   independent oracle.
4. **Exhaustive one- and two-operation sequences**, crash at persistence points,
   recover, fsck, compare against the model.
5. **Crash at *every* prefix, sampled by seed.** This is what tests a CoW core,
   and it closes the gap the OSDI harness explicitly leaves: it *"does not
   re-order IO requests… the implicit assumption is that the core
   crash-consistency mechanism… is working correctly."* **That assumption is
   exactly false when the CoW core is the new code.**
6. **`arbitrary` + a committed corpus** for `parse_node(&[u8])` — parser
   hardening, and `arbitrary` builds cleanly here ✓.

**The D-28 demonstration writes itself**: delete the flush between the data
write and the journal commit. **Stage 4 still passes; stage 5 goes red.** That
single experiment justifies stage 5's existence — which is the argument D-28 was
written to make.

Note what this validates. Stages 1, 2, 4, 5 and 6 are *all* host-testable
`libs/` work. **The most valuable property of putting the on-disk format in a
`no_std`, host-buildable crate is that it makes deterministic crash simulation
possible at all** — a stronger justification for the existing rule than it was
originally given.

### Where the line is

**Full functional verification: declined.** FSCQ was **31,397 lines of code and
proof over 1.5 years** for a filesystem *simpler* than xv6's 3,000-line C one —
no CoW B-tree, no snapshots, no checksums — and Hydra later found a bug in it.
BilbyFS: **9.25 person-months and 13,000 proof lines for two operations.**

But **targeted Verus is now a real option**, and the structure that makes it
cheap is already in place. Two things follow. First, **the specification, not
the proof, becomes the dominant cost at data-structure scale** — Asterinas's
page-table work is ~6,000 spec lines against ~2,000 proof lines — so budget
accordingly, and remember a proof against a wrong specification is a confident
wrong answer. Second, and more consequential:

> **Design-for-verifiability is a Phase 5 decision, not a Phase 7 one.** NrOS
> paid 13.3:1 and Atmosphere 4.3:1 *for the same page-table logic*. The
> difference was a flat, pointer-centric permission layout versus Rust recursive
> ownership.

That choice must be made before the ring and the handle table are written, not
after.

Two further items worth naming rather than adopting: **`cargo-mutants`
mechanises D-28** — it would have caught the Phase 3 slice-1 incident where a
check passed with the code it tested deleted — and is blocked only by the
binutils wall; and **SquirrelFS** uses typestate for crash ordering, which fits
this project's aesthetic better than any external tool and is worth reading
before Phase 6.

---
## 10. Platform — what to assume, and what not to foreclose

**x86S is cancelled.** Intel confirmed in December 2024 that it *"pivoted away
from the x86S initiative"*, folding the effort into the Intel+AMD x86 Ecosystem
Advisory Group. Real mode, rings 1 and 2, and the 8259 are architecturally
permanent. The practical consequence here is nearly nil — UEFI already hides the
boot sequence — with one standing obligation: **keep actively masking the 8259
via the MADT's `PCAT_COMPAT` flag, never assume it is absent.** The advisory
group is a *convergence* body rather than a slimming one, and its first-year
output is worth knowing: **FRED** finalised and shipping, **AVX10** finalised,
**ChkTag** (unified Intel+AMD memory tagging) specified but unpublished, and
**ACE** as AMX's successor.

**Baseline.** The honest framing for an ADR is not a `vN` label — those are
userspace compiler targets, and this kernel is `+soft-float`. **The real floor
is "the oldest CPU with SMAP and UMIP", which is Skylake-era 2015.** Practically
that means x86-64-v2 plus XSAVE/OSXSAVE, x2APIC, TSC-deadline, an invariant-TSC
check, and SMEP/SMAP/UMIP. UEFI-only with no CSM remains correct and costs
nothing.

**D-33's x2APIC-only decision holds**, and the reasoning is better than it was:
MSR access is a genuinely smaller `unsafe` surface than an MMIO APIC window. Two
changes are owed. It should become a **checked, diagnosable boot precondition
rather than an assumption** — and that is a real D-28 negative test, since
`-cpu qemu64,-x2apic` can produce a machine without it. And I could not find a
Broadcom document stating plainly that VMware Workstation exposes x2APIC;
empirically it does, but the check turns a triple fault into a message.

### Security features, and when each must be shaped

| Feature | Hardware 2026 | Linux | Shape before Phase 5? |
|---|---|---|---|
| **FRED** | Panther Lake, shipped 2026-01-27 | default-on in 7.1 | **Yes — the big one** |
| CET user shadow stack | Tiger Lake / Zen 3, 2020 | 6.6 | **Yes** — a thread becomes *two* stacks, and W^X needs a fourth permission class |
| CET kernel IBT | same | 5.18 | **Partly** — every indirect target in `global_asm!` needs `ENDBR64`, or the naked switch breaks when it is enabled |
| CET *supervisor* shadow stack | Tiger Lake+ | **not merged as of Aug 2026** | No |
| LASS | Sierra Forest 2024, Intel only | 6.19 → 7.1 | **Discipline only** — the split is exactly bit 63, and `#GP` becomes the null-deref reporter |
| SMAP | universal | on | **Yes** — `stac`/`clac` must be a scoped RAII guard; it *is* the user-copy ABI |
| PKS | Sapphire Rapids, Intel only | **never merged** | **Reserve PTE bits 62:59**, build nothing |
| LAM | Intel only | 6.4 | **No** — being superseded by ChkTag |
| ChkTag | none yet | none | Watch. One rule: **range-check user pointers, never `>> 47 == 0`** |
| eIBRS | Ice Lake+ | on | **Yes — one MSR write at boot** |
| `VERW` on exit-to-user | universal | on | **Yes, day one** — one instruction, painful to retrofit into a mature return path |
| KPTI | — | — | **No** — decide against it and record `RDCL_NO` |

Three items postdate the last pass: **ITS** (Indirect Target Selection, 2025),
**VMSCAPE** (CVE-2025-40300, Sept 2025), and Linux's **Attack Vector Controls**
framework (6.16), whose *shape* is worth stealing even if the mitigations are
not.

**FRED is the finding that matters, and it is closer than the report assumed.**
It replaces IDT delivery and `IRET` with `ERETS`/`ERETU`, replaces IST with four
reentrant stack levels, makes `SWAPGS` a `#UD` in favour of `LKGS`, and delivers
a *uniform* frame regardless of event — and it is the mechanism that finally
makes supervisor shadow stacks tractable.

The research could not establish whether QEMU emulates it. **I checked: it
does, at least in part.** ✓ Our QEMU 11.0.50 lists `fred`, `lkgs` and `wrmsrns`
among its known CPU features and accepts `-cpu max,fred=on` without complaint.
That does not prove the transitions are fully emulated — that needs a boot test
— but it moves FRED from "unverifiable" to "testable here, today".

The recommendation is unchanged in shape and firmer in justification: **design
Phase 5's trap entry FRED-shaped without implementing FRED.** Concretely —
`swapgs` behind an `arch::percpu` accessor; `iretq` behind
`arch::trap::return_to_user`; trap stacks as an enum mapped onto IST today and
FRED levels later; and above all **dispatch from a table on a decoded
`TrapCause`, rather than 256 IDT stubs that each know their own vector.** The
decode is pure arithmetic, so by this project's own rule it goes to `libs/` and
is host-tested — and it is also exactly what an aarch64 port would need, which
is a strong signal it is the right abstraction rather than speculative
generality.

### The XSAVE decision Phase 5 forces

D-29 deferred XSAVE until there was a scheduler; ring 3 arrives in Phase 5, so
the decision is now due. **Eager, compacted, boot-sized, out-of-line,
CPUID-derived, with the arithmetic in `libs/`.**

- **Eager only.** Lazy FPU restore is CVE-2018-3665; Linux went eager in 2016.
  Record it in the ADR so nobody re-optimises it back in.
- **`XSAVES` → `XSAVEC` → `XSAVE`, and never `XSAVEOPT`** — its
  modified-optimisation is a correctness hazard, while the init-optimisation
  that `XSAVEC`/`XSAVES` provide is the one that pays. Prefer `XSAVES` even with
  no supervisor state today, because it shares the compacted format with
  `XSAVEC` — and CET's shadow-stack pointers live in supervisor xstate, so
  wanting CET later means wanting `XSAVES` now.
- **`XCR0` = x87 + SSE + AVX only.** No AVX-512, no AMX. Every enabled bit is
  per-thread memory forever, and **AMX's `XTILEDATA` alone is 8 KiB.**
- **`XstateBuf` out-of-line and growable**, 64-byte aligned, sized at boot from
  the mask — never a fixed array inside `Thread`. That is precisely what would
  make AMX or ACE impossible to add later.
- **Keep `+soft-float`, and add a CI check that it stays.** It is what makes
  trap entry never touch xstate; only a task switch does.
- **Reserve `#NM` (vector 7)** for a future XFD-based allocator. Linux's
  dynamic-XSTATE model is a *first-use allocation trap*, not lazy restore — the
  distinction matters, because one is a security bug and the other is not.
- **The trap to design around:** APX is state component 19 but sits at **offset
  960** in the non-compacted format, in deprecated MPX's hole — a higher index
  at a lower offset. Linux had to add a dynamic ordering table and delete its
  monotonicity assertions in 6.16. **Compute offsets from `CPUID.0DH` into a
  table in a `libs/` crate, host-tested against synthetic CPUID snapshots
  including the APX case**; `wost` keeps only the four instructions and the MSR
  writes. The `port_space`/`frame_space` pattern once more. AVX10 itself changes
  nothing, since it reuses the existing components.

### Virtualisation — and a correction to the driver plan

**virtio is not the right first device family.** VMware Workstation — the
milestone gate under D-18 — **does not implement virtio at all**; it offers
VMXNET3 and PVSCSI. The common denominator across QEMU, VMware and real
hardware is **NVMe** for storage, **`RDSEED`** for entropy (which retires
virtio-rng entirely), and the UART plus the Limine framebuffer for console.
Build virtio-blk only if it is wanted as a teaching step, not as the path to a
VMware boot.

Two Limine items, and one is directly useful. Our vendored binary is from the
`v10.x-binary` branch, fetched 2026-07-18, and D-42 requests base revision 4.
**Base revision 5** additionally guarantees the EFI system table stays mapped
and the local APIC is in a defined state on the bootstrap processor and every
AP — free code we would otherwise write, and `is_supported()` already fails
loudly if the binary cannot honour it. Separately, **Limine 11.3.1 (April 2026)
added a TSC Frequency request**, which would retire the calibration problem
Phase 4 slice 3 hit and the *"counter is not advertised invariant"* warning
still printed on every boot. That one needs a vendored-binary update and a
fresh checksum under D-25, so it is a deliberate change rather than a free one.

Finally, **the clocksource should be an abstraction with at least two
implementations chosen at boot.** TCG will keep punishing anything that
hard-wires the TSC, as it already has once.

### Portability

**Stay x86_64-only, and do not port.** The development machine is x86_64, the
milestone gate is x86_64, and the x86 ecosystem is *more* coherent in 2026 than
before. RISC-V is real — RVA23 silicon is arriving — which makes it a sensible
second target around 2028, not now. A second architecture costs roughly
**30–40% of total kernel effort to date**, and most of that is discovering which
"portable" code was secretly x86-specific.

What must exist before any port, all free to shape now and all improvements on
their own terms: `AddressSpace::activate()` rather than `load_cr3`; `PageFlags`
rather than raw PTE bits; `arch::PAGE_SIZES` and `VA_BITS` rather than literals;
an opaque `TrapContext` with a decoded `TrapCause`; `arch::percpu` rather than
`swapgs`; `arch::interrupt` and `arch::timer` rather than APIC and TSC nouns;
`arch::io`, since port I/O does not exist elsewhere; `arch::cache`, which is a
no-op on x86 and mandatory elsewhere; and `SyscallArgs` with named fields, so no
portable code ever reads `rdi`.

The single most-cited first casualty is worth recording now: **aarch64 has two
translation roots (`TTBR0`/`TTBR1`), so a single-value `Cr3` type is the number
one portability trap.** And **be pedantic about atomic `Ordering` immediately**
— x86's TSO makes a mis-specified ordering invisible, and it is the one class of
error that cannot be refactored away later. If a port ever happens, make it
aarch64.

---

## 11. Doc hygiene — what has gone stale

Found by auditing the docs against the code and the ADRs. All small, all
cheap, and one of them is a factual error.

**`002-architecture.md`:**

1. **Pillar 1 says channels are "the sole handle-delegation mechanism."**
   Factually wrong (§3). Replace with the checkable spawn-invariant.
2. **Open question — "Kernel-memory accounting: is seL4-style untyped-retype
   … worth adopting? (taken up in `007-memory.md` — Phase 3 must answer it)."**
   **Closed by D-44.** Phase 3 answered it; the doc still asks.
3. **Open question — "CHERI/CET/PKS hardware security pass."** **Closed by
   `008` §6.**
4. **Open question — "re-verify Tier-2 claims" and "framekernel SMP scalability
   is unquantified."** Closed by this document, in both cases with answers that
   change something.
5. **Open question — "weigh Capsicum's ~60 per-method rights vs Zircon's
   smaller set."** Answered (§3); also, Capsicum's count is now ~81.
6. **"Runtime: wasmi 1.0"** — `008` §4.5 asked for this to be softened and it
   was not actioned. Now doubly stale: wasmi is at 2.0.0-beta, and §6 changes
   the surrounding decision anyway.
7. **The security-baseline section is headed "on from the start, not
   retrofitted" and lists KASLR**, which D-46 deferred to Phase 12. The heading
   is now false as written. `000-vision.md`'s Core decisions table repeats it.

**Outstanding from `008` §8:** of sixteen proposed actions, thirteen landed
inside one phase — a good record. Two remain. **#15, the wasmi pin**, is
genuinely overdue and §6 supersedes it anyway. **#10, Kani harnesses in the
page-table crate**, turns out to have been unactionable all along: Kani does not
run on Windows (§9). Verus does, and is the better fit — so the proposal should
be replaced rather than chased. The third, #16 (the queue-austerity checklist),
is pending rather than overdue, because the Phase 5 queue doc does not exist
yet.

**One correction to `008` itself**, recorded here so the number does not
propagate further: §5's Atmosphere proof-to-code ratio of **7.5:1 is wrong**.
The SOSP'25 paper says **3.32:1**; 7.5:1 comes from the earlier KISV'23
workshop paper, and the project page still repeats it (§9).

**A structural suggestion, not a correction.** `008` §7 records the Asterinas
result as author-measured and unreplicated, while **D-13 carries "Status:
Accepted"** with no such qualifier. The decision log has two statuses; a third
would carry this honestly:

> **`Provisional`** — accepted and built on, but resting on evidence that has
> not been independently confirmed. Carries a **falsification condition**: what
> would have to be observed for this to be revisited.

D-13 is the obvious first holder. No foundational ADR currently states what
would change its mind, and after this pass three of them could: D-13
(replication), D-09 (Component Model reaching Phase 4, or an interpreter
shipping support), and D-08 (whether the crash harness of §9 can actually be
built before the format is).

---

## 12. Proposed actions

Proposals, not decisions — the same convention `008` used. Each becomes an ADR
only if taken. Ordered by when the door closes.

### Highest value per minute

1. **Vendor MinGW binutils into `tools/`** (§9). It clears the missing-`as.exe`
   wall that gates `loom`, `cargo-mutants` and every future host-side crate
   reaching `getrandom >= 0.3.2` or `windows-sys >= 0.61`. It is vendored
   tooling — the same category as `boot/limine/`, checksum-pinned under D-25 —
   so it *keeps* D-16's self-contained posture rather than breaking it. Until
   then, `proptest` works with `getrandom = "=0.3.1"` ✓ and `shuttle` works
   unpinned ✓.

### Free now, and verified to work ✓

2. **Add `unsafe_code = "deny"` to `[workspace.lints.rust]`**, with `wost`
   opting out in source. Tested both directions: with it, an unsafe block in a
   crate that forgot `forbid` is an error; without it, that code compiles
   silently. Closes the gap for every crate added from here on.
3. **Make `SpinLockGuard` and `ThreadLockGuard` `!Send`** with
   `PhantomData<*const ()>`. Verified they are `Send` today. One line each; the
   idiom is already used by `ThreadContext`.
4. **Fix the seven doc-hygiene items** in §11.

### Before Phase 5 writes code

5. **Make `SpinLock::lock` borrow the `ThreadContext`** ✓ — verified on the
   pinned toolchain that this makes "block while holding a spin lock" and "take
   two spin locks" into compile errors, while the legal ordering still
   compiles. Two caveats found by testing: it **depends on the guard having a
   `Drop` impl** (without one, NLL ends the borrow early and both violations
   compile silently — so the break-test is *delete the `Drop`*), and handlers
   need a separate `lock_from_trap()` because `ThreadContext::claim()` returns
   `None` in trap context by design. 13 statics, 73 call sites.
6. **Add a preempt-disabled-interrupts-on lock** ✓ — verified there is no
   preempt-disable mechanism at all today, so `SpinLock`'s `cli` is doing that
   job. This is OSTD's *default* guardian and much cheaper than `cli`/`sti`.
7. **The handle table** (§3): 16–24 rights in one `u32`; the "a right is a
   distinction the kernel makes, never a syscall name" rule; a
   `libs/handle-space` crate for the encoding; **no lowest-free promise**; a
   refcounted table rather than an inline field; attenuate-on-send; **no
   revocation**, with a per-object epoch named as the future mechanism.
8. **The queue ABI** (§2): exactly one terminal completion per operation;
   kernel-owned buffers; **reservation-based backpressure**, not Linux's;
   read-once-then-validate enforced by API shape; single-owner rings; **no
   global FIFO promise across submitters**; kernel-allocated ring pages.
9. **Write the SMP-discipline ADRs** (§4), which cost no code: the big-kernel-
   lock target model; never conflate interrupt-disable with preempt-disable;
   no lock guard across a context switch (naming the SMP reason alongside
   D-66's); `Relaxed` justified like `unsafe`; lock ordering as a type-level
   `libs/` crate; **split `THREADS` into a thread table and a run queue.**
10. **Decide the extended-state design** (§8): compacted `XSAVES`/`XRSTORS`,
    size computed at boot, XFD-gated, never lazy.
11. **Answer "how does a signal interrupt a blocking queue wait?"** (§5) before
    the queue ABI is fixed.
12. **Shape the trap entry FRED-first** (§10): an opaque `TrapContext`, a
    decoded `TrapCause` dispatched from a table rather than 256 vector-aware
    stubs, `swapgs` behind `arch::percpu`, `iretq` behind
    `arch::trap::return_to_user`, and trap stacks as an enum. The decode is
    arithmetic, so it goes to `libs/`. **FRED is testable on this machine
    today** ✓ — QEMU 11.0.50 knows `fred`, `lkgs` and `wrmsrns` and accepts
    `fred=on` — so whether the transitions are fully emulated is a boot test,
    not a literature question.
13. **The thread stack allocator needs a second region slot** (CET shadow
    stack) and **W^X a fourth permission class**, both before the allocator is
    fixed (§10).
14. **Two one-liners in the Phase 5 return path from day one**: `VERW` gated on
    `MD_CLEAR`, and an eIBRS MSR write at boot. Both are near-impossible to
    retrofit gracefully.
15. **Reserve PTE bits 62:59** and **range-check user pointers rather than
    testing `>> 47 == 0`** — two rules that cost nothing and preserve
    PKS/LAM/ChkTag optionality.
16. **Make x2APIC a checked, diagnosable boot precondition** rather than an
    assumption, with `-cpu qemu64,-x2apic` as its D-28 negative test.

### Sequencing changes

17. **Move mlibc to immediately after the Phase 5 ABI stabilises** (§5). The
    largest recommendation in this pass; the current plan writes the userland
    twice.
18. **Restate D-09** as core Wasm + `wasip1` on wasmi, with the Component Model
    a watch item carrying a named trigger (§6).
19. **Demote WizFS from a phase to a destination** (§7) ⊕: read-only FAT32 or
    ext2 first, then the crash harness, then a minimal journalled v0 — with the
    `snapshot` key field reserved from v0 because adding it later is a format
    break.
20. **Reorder the driver plan: NVMe before virtio** (§10). VMware Workstation —
    the D-18 milestone gate — does not implement virtio at all. NVMe plus
    `RDSEED` plus the UART and Limine framebuffer is the common denominator
    across QEMU, VMware and real hardware.
21. **Rename the Phase 11 milestone honestly.** NetSurf's last release is
    3.11 (2023-12-28); its own docs say JavaScript is *"primitive and
    incomplete… disabled by default"*; `UnimplementedJavascript.md` lists 1,539
    missing bindings. A new frontend is ~1,000 lines minimal / ~7,000 basic, and
    the framebuffer frontend needs no toolkit — so it is a *good, right-sized*
    integration trophy that proves libc, windowing, networking and TLS. It is
    not "browse the web," and `000` currently promises the latter.
22. **Make "CPython's test suite passes" the acceptance test for Phase 9**
    rather than a feature of it — it exercises threads, signals, subprocesses,
    sockets and the filesystem at once.
23. **Consider updating the vendored Limine** (§10) to request base revision 5,
    and to Limine 11.3.1+ for its TSC Frequency request — which would retire
    the invariant-TSC warning printed on every boot. A deliberate change: it
    needs a fresh checksum under D-25.

### Practices the evidence supports

24. **Track absolute unsafe lines in `wost` against a budget**, never a
    percentage (§1).
25. **Add a `Provisional` ADR status with falsification conditions** (§11),
    starting with D-13, D-09 and D-08.
26. **Measure counts, not times** (§8) — allocations, lock acquisitions and
    traps per operation, and boot instruction count under `-icount`. The only
    numbers emulation reports truthfully.
27. **Add a conformance lane** — os-test and the Open POSIX Test Suite, counts
    recorded over time — as a third check location beside `libs/` and
    `selftest.rs`. Both Redox and managarm converged on this independently, and
    it is D-28 extended past the kernel boundary.
28. **Publish a dated monthly note.** The single most common trait among
    projects that kept moving, and the ADR log already does the harder half.
29. **Acquire a netboot-able bare-metal machine** — ideally one AMD and one
    Intel, since Zen 5 and Intel no longer predict each other on mitigation
    cost (§8).

### Verification, staged (§9)

30. **Vendor `tla2tools.jar` and add `cargo xtask model-check`** — then model
    the Phase 5 ring protocol *before writing it*, and the Phase 6
    crash-consistency protocol before freezing the format. Measured at
    sub-second checking on a 39-line spec; JDK 21 is already present.
31. **`libs/ring-proto`, generic over its atomics** — checked by **shuttle for
    interleavings and loom for memory ordering**, which are complements rather
    than alternatives (measured: shuttle does *not* model weak memory and misses
    a `Relaxed` publish that loom catches). Include a deliberately-`Relaxed`
    variant that must fail, per D-28.
32. **`proptest` on the ring indices and on `libs/eevdf`** — works today with
    `getrandom = "=0.3.1"` ✓.
33. **One Verus spike, on one function** — the slot-mask bound — driven by
    `verus.exe` rather than `cargo-verus`. Verus pins stable 1.97.1, this
    project's exact version, and was measured green on a `no_std`,
    `forbid(unsafe_code)` crate here ✓. Expect ~15 proof lines per 4 executable
    lines, and expect `by(nonlinear_arith)` to be needed wherever the
    arithmetic multiplies or divides — which is EEVDF's and ring indexing's
    shape exactly.
34. **Correct `008` §5's Atmosphere figure**: the proof-to-code ratio is
    **3.32:1**, not 7.5:1 — the stale number is from the KISV'23 workshop paper.
    Note also the three clean-slate rewrites and the ~3,000-line trusted TCB
    the headline omits.
35. **Treat design-for-verifiability as a Phase 5 decision.** NrOS paid 13.3:1
    and Atmosphere 4.3:1 for the same page-table logic; the difference was the
    data layout. That choice is made when the ring and handle table are
    written, not afterwards.

### Explicitly declined

36. **Full functional verification** (§9) — FSCQ's 31,397 lines and 1.5 years
    for something simpler than xv6 settles it.
37. **seL4-style revocation** (§3) — a one-second pre-mitigation worst case,
    zombie capabilities, and two documented partial-completion cases.
38. **Writing an AML interpreter** — `rust-osdev/acpi` gained one, and Redox
    rebased onto it in April 2026 where it *fixed real hardware*.
39. **The own-bootloader side-quest**, reconsidered: the one Phase 12 item with
    a maintained alternative and essentially no learning Phases 0–4 have not
    already delivered.
40. **Kani** (§9) — Linux and macOS only, zero Windows artifacts. This was
    `008`'s proposal #10 and its one genuinely overdue item; the reason it
    never landed turns out to be better than "not done". Revisit only if a
    Linux CI runner ever appears, at which point `--autoharness` makes it
    nearly free.
41. **Coverage-guided fuzzing** (`cargo-fuzz`, afl.rs, honggfuzz) — Unix or
    MSVC-only. `arbitrary` plus a committed corpus covers the need here ✓.

---

## Sources

Recorded per-angle in the research working notes rather than reproduced here;
the load-bearing primary sources are cited inline above. The eleven angles were:
async ABI verification; capability model verification; the framekernel and its
SMP question; SMP retrofit cost; performance engineering; an adversarial pass
over every foundational choice; filesystem verification; Rust kernel
concurrency; project retrospectives; formal verification; and platform futures.
