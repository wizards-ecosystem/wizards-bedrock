# 010 — Deep Research III: Verifying the Foundation

> **Historical pre-Phase-5 snapshot.** Numeric measurements, dependency and
> tool versions, and words such as "current" or "today" are observations from
> the production date below. Later correction notes and
> `004-decision-log.md` govern the present repository.

Produced 2026-08-23, at the close of Phase 4 and before Phase 5 fixes the handle
table and the async queue ABI. **This pass is verification-first**: "is what we
already decided true". Eleven research angles (async ABI; capability model; the
framekernel and its SMP question; SMP retrofit cost; performance; an
adversarial pass over every foundational choice; filesystem verification; Rust
kernel concurrency; project retrospectives; formal verification; platform
futures), each told to try to *refute* the project's recorded claims and to
return a verdict per claim, plus a local audit of the docs against the code and
claims compiled and run against the pinned toolchain. Several angles exhausted
their search budgets; where a finding rests on one source or on inference, it
says so. Per-angle sources stayed in the research working notes; the
load-bearing primary sources are cited inline. Marks:

- **✓**: verified locally, by compiling or running it against this repo's
  pinned toolchain (`rustc 1.97.1`) or by reading this repo's own code.
- **◆**: a primary source was fetched directly.
- **⊕**: two independent angles reached it without sight of each other.

The three debts `008` §7 left open (the Tier-2 re-verification, independent
replication of the Asterinas numbers, framekernel SMP scalability) are closed or
answered, two of them not as the project expected.

---

## 0. What held, what moved

| Decision | Verdict |
|---|---|
| D-13 framekernel discipline | **Holds**: the property we rely on is *verified*; the number we quote is *wrong* |
| D-06 async submission/completion queues | **Holds, rationale must be rewritten**; the recorded reason is refuted by its own source |
| D-06 capability handles + rights | **Holds**, and `002`'s open rights question is now answered |
| D-04 monolithic-modular | **Holds**, unchallenged by the evidence |
| D-15 EEVDF, tickless | **Holds** |
| D-45 direct map, 2 MiB | **Holds**, confirmed by measurement |
| D-07 spawn-only | **Holds as a kernel design**; the compatibility bill is unbudgeted |
| D-10 POSIX as a translation layer | **Holds, but the sequencing is wrong** and the surface is undocumented |
| D-08 WizFS CoW B-tree | **Design holds; the schedule does not** ⊕ |
| D-09 WASI 0.3 components first-class | **Should change**: the named runtime cannot run the named standard |
| SMP at Phase 12 | **Bring-up: correct. Design discipline: must move to Phase 5** |
| D-16 self-contained Windows build | **Holds, with a tax to record** ✓: a toolchain gap gates host-side crates; passable with pins, fixed by vendored binutils |

New in this pass: `002` has a factual error about Zircon (§3); our lock guards
are `Send` ✓, latent on one CPU and a hazard on SMP; the borrow checker can
enforce our lock discipline ✓, a compile-time form of the runtime
`SPINLOCKS_HELD` counter; there is no preempt-disable mechanism at all ✓, so
`SpinLock`'s `cli` does that job, the conflation the SMP literature names as its
most-cited breakage; and Verus runs here on stable ✓, with `008` having recorded
its cost 2.26x too high (§9).

---

## 1. The framekernel — keep the discipline, drop the metric

`008` §7 flagged the Asterinas numbers behind `002` pillar 5 and D-13 as
author-measured and unreplicated. Both halves resolved, in opposite directions.

**The containment property is verified, and it is the one we rely on.** A
whole-repo search found **zero occurrences of `#[allow(unsafe_code)]` in the
entire Asterinas repository** ◆, every non-`lib.rs` hit read individually; real
drivers (virtio, NVMe, PCI, i8042, framebuffer, uart, block, network) live in
`deny(unsafe_code)` crates, at ~100k lines, 90 contributors and four years.
**The discipline scales.**

**The 14.0% TCB figure does not survive scrutiny.** Table 9: Asterinas TCB =
10,571 lines, Tock = 2,903, so **Asterinas's TCB is 3.6× larger than Tock's in
absolute terms**, reported as a 3× improvement because a ratio improves for
free as safe features accumulate. For *how much unsafe must one person
audit*, Tock wins outright. The numerator is very likely OSTD-only (the
workspace labels `hashbrown`, `bitvec`, `lru`, `itertools`, `spin`, `zerocopy`
"external dependencies only for safe crates", meaning authored without unsafe,
not linking none), and line counting misses the highest-risk unsafe
(`unsafe impl Send for Foo {}` counts as about zero). **Track absolute unsafe
lines in `wost` against a budget, never a percentage.**

**The performance caveats are worse than `003` recorded.** `mitigations=off`
is conservative, but `hugepages=0` disables THP on the Linux baseline, and Redis
(their best result, 1.31) is THP-sensitive; smoltcp's missing congestion
control is a pure win over a lossless tap, so **both network results measure a
simpler TCP stack, not a faster kernel**. The defensible summary is
*"single-core syscall dispatch is at parity"* (LMbench 1.08). **Independent
replication: none exists**; LWN's coverage audits no measurement. The project
is alive: 90 contributors, 1,111 commits in 2026, v0.18.1 on 2026-08-18.

