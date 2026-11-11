# 011 — Userspace (Phase 5)

The phase that gives the kernel someone to serve. Ring 3, a handle table that
is a process's external-object authority, `spawn` in place of `fork`, ELF
images loaded from an initrd, an opaque bootstrap identity, and the
submission/completion rings the native ABI is made of, closing with a stub
mlibc sysdep whose only job is to find out whether a POSIX libc can sit on this
ABI while the ABI is still cheap to change (D-74).

Two interfaces are fixed here that this project will live with longer than any
other, the handle table and the queue ABI, and both are the kind that get
frozen by their first caller. That is why this file was written before the code
(D-19), and why the ring protocol got a model before it got an implementation.
Each slice is recorded as it was built.

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

Quantities and host-tool observations below are dated proposal or slice-close
measurements. The phase-close total is the total at the end of Phase 5, not a
promise that later phases retain it; `014-storage.md` owns the checked current
gate totals.

**Current-state correction (D-248).** The channel and Event sections below are
the historical Phase 5 record, not the current implementation inventory. The
kernel now retains one counted, opaque `Channel` identity solely because every
spawned program starts with one typed bootstrap handle. It has no paired end,
message buffer, send/receive, wait, backpressure or orphan semantics. `Event`
and the duplicate HandleTable-level channel/close operations were removed
until a concrete post-gate caller fixes their contracts. The queue's direct
`Duplicate` operation and `WaitQueue::wait_unless` lost-wakeup proof remain.

The debts Phase 5 opened with, and what became of each, are the table at the
close of this file. Not this phase's debt: KASLR waits for Phase 12 (D-46), SMP
bring-up waits for Phase 12 (D-70), the device-entitlement half of D-77 waits
for Phase 6's PCI discovery, and compat-fork mechanics wait for Phase 9 (D-07,
D-74).

## Slices

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
| 3 | The address space that grows: two-way mapping, mappings that carry their evidence, synchronous unmap invalidation with a frame-ownership token, the double-fault stack guarded | ✅ 2026-08-24; TLB lifetime amended 2026-08-30 |
| 4 | Trap entry, FRED-shaped: `libs/trap-cause` and table dispatch, the boot thread's own guarded stack, eIBRS and the speculation report, x2APIC as a checked precondition | ✅ 2026-08-24 |
| 5 | Ring 3: user descriptors, one gate at DPL 3, the SMAP copy guard, `VERW` on the way out, the XSAVE layout in `libs/x86-cpuid` | ✅ 2026-08-24 |
| 6 | The handle table: `libs/handle-space`, rights in one `u32`, a refcounted table, the object header, per-process quota | ✅ 2026-08-24 |
| 7 | Memory objects, spawn, ELF loading, initrd: `libs/elf64`, a second address space, `UFrame`, per-thread `RSP0` and eager XSAVE, stacks that arrive with their thread, two processes, exactly one handle each at spawn, and `THREADS` split into a thread table and a run queue | ✅ 2026-08-24 |
| 8 | Historical channel implementation; retired to one opaque bootstrap identity by D-248 | ✅ 2026-08-24; superseded 2026-08-30 |
| 9 | The async queue ABI: the model before the protocol, `libs/ring-proto` under shuttle and proptest, reservation backpressure, exactly one terminal completion | ✅ 2026-08-24 |
| 10 | The stub mlibc sysdep (D-74): written and analysed, not compiled — the toolchain wall is recorded in D-114 | ✅ 2026-08-24 |
| 11 | Phase close: the VMware boot (D-18), the adversarial audit (D-77), the roadmap and this file's closing section | ✅ 2026-08-24 |

## The handle table

`002` listed "handle table representation and rights taxonomy" as owed before
this phase. `010` §3 answered the taxonomy half; this section takes the layout
and the lifetime rules (D-78).

**One global arena, not a per-process array.** As in Zircon: one arena,
`base_value = generation ‖ arena_index`, and the handle value a process sees is
`(base_value << 2 | 0b11) ^ per_table_mixer`. Lookup is arithmetic plus one
ownership comparison. The generation turns use-after-close into a decode
failure, and the mixer means a value that leaked from one table is not a handle
in another.

**The handle table does not promise the lowest free handle** (D-70 rule 7),
and the reserved low bits make the encoding non-dense, so the promise is
unmakeable rather than merely unmade. POSIX's "lowest free descriptor" is an
ordering promise that serialises allocation; by the Scalable Commutativity
Rule an interface whose operations commute can be implemented to scale, and
sv6 measured the unordered alternative scaling linearly with no single-core
penalty.

**The encoding arithmetic goes to `libs/handle-space`**, beside `port_space`
and `frame_space` (D-30 and D-43's pattern). The kernel keeps the arena and the
lock. Per D-28, the two checks most likely to be silently unfailable here are
*the invalid value never decodes* and *a reused slot never yields the same
value twice*; both get broken.

**Shape: `HandleTable { table: AtomicPtr<HandleArray> }`**, Linux's
`files_struct`/`fdtable` split, so readers can go lock-free later without
touching a call site.

**The table is a refcounted object, not an inline field of `Process`,**
because Zircon's strict per-process invariant was the first casualty of its
own compatibility layer (RFC-0261 shares handle tables for Starnix).

**Rights: 16 to 24 in one `u32`**, universal (`DUPLICATE`, `TRANSFER`,
`WAIT`, `INSPECT`, Zircon's `RIGHTS_BASIC`) split from per-type. Capsicum's
taxonomy outgrew a machine word and broke its ABI, and WASI deleted its rights
system in Preview 2. The rule that keeps the number where it is:

> A right must correspond to a distinction the kernel actually makes about that
> object — never to a syscall name.

**Type encodings get headroom for 64 object types, not 8**, because Zircon is
at 26 and still growing. Policy is
**monotonic and frozen**: tighten-only, immutable once a process has children,
Zircon's `zx_job_set_policy` shape, with bits encoded so that adding an object
type never renumbers an existing one.

**No revocation.** Object death plus a strippable `TRANSFER` right covers the
real cases; seL4's recursive revocation costs preemptible, resumable
operations. If revocation is ever needed, a per-object **epoch bump is O(1)**,
and the epoch already exists as the generation counter in the handle encoding.

**Lifetime.** A handle is a reference; the object dies when the last one goes.
Refcounts use `inc_not_zero` from day one (D-70's list) so that a lookup racing
a close cannot resurrect a corpse. Closing does **not** promise global
visibility on return (D-70 rule 9), the same decision as Linux's `mmu_gather`
batching, and it is unmakeable later.

**The quota obligation D-44 assigned here** is answered at this layer, not the
frame layer: a process's kernel memory is bounded by what its handle table can
name, and the bound is a policy number that stays revisable. Making callers
donate memory would have been an ABI decision and permanent; this is not.

## The queue

`002` listed "queue memory layout and wakeup semantics" as owed before this
phase. This section takes it (D-79), and `010` §2 supplies the answers.

**The rationale was rewritten.** D-06 justified async-first with FlexSC,
reading it as batching and cache locality rather than trap avoidance; FlexSC
says batching reduces mode switches and improves both direct and indirect
costs. The attribution that matters for API design: naive io_uring adoption
bought 1.06×, and the 33× came from the caller becoming asynchronous. The win
is in making async the natural way to write a service; an ABI that lets callers
stay synchronous will deliver the 1.06×.

**The four hard problems, answered.**

- **Cancellation is advisory.** io_uring's own manual: `-EALREADY` means "the
  caller must wait for the canceled request to post its completion event." The
  rule this becomes: **every operation posts exactly one terminal completion,
  always.**
- **The kernel owns the buffer.** Blocking in `Drop` is unsound because leaking
  is safe, so a userspace-owned buffer has no sound lifetime. Windows RIO
  states the rule kernel-side: page-locked at registration, deregistration
  forbidden while requests are outstanding.
- **Backpressure is reservation at submission**, RIO's admission control, which
  makes overflow structurally impossible where io_uring's `CQ_NODROP` took an
  unbounded kernel-memory liability. The CQ is a power of two and **at least
  twice the SQ size**.
- **Copy each entry out exactly once, then validate the copy.** The double
  fetch is the canonical hole (CVE-2025-39816). This is enforced by the shape
  of the API, not by discipline: the validated copy is the only thing the
  kernel can reach. D-221 later closes the distinct ordering half:
  acquire the producer's tail, volatile-copy once, then release the consumer
  head after use. D-282 keeps those externally writable words outside Rust's
  atomic-object model while preserving the same x86 ordering.

**Ports and completion queues are the same shape; build one.** Building both
gives userspace two event planes to join, which is why io_uring grew
`poll_add`, `futex`, `waitid` and `msg_ring`. Zircon's four semantic wins move
into the CQ instead: one-shot arming by default, edge-versus-level as an
explicit flag, cancellation that purges already-queued entries, and exactly one
waiter released per entry.

**Austerity, from io_uring's security record.** Its 2025 to 2026 CVE cluster is
in newly added surface (`zcrx`, provided buffers, `ublk`, passthrough): the
lesson is feature velocity on a ring surface. Ring structures carry explicit
version fields (IoRing's improvement); the opcode surface stays minimal; there
are no privileged opcodes.

**No global FIFO across submitters** (D-70 rule 8): an ordered queue forces one
shared queue, an unordered one allows per-core queues. Rings are single-owner
and the pages are kernel-allocated.

**Observing syscalls is not a valid basis for auditing, sandboxing or
introspection** when the native ABI is a ring. ARMO's 2025 "Curing" rootkit did
command and control without a single system call, and Falco, Tetragon and
Microsoft Defender were blind to it. Every observability and security mechanism
this kernel will ever want has to be designed into the ring from the start, so
slice 9 owes at least the hook, even if nothing consumes it until much later.

## How a signal interrupts a blocking queue wait

The question D-74 called the hardest to answer late, and `002` did not mention
signals at all.

**A signal must share the completion plane** (D-80). The target blocking wait
is interruptible through that same mechanism rather than a second event plane,
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

Slice 2 changed what enforces most of it (D-85). `SpinLock::lock` takes
`&'a mut ThreadContext` and the guard borrows it, so **while one token is in
scope**, taking a second lock with it or blocking with the guard live are both
`E0499`. `lock_leaf` is the second level, for an acquisition that is the whole
operation — the only one a `Drop` impl can perform, since `drop` takes no
arguments. `HEAP → FRAMES` is no longer an order this kernel has.

**The compile-time half is a strong default, not a proof.**
`ThreadContext::claim` mints a token wherever the interrupt flag says thread
context, so a second one can be minted inside a live guard and the borrow says
nothing about it. That is one visible, greppable line, audited like an `unsafe`
block. What holds unconditionally is the runtime side: D-66's assert refuses a
context switch with a guard live, and `ThreadLock` refuses to be taken while a
spin lock is held. Two independent checks is what OSTD ships, and here the
second is the one that cannot be side-stepped.

D-194 closes the remaining `Drop` question as a bounded liveness contract,
not a stronger type claim. Safe code can forget a guard or never
return, and no closure or token can guarantee progress against either. Such a
caller can strand this CPU and the lock's waiters but gains no memory or
privilege authority. Every transition which would abandon that holder keeps
D-66's runtime refusal. A self-test-only controlled `mem::forget` distinguishes
ordinary contention from deliberate leaked ownership and then restores the
machine; shipped code has no force-release door.

**No closure runs under a leaf guard.** `mm::with_memory_map` and
`dev::with_io_apic` ran caller-supplied code with a lock held; D-121 deleted
both closure forms, and query results are copied out after the guard drops.
The separate `mem::with_active_space` door was not a leaf-lock violation, but
it did hand safe code the unrestricted live address space; D-189 later made
that borrow private and replaced it with narrow operations.

A preempt-only lock would be a third kind below both: it disables preemption
and leaves interrupts on, so it may be taken while nothing else is held and may
not be held across a block. None exists (see the close). Never write "disable
interrupts" where "disable preemption" is meant: the single most-cited
breakage across the SMP survey, and the conflation this kernel currently has.

**`ARENA` is a leaf**, added by slice 6. Every acquisition is
take-mutate-return: growth is reserved outside the lock and the drop buffer is
sized before it is taken (D-95), because calling the allocator under the guard
rebuilds the `ARENA → HEAP → FRAMES` order D-85 deleted. An object's last
reference is always released *after* the guard is gone, because dropping one
can reach the frame allocator.

For every later object and ring the rule is the one D-70 rule 5 already binds:
**interrupt handlers never block**.

## How it is checked

Phase 4's rules carry over whole (`009-multitasking.md`): host arithmetic
first (D-30), every mechanism with an on-machine check broken once (D-28), a
break that reaches the path the check watches, and a VMware boot at the close
(D-18). A check whose break would corrupt state every later check runs on goes
to a host crate as arithmetic instead, as slab slot uniqueness did to
`libs/slab-layout`.

This phase adds two kinds of claim.

The **model**: the ring protocol is specified in TLA+ and checked before it is
written, because the Scalable Commutativity Rule's decisive property is that it
is analysable "before an implementation exists and before the necessary
hardware is available", and because the one bug class a single CPU cannot
show, Store→Load reordering, is exactly the shape of the lost-wakeup re-check
the ring depends on. D-70 rule 6's full barrier on that re-check is settled by
the model, not a self-check. `cargo xtask model-check` runs it. A deliberately
weakened invariant must produce a counterexample, or the model is decoration.

**Host concurrency testing** of `libs/ring-proto` under shuttle and proptest.
On 2026-08-24 `010` §9's wall held: shuttle 0.9 worked unpinned, proptest
needed `default-features = false` and `getrandom = "=0.3.1"`, and anything
reaching `getrandom >= 0.3.2` or `windows-sys >= 0.61` failed for want of an
assembler. The pins are a recorded tax under D-16. A concurrency harness that
passes a knowingly wrong program is not a harness.

D-112 records that the proposed Shuttle break (a `Relaxed` variant) itself
passed: Shuttle explores its own scheduling points under sequential
consistency and cannot answer a weak-memory question. D-221 replaces that false
oracle with an explicit event order model bound to the actual `wost`
acquire/release methods; weakening either edge admits the
published-index/old-payload outcome and fails its exact host witness. D-282
replaces the intervening Rust atomic object with the exact aligned x86 access
because ring 3 is a hostile machine-code participant, not a Rust atomic peer.

`002` pillar 1 had claimed channels were the sole delegation mechanism, which
is false even of Zircon. Slice 7 replaced it with the part that remains
checkable, and landed the check in the same commit:

> A process receives exactly one handle at spawn — its bootstrap channel.

D-151 later made handle-producing completions an authorized second provenance
path without changing what a process holds at birth.

## Audit holes and their later disposition

The Phase 5 adversarial pass (D-94) carried these rather than patching them
without a decision. Later audits supplied those decisions; this list preserves
the finding without presenting it as live work.

- **Closure under a leaf guard: closed by D-121.** The closure forms are gone;
  leaf-backed queries return copied values after releasing their guard.
- **Stale `MappedStack` evidence: closed at installation by D-125 and after
  installation by D-214.** Installation re-walks the live mapping, and an
  installed hardware stack makes its complete virtual extent irrevocable.
- **Broad live-address-space access: closed by D-189.** The unrestricted safe
  closure is private; production callers get only the narrow mapping operation
  they require.

## Slice 2 — the lock discipline in the borrow checker (2026-08-24)

**D-69's counts were exact and its shape was not** (D-85). A `Drop` impl
cannot receive a token (`Frame` and `Segment` free themselves from wherever
they die), so `FRAMES` could never be a token-bearing lock, and neither can
`GlobalAlloc::alloc`, whose shape the language fixes. So a lock has **two
levels**, not a rank lattice: `lock(&mut ThreadContext)` borrows the token for
the guard's life, and `lock_leaf()` is for an acquisition that is the whole
operation, calling nothing. A leaf may be taken while a token-bearing lock is
held, which is what makes a nested acquisition sound rather than an exception.
The token earns its keep on the blocking paths; `libs/lock-rank` and
`lock_from_trap` went unwritten because a rank with one non-trivial pair is not
an abstraction and every acquisition a handler makes is a leaf.

**The heap stopped nesting.** `take_slot` returns null on an empty free list
and the caller allocates the frame with the lock *dropped*, re-acquires and
carves. `wost` holds no lock across a call at all; every non-leaf lock in this
kernel is the kernel's own.

**The guarantee is `Drop`-dependent**: without `Drop for SpinLockGuard`, NLL
ends the borrow at the call and both `E0499` errors vanish. **Closed by
D-247.** A `compile_fail` doctest still cannot run where the cross-compiled
types live (D-41), and a copy in `libs/` would still drift. D-223 supplied the
durable alternative: a host harness cross-compiles probes against the actual
`wost`. D-247 extends it to require the two live-guard programs to fail with
exactly `E0499`, require the explicit-drop control to compile, and retain
removal of `SpinLockGuard`'s `Drop` impl as the mutation.

## Slice 3 — the address space that grows (2026-08-24)

**`activate` borrows** (D-86). D-45 kept `AddressSpace::map` sound by making it
unreachable, but D-77 had already hardened `set` so that a present entry
accepts only an identical rewrite, and *that* is what stops a live mapping
moving under the running kernel. Mapping into a running space is safe for a
reason that is checked rather than promised. An
`ActiveSpace`/`InactiveSpace` typestate was rejected because `010` §4 records
Theseus's was wrong once two cores existed; a `wost`-held global behind a
closure would have put a lock where the kernel's policy belongs, against D-43.

**A mapping carries its evidence** (D-87), D-76's first authority debt and the
reason D-77 left it: *"Phase 5 must replace the first with owned
user-memory mappings."* There is no public entry taking a bare physical address.
`map_owned` takes frames the caller owns and moves them in; `map_boot` takes a
`BootRegion`, which exists only for a range the bootloader's map covers,
page-aligned, non-empty and non-overflowing; `map_device` was already the
narrowest. The direct map and the kernel image are nobody's frames, so
demanding ownership there would be a fiction; what is demanded is that the
handoff described them, the bound D-64 gave physical reads.

**Unmap pays translation debt before returning frame ownership.** A CPU may
still hold a cached translation to a cleared entry, so every leaf removed from
the current root is invalidated synchronously. Inactive roots contain no
caller-selected global leaves and this kernel keeps PCID disabled, so their
next `CR3` load flushes every cached mutable translation. The returned token
therefore retains only the frames and their user provenance: forgetting it
leaks those frames but cannot preserve a stale mapping (PW-131, amending
D-88's deferred-flush mechanism).

Provenance is bit 9 of the page entry, one of the three the architecture leaves
to software: a frame the allocator handed out must come back and one the
firmware placed must not, and the entry is the only thing every path to an
unmap can see. Large pages are not split: the direct map is two-megabyte on
purpose (D-45), and `010` §8 measured that as right.

**The double-fault stack is guarded** (D-89). It stood on a `.bss` array with
nothing below it, where an overrun corrupts its neighbour *while the fault
handler is running*. It now stands on a guarded region whose geometry is the
thread arena's own, placed at its end, so there is no new hardcoded window and
no entry to D-46's ledger. It is the first shipped caller of
`with_active_space`: an unsafe-boundary change whose only user is its own test
is one to be suspicious of.

**CET's second region is not foreclosed, and needed no code to keep open.**
`010` §10's requirement is that the stack allocator not make a shadow stack
impossible to add. `libs/stack-layout` computes every address from
`(base, stack_len, guard_len, slots)` and hands back a `Slot`; a second
region per thread is a field on `Slot` and a term in the stride, in a
host-tested crate with no hardware in it. Building an unused slot now would
be the speculative API `005` forbids; recording that the arithmetic is
parameterised is the whole of what was owed.

The frame D-86's check maps is never unmapped; it costs four pages for the life
of a boot.

## Slice 4 — trap entry, FRED-shaped (2026-08-24)

**The decode is one place** (D-81's shape, `libs/trap-cause`), host-tested over
all 256 vectors and cross-checked against `x86-tables`. The handler no longer
matches on a raw vector number or throws the page-fault error code away, so a
report says *why* a fault happened as well as where.

**The gate boots a machine the kernel cannot fully use** (D-90). A fourth boot
runs on `-cpu qemu64,-x2apic` and requires the kernel to name the missing
feature and still reach its prompt, because on `-cpu max` every precondition
had only ever been met. D-33 left no fallback to test, so the diagnosis *is*
the behaviour.

**The speculation decisions live in `libs/x86-cpuid`** (D-91), driven by
synthetic capability words, because TCG does not emulate
`IA32_ARCH_CAPABILITIES` and an unexercised branch inside the memory-safety TCB
is the worst place to put one; `wost` keeps a `cpuid` and two MSR accesses.
Silence is not safety there: a processor with no capability word is a different
value from one reporting all clear, because it is the majority case and the
wrong answer for it.

**And the last stack got its guard** (D-92). The bootloader's stack lives
inside the direct map, which D-88 refuses to split, so the boot thread leaves
it and `kernel_main` ends at the switch. The primitive is safe because its
argument is the proof: `map_stack` maps the region and then *verifies the guard
is absent*, and no caller could have told from the return value. **D-193 closes
the later fail-open edge:** if that mapping refuses, diagnostic kernel service
continues but process construction returns a typed refusal before allocation
and ring 3 remains closed.

**`002`'s security baseline now has no exceptions.** Every stack this kernel
runs on has an unmapped page beneath it, and each has a check that goes red
without it.

Trap stacks stay a single IST user rather than an enum, because an enum with
one variant is not an abstraction (`005`). The Limine base-revision-5 bump
moved to slice 11, because replacing a checksum-pinned bootloader binary (D-25)
is a deliberate change whose only failure mode is not booting.

**Still owed.** `ENDBR64` on the indirect targets in `global_asm!` is written
down (D-81) and not written: nothing enables kernel IBT, so today it would be
an instruction with no effect and no way to tell if it were wrong. It arrives
with the first thing that turns IBT on.

## Slice 6 — the handle table (2026-08-24)

The generation **wraps** rather than saturating, because a saturating counter
would make one slot's last generation valid for ever, the failure the counter
exists to prevent, made permanent instead of transient.

The ownership identity does **not** share that tradeoff. D-204 widens it to a
non-repeating `u64`, commits it with the table's quota under the arena lock, and
refuses the terminal sentinel. Mixers can repeat because they only prevent
accidental values; the full identity is the authority comparison.

Lookup is decode, bounds, generation, owner. `Arc` is the table's refcount:
D-78 wants `inc_not_zero` from day one, and `Weak::upgrade` is that operation
with the atomics already audited.

**`Event` was the first object**, exercising the table, rights, refcounting and
lifetime without waiting on ring 3. D-248 removed it after the caller audit
found that only its own checks reached it.

**A wake from thread context arms the timer** (D-93). `unblock` changed who was
runnable and armed no timer, which the kernel's own module doc forbids; the
first waker in thread context left the woken thread Ready with nothing to run
it, on a tickless kernel where there may be no timer at all.

**Registration and the condition check happen under one lock** (D-95).
`Event::wait` checked its condition before registering, so a signal in the gap
woke an empty queue and the waiter slept for ever. `WaitQueue::wait_unless`
does both under the lock a waker must take.

**D-78's rights rule is enforced.** An event shipped with `READ`, `WRITE`,
`EXECUTE` and `MAP` granted and has no pages. `Rights::are_meaningful_for` and
a per-type mask make the rule enforceable, and `insert` refuses the rest.
`attenuate` must not clamp, because clamping turns "give me more than I have"
into a silent success, and `contains` must be subset, because an any-of form
grants everything.

`mod handle` and `mod object` were `#[cfg(feature = "selftest")]` until `spawn`
became their first shipped caller in slice 7; D-248 later reduced that shipped
object surface to the opaque bootstrap identity.

D-222 closes the `NotYours` versus `Stale` occupancy oracle before a direct
Wizzy handle operation can freeze it. One exact slot-generation probe formerly
returned `NotYours` for a live foreign entry and `Stale` once that entry was
absent: one architectural occupancy bit per probe, though no authority crossed
and the current ring already flattened both to `BAD_HANDLE`. Empty, reused and
foreign slots now all return `Stale`, as Zircon does; the ownership comparison
remains on resolve and close. The result carries zero foreign-occupancy bits.
This is an API-result claim, not a constant-time or microarchitectural
side-channel claim.

## Slice 5 — ring 3 (2026-08-24)

**An interrupt gate at DPL 3, not `syscall`** (D-96). D-81 left the
`syscall`-versus-trap-gate question open and said the measurement that would
settle it is meaningless under TCG. It is settled on something else: `syscall`
needs `swapgs`, `swapgs` needs a per-CPU block, and that is two unproven
mechanisms in the commit that establishes a privilege boundary. A gate the CPU
reaches through the interrupt table needs neither, because it loads `RSP0` from
the task-state segment itself. It is an *interrupt* gate, not a trap gate: the
flag is clear on entry, and `libs/x86-tables` offers no trap gate at all.
`syscall` becomes an optimisation with something working to compare against, on
hardware, where the comparison means something.

Exactly one gate admits ring 3, and it is named rather than defaulted. Every
other vector faults when user code says `int`, because a device's line is not a
service.

The descriptor order is fixed for `sysret` already: `USER_DATA` sits *before*
`USER_CODE`, which is what `STAR + 8` and `STAR + 16` require. Moving a
descriptor later means moving every selector that names it, so the order is
settled now and the mechanism is not.

**Shared kernel mappings have their own admission proof** (D-250). D-81's
statement that the target processor reports `RDCL_NO` did not prove the
processor actually running this image, and D-91 already established that an
absent capability word says nothing. A native processor or ordinary virtual
machine now enters ring 3 only after an affirmative `RDCL_NO`. The controlled
QEMU development machine has one narrow exception because TCG does not model
`IA32_ARCH_CAPABILITIES`: the guest must observe the hypervisor bit and the
exact QEMU TCG CPUID base tuple, while the host runner pins TCG and consumes the
guest's exact policy marker. That tuple is trusted development configuration,
not attestation; no generic hypervisor, QEMU or VMware signature is accepted.
Without either proof the kernel may continue diagnostic service, but process
construction returns a typed refusal before allocating anything. KPTI remains
the trigger when supported native hardware without `RDCL_NO` must run user
programs.

**That KPTI trigger has fired, and its implementation is present while external
qualification remains open** (D-407). Current VMware guests read `0x44` without
`RDCL_NO`; the different value printed in a Workstation log is not guest
architectural authority. A no-`RDCL_NO` native process will therefore bind a
pair: a full root with its user mappings and the kernel half, and a stripped
root with the same lower-half user subtrees plus only the exact supervisor
entry enclave. The existing D-250 shared-root path remains available only for
affirmative guest-read `RDCL_NO` or the exact host-pinned QEMU TCG development
tuple. Neither path silently falls back to the other.

The first privilege-crossing instructions run on a dedicated guarded
transition stack. They save the user frame, load the full root, move the frame
to the pinned ordinary task stack, clear the transition stack and only then
call Rust. The return tail finishes every secret-capable access under the full
root, copies only sanitized user-visible state onto the clean transition stack,
loads the stripped root and reaches `iretq` through enclave assembly without
touching an ordinary kernel byte. The entry enclave contains only dedicated
page-aligned stubs/common assembly, hardware-read descriptor bytes, one
non-secret routing page and the transition/IST stack pages. No ordinary kernel
text, data, direct map, task stack or device window is present there.

D-106's task binding becomes authority over both roots: a context switch
installs the destination's full root, and only the final exit tail can install
its stripped mate. D-88's no-PCID flush and release evidence cover the shared
lower tables and both root frames, and cleanup retains the complete pair on any
refusal. The host contracts and mutations 2,048 through 2,061 are retained, and forced-paired debug and
optimized QEMU reached 290/290. PW-580 and PW-706 stay open for the complete
release sweep and fresh self-test plus shipped VMware boots.

**`enter_user` is safe because it checks** (D-96). `iretq` will send the CPU to
an unmapped address in a privilege level that cannot report it, so the tables,
the entry's user-executability, the stack's user-writability, the alignment and
`RSP0` are all walked first.

**The ring-0 stack moves with the thread** (D-99): `RSP0` follows the thread,
so more than one can be in ring 3, and each needs its extended state saved
(D-108).

**The evidence is the selector the CPU pushed.** A ring-3 `CS` in the trap
frame cannot be manufactured by a kernel thread saying `int 0x80`; the number
is the hardware's account of where the caller was.

**The user copy is the boundary** (D-97). `stac` and `clac` must not be given
`nomem` or `preserves_flags`: `stac` changes whether a *later* access faults,
so `nomem` let the optimiser move the copy out of the window, and the flag it
sets is `EFLAGS.AC`. Debug passed and release faulted. D-65 made the gate boot
both profiles for overflow checks; this is the same lesson through a different
door.

**`VERW` is armed unless the processor claims `MDS_NO`** (D-98), rather than
gated on `MD_CLEAR`, which QEMU does not report, so gated that way the
instruction would have shipped unexecuted. `verw` exists on every x86-64 and
`MD_CLEAR` says only whether it also flushes. A machine whose capability word
cannot be read gets the instruction rather than the assumption.

**A ring-3 fault retires the program, not the machine** (D-102). D-31 made a
fault terminal when a fault was necessarily a kernel bug; after D-96 it can be
a program bug. `is_the_programs` in `trap.rs` refuses `#DF`, `#MC`, NMI and
anything from ring 0, and everything else from CPL 3 retires the caller;
`a_faulting_program_dies_alone` is in the gate.

## The phase closes (2026-08-24)

### The debts, settled

Every debt the phase opened with, with what actually happened rather than what
was proposed. Where the answer is "not paid", it says so and says where it went.

| Debt | Settled |
|---|---|
| Boot and double-fault stacks unguarded | **Paid.** D-89 guards the double-fault stack; D-92 moves the boot thread onto a guarded one, since the bootloader's could not be guarded where it sat |
| The address space cannot grow after boot (D-45, D-49) | **Paid.** D-86: `activate` borrows rather than consumes, and mapping is reachable on a live space |
| Memory objects, moved out of Phase 3 | **Paid only in the part that had a caller.** Owned frames, `map_user`, `map_shared` and the untyped split exist. A first-class `MemoryObject` does not; PW-701 waits for the later zero-copy/runtime caller that can determine its shape |
| `map` accepts any physical address without evidence | **Paid.** D-87: every mapping entry takes evidence — an owned run, a `BootRegion`, or a `USegment` — and D-105 closed the two doors that let the evidence be forged |
| The untyped split, absent on purpose | **Paid.** D-107, with the caller it was waiting for |
| Port claims prove exclusivity, not entitlement | **Not this phase, as planned.** Phase 6, with PCI discovery |
| XSAVE, deferred until there is a scheduler (D-29) | **Paid.** D-82's offset arithmetic, host-tested including the APX case; D-108: eager, boot-sized, out of line, and the compacted form is live: the VMware boot reports `compacted`, which QEMU never exercised |
| `swapgs` and the GS base, left for ring 3 (D-29) | **Not paid, and the reason changed.** D-154 moved the held-lock count behind the real `arch::percpu` seam. There is still one CPU and no `syscall` entry, so nothing needs a `gs:` accessor or `swapgs`; those move with SMP to Phase 12 |
| No ring-3 descriptors | **Paid.** D-96 |
| The lock discipline in the borrow checker | **Paid.** D-85, first, so nothing here was written twice |
| No preempt-disable mechanism | **Not paid.** Every lock a handler can reach must keep disabling interrupts, and today that is all of them. Every lock a trap handler can reach still disables interrupts, which is correct and is not what D-70 rule 2 asked for. A preempt-only lock has no caller that would benefit on one CPU, and building one without a caller is how an abstraction gets the wrong shape. Phase 12 |
| The per-CPU word widens (D-56, D-70) | **Structural seam paid by D-154; GS-backed CPU selection not paid.** One CPU and D-96's interrupt gate need neither a `gs:` accessor nor `swapgs`; both remain with SMP in Phase 12 |
| Per-process quota | **Paid at the handle layer** (D-111: 64 handles per process). **Not paid** for frames or for per-message allocation, which is D-44's other half and needs an accounting owner |
| A wait with a timeout, and a wait on two queues | **Half paid.** D-164 made `RingWait` sleep until the earliest operation deadline. A wait on multiple queues remains absent because nothing calls for one |
| The thread cap of sixteen is a real cap (D-52) | **Not paid.** D-86 and D-88 made stack mappings follow spawn and exit, but the arena remains a fixed sixteen slots. That is a ceiling, not a dynamic-stack implementation; PW-701 names the trigger for changing it |
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

Recorded the day it happened (D-18):

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
a boot check reading numbers ring 3 owns; `xstate::init` enabling SSE before
three paths that refuse, leaving two processes sharing the register file with
nothing saving it; a safe public walk taking a bare physical address;
`map_shared` and `map_user` able to alias one run; and three `lock_leaf`
obligations breached by this phase's own code. Two patterns carry forward:

- **A check must compare what the thing under test touches.** Ask "which
  deletion turns this red" before the check is trusted.
- **The guard must be the hardware, not a flag remembering the hardware.**
  `extended_control()` once returned early on a remembered flag rather than
  reading the register.

`reap_up_to`'s head bound and `Ring::reap`'s empty-queue refusal each cover
every input the other does, so neither can be shown load-bearing; D-115 says so
rather than shipping a confident comment on a redundant guard.

### Still owed, and where it went

- ~~A program cannot drive its own ring.~~ **Closed by D-116**: the gate
  dispatches on `rax` and only `exit` retires, `serve` reserves at the doorbell
  rather than in an earlier call, and the initrd's program posts an operation,
  rings the doorbell and reads its own answer. The shipped kernel runs it,
  because a kernel that has userspace and never enters it has not shipped
  userspace. 125 checks.
- ~~**`FS_BASE` is a fourth thing that must follow a thread**, found by
  writing the sysdeps (D-114).~~ **Closed by D-118.** It lives in
  `task::prepare` with `RSP0`, `CR3` and the extended state. Under
  D-114's former vocabulary that made it one of four MAPS entries; D-181's
  exact register calls its body WIRED and its platform remainder WORKS. The
  kernel range-checks the base against `USER_END`: a program asking to point
  `fs:` at kernel memory has no reason it could be given.
- ~~Nothing is reclaimed.~~ **Closed by D-117.** A retired program's resources
  become a zombie; D-203 gives deterministic progress by waking the mandatory
  service worker, while idle and the next spawn remain opportunistic fallback
  collectors. An address space remembers its own page tables so it can give
  them back. The program's *stack* is still the boot-time arena, so sixteen
  remains a cap on concurrency, no longer on programs ever run. **D-190
  removes D-117's former 32-table tracking ceiling:** ownership grows fallibly before a table frame is taken, every table returns, and a
  refused table cleanup is a typed, logged and counted result rather than a
  zero-valued success record. **D-191 closes construction as well as
  destruction:** a never-run process remains one rollback transaction through
  the reserved-thread commit point, and loader placements are per-call data
  with the executable entry validated before allocation. **D-192 makes the
  final ring settlement fail-atomic:** a terminal post retains its pending
  operation until publication succeeds, and collection retains the complete
  zombie until the ring is observably quiet.
- ~~A fault during a user copy is still terminal.~~ **Closed by D-120**, the
  last correctness hole the close named in the boundary. The copy is assembly
  with its faulting instructions between named labels, a page fault in that
  range resumes at a fixup, and the caller gets a count of what did not move.
  The trap dispatch D-81 built is what consults it. D-279 later
  removed the safe general unwalked door because a lower-half leaf can still be
  supervisor-only. Production copy now walks user permission first; only a
  fixed, selftest-only driver exercises the unwalked recovery assembly (PW-538).
- ~~`xtask/src/elf.rs` carries its own transcription of the ELF header
  offsets.~~ **Closed by D-158**, which moved the builder to `libs/elf64`.
- ~~No completion carries a handle.~~ **Closed by D-163** for same-object
  Duplicate; D-173 later added OpenAt's cross-type derivation.