**The strongest counterargument is legitimate.** *Containment* (all unsafe in
one crate, an audit surface that does not grow with features) costs one lint
attribute; *soundness* (safe code cannot cause UB
however hostile) is where the difficulty lives (typed/untyped frames, `!Send`
guards, sensitivity types, re-validated policy output), and it defends against
your own service code, which a solo author writes. **Converos (ATC'25)
model-checked 12 Asterinas concurrency modules and found 20 bugs in the safe
code**, and memory safety is not reliability: a misplaced `unwrap()` in safe
Rust is the same outage. So **take
containment unconditionally and soundness selectively**, naming that 80/20 in
the ADR as the fallback position.

**Patterns worth stealing**: capability tokens as arguments
(`fn alloc(&mut self, guard: &DisabledLocalIrqGuard)` makes "pinned to this
CPU" a compile-time fact); `!Send` guards; sensitivity in the type
(`IoMem<Sensitive>` vs `IoMem<Insensitive>`, with the allocator pre-removing
privileged ranges), which is what makes safe-Rust drivers work.

---

## 2. The async ABI — keep the decision, rewrite the reason

D-06 and `003` justify async-first with FlexSC: *"the win comes from batching +
cache locality (FlexSC), not from avoiding the trap."* **That sentence is
refuted by FlexSC itself.** ◆ Its numbers are as recorded (150 cycles round
trip, up to 65% IPC degradation, 44 d-TLB and 3,160 L3 entries evicted by one
`pwrite`), but §2.3 says the *direct* cost dominates below ~2,000 instructions
per syscall and the indirect cost only in the 2,000 to 100,000 band, with two of
its own three motivating workloads on that crossover, and §3 says batching
*"minimizes the frequency of switching between user and kernel execution,
**eliminating some of the mode switch overhead** … improves **both** the direct
and the indirect costs."* Batching and trap-avoidance are the same lever.

A July 2026 io_uring study for databases gives the ladder from an already-async
batched design: registered buffers +11%, NVMe passthrough +20%, IOPOLL +21%,
SQPOLL (no trap at all) +32%; against *blocking* I/O, **up to 12.5×**:

> **Structural asynchrony ≈ 10×. Total trap elimination ≈ 1.3×, and it costs a
> dedicated core.**

The trap became 5 to 10× *more* expensive than in 2010 (76 ns unmitigated,
231 to 560 ns mitigated) while caches grew 4 to 16×. **Naive io_uring adoption
bought only 1.06×**; the 33× came from the *caller* becoming asynchronous. The
decision stands; replace the ADR's rationale with the measured ordering. *The
win is in making async the natural way to write a service, not in the ring
itself*: an ABI that lets callers stay synchronous delivers the 1.06×.

**The four hard problems:**

- **Cancellation is advisory.** `-EALREADY` means *"the caller must wait for the
  canceled request to post its completion event."* Rule: **every operation
  posts exactly one terminal completion, always.**
- **Buffer lifetime: the kernel must own the buffer.** Blocking in `Drop` is
  unsound because leaking is safe. Windows RIO page-locks at registration and
  forbids deregistration while requests are outstanding.
- **Backpressure: reserve completion-queue space at submission.** io_uring's
  `CQ_NODROP` took an unbounded kernel memory liability; **RIO's
  reservation-based admission control makes overflow structurally
  impossible.** Take RIO's, and its CQ sizing: a power of two **≥ 2× the SQ
  size**.
- **Shared-ring security: copy each entry out exactly once, then validate the
  copy.** CVE-2025-39816 is *"always use `READ_ONCE()` to read ring pointers."*

**Ports and completion queues are the same shape: build one.** A Zircon port
*is* a CQ whose entries are events; two event planes are why io_uring grew
`poll_add`, `futex`, `waitid` and `msg_ring`. Lift Zircon's semantics into the
CQ: one-shot arming by default, edge versus level as an explicit flag,
**cancellation that purges already-queued entries**, one waiter released per
entry.

**io_uring's security record**: the ~60% kernelCTF figure is confirmed ◆ and
Google still disables io_uring, but the statistic is frozen *because* they did.
NVD returns **143 io_uring CVEs**, rising (2024:38, 2025:40, 2026:36 by partial
year), clustered in **newly added** surface (`zcrx`, provided buffers, `ublk`,
passthrough); by class **50 race/TOCTOU, 23 use-after-free, 10 refcount**.
**The lesson is feature velocity on a ring surface, not io_uring the name**,
which argues for `003`'s austerity. And ARMO's 2025 "Curing" rootkit did
command-and-control **without making any system calls**, unseen by Falco,
Tetragon and Microsoft Defender: **if the native ABI is a ring, "observe the
syscalls" is no basis for auditing, sandboxing or introspection**, so those
mechanisms must be designed into the ring from the start.

---

## 3. Capabilities and the handle table — `002`'s open question, answered

**The error to fix first.** `002` says **channels are the sole
handle-delegation mechanism**. That is **false, even of Zircon** ◆, which also
passes handles via `zx_process_start()`. The checkable replacement:

> A process receives exactly one handle at spawn — the bootstrap channel.

A selftest asserting a fresh process's table holds exactly one entry makes it a
D-28 check. D-151 and D-163 later add the other authorized provenance: a
handle-producing ring operation derives authority from a handle already held.

**Rights: small, and the rule matters more than the number.** seL4 has **4**;
Zircon **25**; CloudABI **31**; Capsicum **~81** (not the ~60 `003` recorded).
**Capsicum's taxonomy outgrew a machine word and broke its ABI**: `cap_rights_t`
is a multi-word struct spending **7 of every 64 bits on metadata**, with an
`init`/`set`/`is_set` API because rights can no longer be ORed ◆. **WASI deleted
its rights system in Preview 2**: it *"created a lot of ambiguity about how it
mapped to POSIX semantics."* ◆ managarm, adding rights in August 2026, landed on
**~14** ⊕. **Recommendation: 16 to 24 rights in a single `u32`**, universal
(`DUPLICATE`, `TRANSFER`, `WAIT`, `INSPECT`, Zircon's `RIGHTS_BASIC`) split from
per-type, under the rule WASI's rights violated:

> A right must correspond to a distinction the kernel actually makes about that
> object — never to a syscall name.

From day one: **attenuate-on-send** (Zircon's `zx_channel_write_etc` strips
rights in transfer, including `TRANSFER` for a non-delegatable handle; ~4%
cost, least privilege by default) and **rights declared per handle-field with
no defaults** (RFC-0028: *"Defaults have unclear behavior that can be
inconsistent across object types"*), validated fail-fast at transfer by both
ends, as managarm's `PushDescriptor`/`PullDescriptor` do ⊕.

**The handle table: one encoding, reached twice.** **Zircon has no per-process
handle array** ◆: one global arena, `base_value = generation ‖ arena_index`,
handle value `(base_value << 2 | 0b11) ^ per_table_mixer`, lookup by arithmetic
plus one ownership comparison. Generation tagging kills use-after-free on
reuse; and under the **Scalable Commutativity Rule** (Clements et al., SOSP'13:
*"whenever interface operations commute, they can be implemented in a way that
scales"*) Zircon's contract commutes **by construction**, while "lowest free
descriptor" is an ordering promise that serialises allocation. sv6's `O_ANYFD`
scaled linearly **with no single-core penalty**.

- **The handle table must not promise "lowest free handle."** ⊕
- **Put the encode/decode arithmetic in a `libs/handle-space` crate** beside
  `port_space` and `frame_space`; `wost` keeps only the arena and the lock
  (D-30 and D-43's pattern).
- Per D-28, break the two checks most likely to be silently unfailable: *"the
  invalid value never decodes"* and *"a reused slot never yields the same value
  twice."*
- **Shape it as `HandleTable { table: AtomicPtr<HandleArray> }`** (Linux's
  `files_struct`/`fdtable` split) so readers can go lock-free later.
- **Make the handle table a refcounted object, not an inline field of
  `Process`.** Zircon's per-process invariant was the first casualty of its
  compatibility layer (RFC-0261 shares tables for Starnix).

**Revocation: don't.** Zircon and Capsicum have none. seL4's recursive
revocation is **preemptible and resumable** (not "abortable", as `003`
recorded), with progress in a zombie capability and a pre-mitigation worst case
of **about one second**, reduced to 436 µs computed / 81 µs observed. **No
revocation in Phase 5**: object death plus a strippable `TRANSFER` covers the
cases, and if ever needed a **per-object epoch bump is O(1)** using the
generation counter already in the encoding. Put seL4's numbers in the ADR as
the reason.

**Sizing and qualifiers.** Zircon has **26 object types and growing**: give type
encodings headroom for 64, not 8. Policy should be **monotonic and frozen**
(Zircon's `zx_job_set_policy` is tighten-only and immutable once a job has
children), encoded so a new type renumbers nothing. `003`'s seL4 "~720 cycles"
is ◆ **718 ± 29 cycles on x86_64 Skylake, same-core fastpath, hot cache,
*without Meltdown mitigation***, and 468 → 687 (+47%) with mitigations, the
figure to compare against. Capsicum's "few percent" ◆ is measured **against
standard UNIX syscalls, not message passing**; worst case `fstat` **10.2%**,
`cap_new` versus `dup` **50.7%**.

---

## 4. SMP — the bring-up can wait; the discipline cannot

Framekernel SMP scalability is worse than unquantified: **Asterinas's own
scalability work became a separate SOSP'25 paper (CortenMM) that deletes the
VMA layer entirely** for a transactional MMU interface. The authors changed the
design rather than measuring the old one.

**Deferring SMP bring-up is correct. Deferring SMP design discipline past Phase
5 is the mistake.** Decade-long retrofits were of large kernels with frozen
ABIs; small kernels take months (**Maestro, 11 weeks**; **Theseus, 24 days** to
eight cores; **seL4, one release** with a big kernel lock).

1. **Our locks are already SMP-shaped** ✓: `wost/src/sync.rs` uses
   `compare_exchange(Acquire, Relaxed)` and `store(Release)`, carries
   `unsafe impl<T: Send> Sync`, and disables interrupts *before* the exchange.
2. **Lock ordering is observable with one CPU**: lockdep says complex
   multi-CPU deadlocks *"can be detected on a plain, lightly loaded single-CPU
   system as well!"* Atomic protocols are host-model-checkable in principle
   (§9).
3. **The one bug class one CPU hides completely is in our own plan.** x86's
   only permitted reordering is **Store→Load**, invisible to one core; the
   shape that hits it is the **lost-wakeup re-check pattern** `002` lists as
   owed before Phase 5. Message-passing shapes do not break when cores are
   added; this one does, silently.

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

**Write down now, at zero code cost:**

- **The target model**: one big kernel lock at first bring-up, per-CPU run
  queues, correct shootdown. seL4 measured a BKL at +10% on x86 IPC (*"for
  8-way parallelism … the choice of lock is essentially irrelevant"*). **Today's
  coarse locking is the plan rather than debt.**
- **Never write "disable interrupts" where you mean "disable preemption"**
  (Maestro, Theseus #603, Tock #4544). A preempt-only lock is much cheaper than
  `cli`/`sti` and the common case.
- **Make `SpinLockGuard` and `ThreadLockGuard` `!Send`** ✓ with
  `PhantomData<*const ()>`, the idiom `ThreadContext` uses.
- **No lock guard live across a context switch.** Redox's September-2025 hang
  fix was **292 lines of `{ }` scoping**. We enforce it at runtime ✓ (D-66's
  assert in `prepare()`), but it was reached from a UP deadlock, so the ADR
  should name the SMP rationale too, or someone will weaken it thinking it only
  guards a UP bug.
- **`Relaxed` requires justification the way `unsafe` does.**
- **Split `static THREADS: SpinLock<Threads>` into a thread table and a run
  queue.** `Entity::place(average, lag, slice)` in `libs/eevdf` *is* the
  migration hook; Asterinas makes per-CPU run queues an *API requirement*.

**The Phase 5 lever.** The Commutativity Rule is analysable *"before an
implementation exists and before the necessary hardware is available."* Linux
scales for 68% of 13,664 POSIX tests, sv6 for 99%: **the difference is
interface, not effort.** So: no lowest-free handles (§3); **the async queue ABI
must not promise global FIFO across submitters**; **unmap, close and release
must not promise global visibility on return** (Linux's `mmu_gather`); and **a
full barrier on both sides of the lost-wakeup re-check**, one locked
instruction on the slow path. **Rust does not save you here**: Converos's 20
bugs, and Theseus's `ActivePageTable` / `InactivePageTable` typestate was wrong
once two cores existed.

---

## 5. The POSIX bet — sound, unbudgeted, and sequenced wrong

D-07 (spawn-only) and D-10 (POSIX as a translation layer) are not wrong; the
project has priced them at zero.

**Fuchsia ran this experiment and reversed.** ◆ Capability handles with POSIX in
a userspace layer, the shape of mlibc-over-handles. RFC-0082: POSIX Lite omits
*"parts of POSIX that imply mutable global state (e.g., the `kill` function)
because Fuchsia is designed around an object-capability discipline."* The answer
was **Starnix**, a Linux kernel in userspace (**≥15 MB of Rust, ≥972 files, 493
syscall-table entries**), then RFC-0261 (2024) **modifying Zircon itself** to
make it fast, then RFC-0264 (2025) adding a second ISA. **Starnix implements
`fork`.** Plan 9's APE hit the same wall: no uid/gid, umask, controlling tty,
`link`, or `sigprocmask`/`sigpending`/`sigsuspend` ◆, POSIX's ambient-authority
and mutable-global-state surface. **Signals are on both lists.** Starnix's
answer is a 96 KB signals implementation. And Pike's own
estimate of the tax: *"90–95% of the work in Plan 9 was directly or indirectly
to honor externally imposed standards."*

**The mlibc budget, measured at D-74's unrecorded snapshot** ◆: 296 sysdep tags,
**16 mandatory** (*"enough for running a basic statically-linked hello world
program"*); real ports implement **147 (ironclad), 163 (astral), 202
(managarm), 280 (linux)**, every one with `Fork`, `Sigaction`, `Ioctl`,
`Termios`, `Poll` and `Waitpid`. Adopting mlibc adopts `abis/linux`, a
Linux-shaped userspace ABI pinned to a moving upstream (mlibc says pin a
release, not `master`).

**A trap for the CPython milestone**: `subprocess`'s `posix_spawn` fast path
requires **`cwd is None`** ◆, so `subprocess.run(..., cwd=...)` falls back to
fork+exec, as does anything setting `uid`, `gid`, `umask`, `pass_fds`,
`start_new_session` or `preexec_fn`. A spawn-only OS needs a far richer
`posix_spawn` or accepts many failing subprocess calls. CPython builds without
fork (WASI is Tier 2), so this is feature surface, not a build blocker.

Redox chose POSIX-first in 2015 and has **2,665 port recipes** against
managarm's 84 and CloudABI's 131; its userspace `fork` (~181 lines of unsafe
plus a trampoline, CoW duplication through per-grant descriptors) proves a
compat-fork is achievable on a capability kernel, and its price. CloudABI,
needing *zero* kernel work, **died of adoption**, and even it kept `proc_fork`.

### The recommendation, and it is the largest in this pass

**Move mlibc from Phase 9 to immediately after the Phase 5 ABI stabilises.** ⊕
The plan built a Phase 7 userland on a bespoke ABI and introduced mlibc in
Phase 9: **writing the userland twice.** mlibc's `sysdeps/demo` is a five-file
~9.4 KB skeleton. The goal is to answer *"can a POSIX libc sit on this ABI?"*
while the ABI is cheap to change, and find which of `openat`, `dup`,
`O_CLOEXEC` and descriptor inheritance across `posix_spawn` the handle model
made awkward, before 73 call sites depend on the answer.

**Zinnia** (one person, from 2024-01) reached a **Wayland/X11 desktop in about
2.5 years** on mlibc with a **13-file, ~45 KB** sysdep; Asterinas reached a
Linux-compatible userland in ~3 years on the Linux ABI. ToaruOS, **15 years solo
with a no-third-party-dependencies rule, still not self-hosting**; HelenOS,
**20+ years, no path to third-party software**; Redox, own libc, CPython at
10.4 years.

> **Conform at the boundary, innovate inside.** That single variable explains
> the timelines better than team size, funding, or language.

It does not weaken the capability ABI (Redox is *retrofitting* capabilities
under POSIX, which is harder); the translation layer should arrive early enough
to shape the ABI rather than inherit it. **Also owed**: `002`'s POSIX section is
19 lines, and `dlopen`, dynamic linking, signals, `termios`, `ioctl` and
pthreads appear **zero times in 6,063 lines of docs**, though CPython needs them
all. Answer **how does a signal interrupt a blocking queue wait?** before the
queue ABI is fixed.

---

## 6. WASI — the one decision that should change

D-09 makes WASI 0.3 components first-class with **wasmi** as the runtime. Both
halves are untenable. **WASI 0.3.0 did ship, on 2026-06-11** ◆, so `008`'s note
that ratification is "a fact rather than a projection" is right. But:

- **The Component Model is still a Phase 1 proposal** in the W3C WebAssembly
  CG, the lowest active phase of five ◆.
- **"Component Model 1.0" is a roadmap article dated 2026-06-08** whose open
  workstreams include *"ABI improvements — transitioning from eager allocation
  … to lazy value handles"* and *"Easier implementation — simplifying the spec
  … to reduce implementation burden."* **The Bytecode Alliance says its own
  spec is too hard to implement and its ABI is going to change.**
- **WASI 0.3 is implemented by Wasmtime and jco only**, and D-09 rejects
  Wasmtime as tokio/std-coupled. `wasm32-wasip3` is **Tier 3**.
- **wasmi has no Component Model and no plans for one**: its maintainer
  (September 2025) called it *"an enormous undertaking"* and said it remains
  uncertain if a Wasm interpreter will ever even profit; the tracking issue,
  open since 2024-01-16, left the planned list in February 2025. WasmEdge's funded team has worked on it
  since early 2024 and is not done.
- **Cost**: the spec is **~12,200 lines / ~66,000 words** on top of core Wasm;
  Wasmtime's implementation is **40,902 lines of Rust, larger than wasmi's
  entire core interpreter.**
- **The ecosystem is ten components** (three sample HTTP servers, seven
  micro-utilities); `wa.dev` returns **"No packages available."** CPython, Ruby
  and Go target `wasm32-wasip1`. **No component build of a shell, an editor,
  coreutils, SQLite, or a compiler exists**, and the niche is **a typed plugin
  ABI for a host application**. **No component-capable runtime is an
  interpreter.**

WASI's authority model aligns with handle grants and components survive a
port; D-181 adds that a descriptor adapter still owns numbering, preopens and
cursor state. **Both benefits come with core Wasm plus `wasip1`**, where the
software is; the Component Model buys typed *composition*, which pays off only
when there are components to compose.

**Restate D-09**: core Wasm + `wasip1` on wasmi, first-class. The Component
Model becomes a watch item with a named re-evaluation trigger: *it reaches
Phase 4, or an interpreter ships support*. Keep the capability-mapping
rationale; drop the components-first-class claim and the wasmi 1.0 pin, which
`008` §4.5 asked for and which is now doubly stale (wasmi is at 2.0.0-beta).
**WAMR left the Bytecode Alliance (2026-07-21)**; Fermyon was acquired by
Akamai (2025-12-01) and Spin remains a CNCF project.

---

## 7. WizFS — the design holds; the schedule does not

Every rule `003` adopted for WizFS is **confirmed** against primary sources ◆,
with one omission and two corrections.

**The omission.** bcachefs's Principles of Operation: *"unlike other COW btrees,
btree updates are journalled."* **bcachefs is a journalled B-tree over CoW
bucket storage**, which buys prefix consistency, needs no btrfs-style log tree,
makes small random metadata updates affordable and crash testing tractable. It
should be the spine of the design.

**Strike the blind-upsert ABI rule.** BetrFS (FAST '15) dissolves its own
precondition problem: *"In our experiments, the OS cache of file and directory
information was able to answer these queries, enabling file creation etc., to
run at the full speed of upserts."* Keep the ABI-independent technique (no
read-modify-write; a sub-block write is a self-contained record).

**The parity-RAID reason is wrong; the conclusion survives.** ZFS's RAID-Z
*eliminates* the write hole; btrfs's RAID56 (still **unstable**, *"write journal
is not implemented"*) and classic RAID5 carry it. The correct reason: **every
correct solution needs buckets, copying GC and backpointers first.** And FAST
'18 *kept* full-path indexing with a range-rename; choose inode indirection
because range-rename is research we do not want to invent, not because
full-path keying is wrong.

**The schedule is the problem** ⊕. **bcachefs**: "more or less feature
complete" 2015-08-21, experimental label dropped 2026-06-19, **ten years and ten
months** of one funded engineer, and **all 117,000 lines removed from Linux 6.18
on 2025-09-30**. **btrfs**: mainline 2009, format stable November 2013, removed
from RHEL 8 in 2019. **ZFS**: ~5 years and twelve-plus paid senior engineers.
OSDI '18 ◆: *"btrfs maintains its metadata … in the form of various
copy-on-write B+ trees. **This makes achieving crash consistency tricky, as the
updates have to be propagated to several trees.**"* Of 26 crash-consistency bugs
studied, **btrfs accounted for 24**, and one went **seven years** undiscovered.
**CoW makes the crash-consistency *argument* simpler and the *implementation*
harder**, and ZFS still needs the ZIL because `fsync` must be durable before
the next transaction group. SerenityOS ships **ext2 and FAT**; Redox has worked
on `redoxfs` since 2016.

**Recommendation: demote WizFS from a phase to a destination**, with a later
phase whose definition of done includes crash injection and a repair tool:

- **Phase 6: a read-only FAT32 or ext2 driver**, making the NVMe block path, the
  VFS and the buffers honest at **zero on-disk-write risk** (NVMe because
  VMware has no virtio, §10), and a real image the project did not generate is
  an independent parser test. FAT32's argument is *the oracle already exists*:
  Windows, Linux and macOS mount the image and judge the writer, categorically
  stronger than anything buildable for WizFS in one phase.
- **Before Phase 13: the crash harness, before WizFS has a format** (§9).
- **Phase 13, WizFS v0**: one btree, 4K nodes, journalled updates, no
  snapshots, checksums or GC, *"simply fails when it runs out of blocks"*, but
  with the `snapshot` key field from v0, always zero, because adding a key
  field later is a format break.
- **Not yet**: 128 to 256K log-structured nodes (justified by petabyte scale on
  spinning disks) or a copying collector.

Two hazards. **Free-space deadlock**, in btrfs's words:
*"The balance operation needs enough work space … otherwise this may lead to
ENOSPC reports."* bcachefs's answer is a **seven-level watermark ladder** plus a
reserve normal writes cannot touch, arithmetic for a `libs/` crate. **Devices
that lie about flushes** (*"historically a major (potentially disastrous)
problem for COW filesystems"*): journal scrub at mount plus a bounded rewind.
Read bcachefs's ~117-page Principles of Operation (revision 1.39.2,
**2026-08-22**) end to end before designing.

---

## 8. Performance — the axis never researched

`000` promises "fast"; this is an ordering of which decisions foreclose speed.

- **Syscalls.** A null syscall costs **78 to 649 ns** on Intel; the **vDSO path
  13 to 24 ns**: *a read-mostly kernel value should be mapped, not called for.*
  Full mitigations cost **+33%** on a syscall-bound server; KPTI <0.5% below
  10k/s/CPU, ~5% at 75k, PCID cutting 2.1% to 0.5%; seL4 IPC **468 → 687 cycles
  (+47%)**. **Zen 5 gains essentially nothing from `mitigations=off`**, but the
  kernel carries 18 hardware-vulnerability documents.
- **Extended state.** A context switch is 1,000 to 2,000 cycles direct, **10k to
  1M** with cache and TLB damage. **AMX `TILE_DATA` is 8 KiB, up to 64 KiB
  architecturally.** Linux uses **XFD** (first-touch `#NM`, then allocate), and
  **not lazy FPU restore** (CVE-2018-3665). With D-29's XSAVE deferral ending
  at ring 3: **compute the compacted XSAVE size at boot, use
  `XSAVES`/`XRSTORS`, gate with XFD, never lazy**; the size leaks into
  stack-guard arithmetic, so it is a Phase 5 decision.
- **Data path.** SPSC ring versus Unix socket at 32 bytes: **270 ns versus 5,910
  ns**, because "the data path contains zero system calls", the quantitative
  case for D-06. A silent `io_worker` fallback costs **7.3 µs**: *no silent
  worker fallback; make the completion state explicit.*
- **Memory.** **Direct-map fragmentation does not matter for data**; only kernel
  *code* huge mappings do. **D-45's 2 MiB direct map is confirmed correct.**
  Keep it unsplittable, carve pre-split regions at boot for permission changes,
  and count 4 KiB splits so the invariant is observable. ~20% of Meta's
  datacentre cycles stall on TLB misses.
- **Storage and network.** **`fsync` spans 1.6 µs (enterprise,
  power-loss-protected) to 2,974 µs (consumer)**, and the format decides how
  many flushes a durable operation costs. **`MSG_ZEROCOPY` gives 39% on netperf,
  5 to 8% on real workloads; zero-copy *receive* gives 1.85×.** smoltcp does 3.7
  to 7.9 Gbps and its limits are **no SACK, no jumbo frames, single-threaded
  poll**, not copying, worth knowing before D-20's plan meets a real NIC.
- **Rust's own cost**, since D-01 rests on it: **under 5% typically, 6 to 11%
  more cycles per packet**, concentrated in bounds checks.

**Measurement, which the project has none of.** QEMU/TCG timings are
meaningless, so **measure counts, not times** (allocations, lock acquisitions
and traps per operation; boot instruction count under `-icount`), the only
thing emulation reports truthfully; five of the six cheapest regressions to
catch are counts. Instruction-count benchmarking may hit the
§9 toolchain wall, so **acquire a netboot-able bare-metal machine, ideally one
AMD and one Intel**, because Zen 5 and Intel no longer predict each other.

**Decide before Phase 5 closes**: (1) ring transitions per unit of work, which
*is* the ABI; (2) the buffer-ownership type (pool provenance, DMA mapping,
lifetime, `Drop` returning to the pool), which decides channels, I/O and
networking at once; (3) page- versus folio-oriented memory; (4) per-CPU data
and the interrupt-state token; (5) extended state, which sizes the TCB; (6) an
entry path patchable per attack vector and FRED-shaped; (7) a measurement
capability. If only three fit: **the buffer type, the async ABI shape, and the
measurement loop.**

---

## 9. Verification — cheaper and closer than the project assumed

**The wall is real, and it is passable with two version pins** ✓. `loom`,
`shuttle` and `proptest` fail at default versions: rustup's GNU toolchain ships
`dlltool.exe`, `ld.exe` and a mingw gcc but **there is no `as.exe` anywhere on
the machine**, so **any crate that reaches `getrandom >= 0.3.2` or
`windows-sys >= 0.61`** (raw-dylib) fails.

| Crate | Default | With pins |
|---|---|---|
| `proptest` 1 | fails | **works** ✓ — `default-features = false` + `getrandom = "=0.3.1"` |
| `shuttle` 0.9 | **works** ✓ — no pins needed | — |
| `loom` 0.7.2 | fails | works with two lockfile pins |
| `cargo-mutants` | fails | still blocked |
| `cargo-fuzz` | Unix / MSVC only | out |

The durable fix is a **project-local MinGW binutils drop in `tools/`**, vendored
like `boot/limine/` and not a system install, which keeps D-16's posture.
`Cargo.toml`'s fatfs note (*"drops default features to avoid chrono, whose
Windows bindings need dlltool.exe"*) is one symptom: **a standing tax on every
future host-side dependency**.

**Verus runs here, on stable** ✓. `008` §5 recorded Atmosphere's proof-to-code
ratio as **7.5:1**; the SOSP'25 paper says **3.32:1** (20,098 proof lines over
6,048 executable), and 7.5:1 is the earlier KISV'23 workshop paper's. **Our note
understated Verus by 2.26×** (seL4 20:1, CertiKOS 14.9:1, NrOS 10:1, SeKVM
6.9:1, VeriSMo 2:1). Atmosphere took **three clean-slate rewrites** and leaves a
~3,000-line trusted TCB plus 172 lines of assembly unverified. A `no_std`,
`forbid(unsafe_code)` crate went green under `cargo test` and
`verus.exe --crate-type=lib` (6 verified, 0 errors); **Verus pins stable
1.97.1, this project's exact version.** Asterinas × CertiK's page-table race and
the TASE 2026 buddy result check out. The curve: spec arithmetic promotes to
`int`, bitmasks need `by(bit_vector)`, and **multiplication and division need
`by(nonlinear_arith)`**, the shape of EEVDF weighting and ring indices; about
**15 proof lines per 4 executable**. **Kani is out**: Linux and macOS only,
zero Windows artifacts.

**What the tools catch, measured.** A `Relaxed` publish where `Release` is
required **passed `shuttle`** (2,000 iterations): it checks interleavings and
does not model weak memory. **Loom, on the same shape, reports
`FAILED — stale payload observed`.**
So **shuttle for interleavings and deadlock, loom for memory ordering**.
Loom's blind spots belong in the ADR, because D-28's argument cuts both ways:
a check that cannot fail is also a claim.

| Break | loom |
|---|---|
| `Release` → `Relaxed` on the publish store | **red** ✓ |
| drop the `Acquire` on the index load | **red** |
| publish the index before the data | **red** |
| `SeqCst` → `AcqRel` on a doorbell fence | **green — false pass** (loom models SeqCst as acquire-release) |
| relaxed intra-thread reordering | **green — false pass** |

The doorbell protocol, §4's store-then-load shape, **must therefore be reasoned
about by hand, not merely tested.** **Loom's two threads over-approximate a
uniprocessor kernel with interrupts**, so a loom-clean primitive is correct on
UP today and pre-validated for SMP: a reason to do this in Phase 5.

**TLA+ runs before any Rust exists.** A 39-line ring-buffer spec checks in
**under a second** (35 states); delete the producer's free-slot check and TLC
reports `Invariant NoOverrun is violated` in **0.72 s**. JDK 21 is present, so
the cost is one vendored JAR. **Model the Phase 5 ring protocol before writing
it**, and the Phase 13 crash-consistency protocol before freezing the format.

**The crash harness for WizFS (before Phase 13).** CrashMonkey, Hydra, FSCQ and
Yggdrasil were last touched in 2022 and test mounted kernel filesystems on real
devices, so the harness must be built as small host-side Rust. OSDI '18: *"most
reported bugs can be reproduced using small workloads of three or fewer
file-system operations on a newly-created file system, and all reported bugs
result from crashes after fsync-related system calls."*

0. **Write the persistence contract as an ADR**, prefix consistency
   recommended. Without it there is no oracle.
1. **`libs/blockdev-sim`**: a seeded deterministic device with a volatile
   cache and barrier semantics, yielding every crash prefix. ~400 lines.
2. **A reference model and `allowed_states()`**: ~300 lines.
3. **`fsck` as a library function** returning a structured report, a second
   independent oracle.
4. **Exhaustive one- and two-operation sequences**, crash at persistence points,
   recover, fsck, compare against the model.
5. **Crash at *every* prefix, sampled by seed**, closing the gap the OSDI
   harness leaves by assuming *"the core crash-consistency mechanism… is
   working correctly"*, which is false when the CoW core is the new code.
6. **`arbitrary` + a committed corpus** for `parse_node(&[u8])` ✓.

**The D-28 demonstration**: delete the flush between the data write and the
journal commit; **stage 4 still passes, stage 5 goes red.** Stages 1, 2, 4, 5
and 6 are host-testable `libs/` work: **a `no_std`, host-buildable on-disk
format crate is what makes deterministic crash simulation possible at all.**

**Where the line is.** **Full functional verification is declined**: FSCQ was
**31,397 lines of code and proof over 1.5 years** for a filesystem simpler than
xv6's, and Hydra later found a bug in it; BilbyFS spent **9.25 person-months
and 13,000 proof lines on two operations.** **Targeted Verus is a real
option**, where **the specification, not the proof, becomes the dominant cost**
(Asterinas's page tables: ~6,000 spec lines, ~2,000 proof lines), and a proof
against a wrong specification is a confident wrong answer.

> **Design-for-verifiability is a Phase 5 decision, not a Phase 7 one.** NrOS
> paid 13.3:1 and Atmosphere 4.3:1 *for the same page-table logic*. The
> difference was a flat, pointer-centric permission layout versus Rust recursive
> ownership.

That choice must be made before the ring and the handle table are written.

Named rather than adopted: **`cargo-mutants` mechanises D-28** (it would have
caught the Phase 3 slice-1 check that passed with its code deleted) and is
blocked only by the binutils wall; **SquirrelFS** uses typestate for crash
ordering and is worth reading before Phase 13.

---
## 10. Platform — what to assume, and what not to foreclose

**x86S is cancelled** (Intel, December 2024: *"pivoted away from the x86S
initiative"*). Real mode, rings 1 and 2, and the 8259 are permanent: **keep
actively masking the 8259 via the MADT's `PCAT_COMPAT` flag, never assume it is
absent.** The Intel+AMD Ecosystem Advisory Group, a convergence body rather
than a slimming one, finalised **FRED** and
**AVX10**, specified **ChkTag** (unified memory tagging), and named **ACE** as
AMX's successor.

**Baseline**: **"the oldest CPU with SMAP and UMIP", Skylake-era 2015**, not a
`vN` label: x86-64-v2 plus XSAVE/OSXSAVE, x2APIC, TSC-deadline, an invariant-TSC
check, SMEP/SMAP/UMIP. UEFI-only with no CSM remains correct. **D-33's
x2APIC-only decision holds** (MSR access is a smaller `unsafe` surface than an
MMIO window), but it should become a **checked, diagnosable boot precondition
rather than an assumption**, a D-28 negative test because `-cpu qemu64,-x2apic`
produces a machine without it. No Broadcom document says VMware exposes x2APIC;
empirically it does.

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

Since the last pass: **ITS** (2025), **VMSCAPE** (CVE-2025-40300), and Linux's
**Attack Vector Controls** (6.16), whose shape is worth stealing.

**FRED** replaces IDT delivery and `IRET` with `ERETS`/`ERETU`, IST with four
reentrant stack levels, `SWAPGS` with `LKGS`, delivers a uniform frame, and
makes supervisor shadow stacks tractable. Our QEMU 11.0.50 ✓ lists `fred`,
`lkgs` and `wrmsrns` among its known CPU features and accepts
`-cpu max,fred=on`; full emulation needs a boot test, but FRED is "testable
here, today". **Design Phase 5's trap entry FRED-shaped without implementing
FRED**: `swapgs` behind `arch::percpu`; `iretq` behind
`arch::trap::return_to_user`; trap stacks as an enum mapped onto IST today; and
**dispatch from a table on a decoded `TrapCause`, rather than 256 IDT stubs
that each know their own vector.** The decode is arithmetic for `libs/`, and
it is what an aarch64 port needs, a signal it is the right abstraction rather
than speculative generality.

**The XSAVE decision Phase 5 forces** (D-29 deferred it until a scheduler):
**eager, compacted, boot-sized, out-of-line, CPUID-derived, with the arithmetic
in `libs/`.**

- **Eager only** (lazy restore is CVE-2018-3665); record it so nobody
  re-optimises it back in.
- **`XSAVES` → `XSAVEC` → `XSAVE`, never `XSAVEOPT`**, whose
  modified-optimisation is a correctness hazard. Prefer `XSAVES` even with no
  supervisor state: it shares `XSAVEC`'s compacted format, and CET's
  shadow-stack pointers live in supervisor xstate.
- **`XCR0` = x87 + SSE + AVX only**: every enabled bit is per-thread memory
  forever, and **AMX's `XTILEDATA` alone is 8 KiB.**
- **`XstateBuf` out-of-line and growable**, 64-byte aligned, sized at boot,
  never a fixed array inside `Thread`, which would make AMX or ACE impossible
  to add.
- **Keep `+soft-float`, with a CI check that it stays**: it is what keeps trap
  entry from touching xstate.
- **Reserve `#NM` (vector 7)** for a future XFD allocator, a first-use
  allocation trap, not lazy restore: one is a security bug and the other is
  not.
- **APX is state component 19 at offset 960** in the non-compacted format, a
  higher index at a lower offset (Linux added an ordering table in 6.16).
  **Compute offsets from `CPUID.0DH` into a table in a `libs/` crate,
  host-tested against synthetic CPUID snapshots including the APX case**;
  `wost` keeps only the four instructions and the MSR writes.

**virtio is not the right first device family.** VMware Workstation, the D-18
gate, **does not implement virtio at all** (VMXNET3, PVSCSI). The common
denominator across QEMU, VMware and hardware is **NVMe** for storage,
**`RDSEED`** for entropy, and the UART and Limine framebuffer for console.
Build virtio-blk only as a teaching step, not as the path to a VMware boot.

**Limine.** The vendored binary is from `v10.x-binary`, fetched 2026-07-18, and
D-42 requests base revision 4. **Base revision 5** also keeps the EFI system
table mapped and the local APIC defined on every processor. **Limine 11.3.1
(April 2026) added a TSC Frequency request**, which would retire Phase 4 slice
3's calibration problem and the *"counter is not advertised invariant"* boot
warning; it needs a vendored-binary update and a fresh checksum under D-25, so
it is a deliberate change. **The clocksource should be an abstraction with at
least two implementations chosen at boot**, because TCG punishes a hard-wired
TSC.

**Portability.** **Stay x86_64-only, and do not port.** RISC-V is a sensible
second target around 2028, and a second architecture costs roughly **30 to 40%
of total kernel effort to date**. Shape now, at no cost:
`AddressSpace::activate()` rather than `load_cr3`; `PageFlags` rather than raw
PTE bits; `arch::PAGE_SIZES` and `VA_BITS` rather than literals; an opaque
`TrapContext` with a decoded `TrapCause`; `arch::percpu` rather than `swapgs`;
`arch::interrupt` and `arch::timer` rather than APIC and TSC nouns; `arch::io`,
since port I/O does not exist elsewhere; `arch::cache`, a no-op on x86 and
mandatory elsewhere; and `SyscallArgs` with named fields, so no portable code
reads `rdi`. **aarch64 has two translation roots, so a
single-value `Cr3` type is the number one portability trap**, and **be pedantic
about atomic `Ordering` immediately**: x86's TSO hides a wrong one, and it
cannot be refactored away later. If a port happens, make it aarch64.

---

## 11. Doc hygiene — what has gone stale

Found by auditing the docs against the code and the ADRs.

**`002-architecture.md`**, all seven items applied under §12's proposal 4:

1. **Pillar 1's "the sole handle-delegation mechanism"** is factually wrong
   (§3); replace it with the checkable spawn invariant.
2. **The kernel-memory accounting question** ("is seL4-style untyped-retype …
   worth adopting?") was **closed by D-44**; the doc still asks.
3. **"CHERI/CET/PKS hardware security pass"** was **closed by `008` §6.**
4. **"Re-verify Tier-2 claims" and "framekernel SMP scalability is
   unquantified"** are closed by this document, with answers that change
   something.
5. **"Weigh Capsicum's ~60 per-method rights vs Zircon's smaller set"** is
   answered (§3); the count is ~81.
6. **"Runtime: wasmi 1.0"**: `008` §4.5's softening was never actioned; wasmi
   is at 2.0.0-beta and §6 changes the decision anyway.
7. **The security baseline headed "on from the start, not retrofitted" lists
   KASLR**, which D-46 deferred to Phase 12, so the heading is false as
   written; `000-vision.md`'s Core decisions table repeats it.

**Outstanding from `008` §8:** thirteen of sixteen proposals landed inside one
phase. **#15, the wasmi pin**, is overdue and §6 supersedes it. **#10, Kani
harnesses**, was unactionable all along (Kani does not run on Windows, §9);
replace it with Verus. #16 (the queue-austerity checklist) is pending until the
Phase 5 queue doc exists. **`008` §5's Atmosphere ratio**, **7.5:1**, is
corrected there to **3.32:1** (§9).

**A structural suggestion.** `008` §7 records the Asterinas result as
unreplicated while **D-13 carries "Status: Accepted"**. A third status:

> **`Provisional`** — accepted and built on, but resting on evidence that has
> not been independently confirmed. Carries a **falsification condition**: what
> would have to be observed for this to be revisited.

First holders: D-13 (replication), D-09 (Component Model reaching Phase 4, or
an interpreter shipping support), and D-08 (whether the §9 crash harness can be
built before the format is).

---

## 12. Proposed actions

Proposals, not decisions, as in `008`; ordered by when the door closes.
**Decided 2026-08-23, the same day.** Isaac took the list and answered it:

| Proposal | Outcome |
|---|---|
| 2 — workspace `unsafe_code = deny` | **Taken** — D-67, applied |
| 3 — `!Send` lock guards | **Taken** — D-68, applied |
| 4 — the doc-hygiene items | **Taken** — applied across the current docs; historical passes are labelled |
| 5 — borrow-based lock token | **Taken as an ADR** — D-69; implementation deferred to Phase 5 |
| 9 — SMP discipline ADRs | **Taken** — D-70; the cheap code and an early `-smp 2` were declined for now |
| 13 — restate D-09 | **Taken** — D-72, `wasip1` first-class |
| 14 — demote WizFS | **Taken** — D-73, it becomes Phase 13 |
| 17 — move mlibc forward | **Taken** — D-74, a stub sysdep closes Phase 5 |
| 20 — NVMe before virtio | **Direction recorded**; the implementation ADR is due in Phase 6 |
| 21 — rename the Phase 11 milestone | **Taken** — D-75 |
| 23 — update Limine | **Recorded** as a Phase 5 item |
| 25 — `Provisional` ADR status | **Taken** — D-71; D-08, D-09 and D-13 reclassified |
| 27 — conformance lane | **Taken**, arriving with the mlibc slice (D-74) |
| 1, 30–33 — the verification tooling | **Staged** — see below |
| 34 — correct the Atmosphere ratio in `008` | **Taken** — corrected to 3.32:1 |
| 6–8, 10–12, 15–16, 18–19, 22, 24, 26, 28–29 | Open, or Phase 5's business |

**The verification tooling is staged by judgement.** `proptest` works today
with a `getrandom` pin and has callers (`libs/eevdf`, `libs/timebase`,
`libs/frame-space`, `libs/port-space`), so it is adopted first. The rest wait
for their first real use, per D-41's rule that an API is written alongside its
first caller: **the vendored binutils and loom when Phase 5 has a ring protocol
to check, TLA+ at the start of Phase 5 to model that protocol before it is
written, and the Verus spike when there is a bound worth proving.** Adopting
them now would be tooling in search of a subject.

**Highest value per minute**

1. **Vendor MinGW binutils into `tools/`**, checksum-pinned under D-25 (§9).

**Free now, verified ✓**

2. **`unsafe_code = "deny"` in `[workspace.lints.rust]`**, `wost` opting out in
   source; tested both directions.
3. **`!Send` `SpinLockGuard` and `ThreadLockGuard`** (§4).
4. **The seven doc-hygiene items** of §11.

**Before Phase 5 writes code**

5. **Make `SpinLock::lock` borrow the `ThreadContext`** ✓: "block while holding
   a spin lock" and "take two spin locks" become compile errors. It **depends on
   the guard having a `Drop` impl** (without one, NLL ends the borrow early, so
   the break-test is *delete the `Drop`*), and handlers need `lock_from_trap()`
   because `ThreadContext::claim()` returns `None` in trap context. 13 statics,
   73 call sites.
6. **A preempt-disabled-interrupts-on lock** ✓, OSTD's *default* guardian (§4).
7. **The handle table rules** of §3.
8. **The queue ABI** (§2): one terminal completion per operation; kernel-owned
   buffers; **reservation-based backpressure**; read-once-then-validate by API
   shape; single-owner rings; **no global FIFO promise across submitters**;
   kernel-allocated ring pages.
9. **The SMP-discipline ADRs** of §4, including **splitting `THREADS`**.
10. **The extended-state design** (§8, §10).
11. **How a signal interrupts a blocking queue wait** (§5).
12. **A FRED-first trap entry** (§10), decode in `libs/`.
13. **A second stack region slot** (CET shadow stack) and **a fourth W^X
    permission class**, before the allocator is fixed (§10).
14. **`VERW` gated on `MD_CLEAR` and an eIBRS MSR write** from day one, both
    near-impossible to retrofit gracefully.
15. **Reserve PTE bits 62:59** and **range-check user pointers rather than
    testing `>> 47 == 0`**: two rules that cost nothing and preserve
    PKS/LAM/ChkTag optionality.
16. **x2APIC as a checked boot precondition**, `-cpu qemu64,-x2apic` its D-28
    negative test.

**Sequencing changes**

17. **mlibc immediately after the Phase 5 ABI stabilises** (§5).
18. **Restate D-09** as core Wasm + `wasip1` on wasmi (§6).
19. **Demote WizFS to a destination** (§7) ⊕.
20. **NVMe before virtio** (§10).
21. **Rename the Phase 11 milestone honestly.** NetSurf's last release is 3.11
    (2023-12-28), its JavaScript *"primitive and incomplete… disabled by
    default"*, 1,539 bindings missing; its framebuffer frontend is a
    right-sized trophy proving libc, windowing, networking and TLS, not
    "browse the web," which `000` promised.
22. **"CPython's test suite passes" as Phase 9's acceptance test**: threads,
    signals, subprocesses, sockets and the filesystem at once.
23. **Consider updating Limine** to base revision 5 and to 11.3.1+ for the TSC
    Frequency request (§10), with a fresh checksum under D-25.

**Practices the evidence supports**

24. **Absolute unsafe lines against a budget**, never a percentage (§1).
25. **A `Provisional` ADR status** with falsification conditions (§11).
26. **Counts, not times** (§8).
27. **A conformance lane** (os-test and the Open POSIX Test Suite, counts over
    time) beside `libs/` and `selftest.rs`, as Redox and managarm converged on:
    D-28 extended past the kernel boundary.
28. **A dated monthly note**, the most common trait among projects that kept
    moving.
29. **A netboot-able bare-metal machine**, one AMD and one Intel (§8).

**Verification, staged (§9)**

30. **Vendor `tla2tools.jar` and add `cargo xtask model-check`**; model the
    Phase 5 ring protocol before writing it.
31. **`libs/ring-proto`, generic over its atomics**, checked by shuttle for
    interleavings and loom for memory ordering, with a deliberately-`Relaxed`
    variant that must fail, per D-28.
32. **`proptest` on the ring indices and on `libs/eevdf`** ✓.
33. **One Verus spike on one function** (the slot-mask bound), via `verus.exe`
    rather than `cargo-verus`.
34. **Correct `008` §5's Atmosphere figure**: **done**, 3.32:1 not 7.5:1, with
    the rewrites and trusted TCB the headline omits.
35. **Treat design-for-verifiability as a Phase 5 decision.** NrOS paid 13.3:1
    and Atmosphere 4.3:1 for the same page-table logic; the difference was the
    data layout. That choice is made when the ring and handle table are
    written, not afterwards.

**Explicitly declined**

36. **Full functional verification** (§9).
37. **seL4-style revocation** (§3): a one-second pre-mitigation worst case,
    zombie capabilities, and two documented partial-completion cases.
38. **Writing an AML interpreter**: `rust-osdev/acpi` has one, and Redox rebased
    onto it in April 2026 where it *fixed real hardware*.
39. **The own-bootloader side-quest**: a maintained alternative exists and it
    teaches nothing Phases 0 to 4 have not.
40. **Kani** (§9). Revisit only if a Linux CI runner appears, where
    `--autoharness` makes it nearly free.
41. **Coverage-guided fuzzing** (`cargo-fuzz`, afl.rs, honggfuzz): Unix or
    MSVC-only; `arbitrary` plus a committed corpus covers the need ✓.
