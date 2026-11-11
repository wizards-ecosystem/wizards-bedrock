# 008 — Deep Research II: Rust Practice, Kernel Design & the State of the Art

> **Historical pre-Phase-3 snapshot.** `010` re-checks this pass and records the
> current conclusions. In particular, it corrects Atmosphere's proof-to-code
> ratio, replaces the WASI target, and stages verification tooling differently.
> Numeric measurements and words such as "current" are dated to this pass;
> normative choices and present repository state live in `004-decision-log.md`.

Produced 2026-08-22, opening Phase 3. It closes the gaps `003` flagged and arms
the decisions `007-memory.md` holds open.

**Method and its limit.** Two independent research passes, run without sight
of each other, merged into one document: about 25 searches, 8 primary documents
fetched, every claim carrying its source. This is **not** the adversarial
multi-vote protocol that produced `003`'s Tier 1, but because the passes ran
blind to each other, where they agree the agreement is worth something. Marks:

- **◆**: a primary source (paper, kernel doc, protocol spec, official book)
  was fetched directly rather than summarised from search results.
- **✓**: verified locally by compiling against this repo's pinned toolchain,
  or by reading the pinned dependency's own source.
- **⊕**: both passes reached this independently. Not proof, but the closest
  thing to confirmation this document has.

Everything unmarked is `003`'s Tier 2: sourced, plausible, cite-checked, and
to be re-read at the source before an ADR leans on it.

**Where the two passes agreed** ⊕: keep the framekernel (Asterinas/OSTD, ~14%
TCB); keep the API Guidelines and Rust for Linux convention base; keep
capability handles with rights-attenuating duplicate/replace and channels as
the sole delegation path; keep async submission/completion queues as the
native data path but austere; keep EEVDF-inspired tickless scheduling with
policy separable; keep WizFS's bcachefs-style key-versioned snapshots and
checksums beside pointers. For Phase 3 both gave: a host-tested page-table
encoding crate first, typed frame ownership, an explicit direct-map contract,
W^X from the first owned CR3, guard pages, zero-on-allocate, and structural
selftests that walk the tables after the switch. **None of the existing
architecture needs to change.** The findings below are additions, sharpenings,
and one live risk.

---

## 1. The Rust you are actually allowed to use

Pinned at 1.97.1, edition 2024, stable-only (D-01). Rows marked ✓ were compiled
against the pinned toolchain for `x86_64-unknown-none` under edition 2024.

| Feature | Since | What it unlocks in this repo |
|---|---|---|
| `#[unsafe(naked)]` + `naked_asm!` | 1.88 ✓ | Interrupt entry and context switch as *functions* rather than `global_asm!` blobs |
| `Layout::{repeat, extend_packed, dangling_ptr}` | 2026 ✓ | Phase 3's heap arithmetic, without hand-rolling it |
| Volatile operations in `const fn` | 2026 ◆ | `const` constructors for `MmioRegion` |
| Atomic transmutation without `unsafe` | 2026 ◆ | Ring-index and flag words in Phase 5 |
| Unaligned pointer accessors for `repr(packed)` | 2026 ◆ | ACPI tables, on-disk WizFS structs |
| C-variadic *definitions* | 2026 ◆ | Phase 9's mlibc sysdeps |
| `core::error::Error` | 1.81 | Already adopted (`005`) |

**Naked functions are the notable one.** `wost/src/arch/x86_64/interrupt.rs:320`
builds the interrupt trampoline with `global_asm!`. A `#[unsafe(naked)]` fn has
a declared Rust type, a symbol the type system knows, `///` docs the lints can
enforce, and a `# Safety` section, none of which a `global_asm!` string can
have, so the blob is outside every mechanism `005` mandates. Phase 4's context
switch is the natural place to adopt the pattern; converting the existing
trampoline is a judgement call about churn.

**The allocator API is still not stable, and Phase 3 must design around that.**
A draft PR proposes stabilising the `Allocator` trait with `Global`, `System`,
the `*_in` constructors and a `GlobalAllocator` trait for installing
implementors globally, but it still needs FCP and has not landed. So Phase 3's
heap goes in through `#[global_allocator]` and the stable `alloc` crate, with
the *policy* kept separable so that adopting `Allocator` later is a type change
rather than a rewrite (§3.5).

**Edition-2024 items.** `unsafe_op_in_unsafe_fn` is already `deny` in the
workspace lint table, ahead of the edition's `warn`. Unsafe attributes
(`unsafe(no_mangle)`, `unsafe(export_name)`, `unsafe(link_section)`) matter
wherever the boot glue or linker script names a symbol, and `static_mut_refs`
means the per-frame metadata array (§3.1) cannot be a `static mut`: it needs
`wost::cell` or equivalent from the first line.

---

## 2. Conventions and documentation — proposed additions to `005`

### 2.1 A third standard worth adopting: Microsoft's Pragmatic Rust Guidelines

It is more specific about `unsafe` admission than `005`'s two authorities (Rust
API Guidelines, Rust for Linux) and maps onto the `wost` boundary almost
exactly. Its distinctive contribution is a **closed taxonomy of valid reasons to
write `unsafe`**: novel abstractions (a new smart pointer or allocator),
performance (`get_unchecked`), and FFI/platform calls. Anything else is a
defect. `005` says "an operation that cannot be expressed safely", which is a
judgement; the taxonomy is a checklist a reviewer can apply. It also requires
plain-text reasoning for *calling* unsafe as well as *providing* it (the `//
SAFETY:` / `# Safety` split `005` took from Rust for Linux) and recommends
`unsafe_op_in_unsafe_fn`.

*Proposal:* add the taxonomy to `005`'s "What may go in `wost`" section as the
admission test, alongside the no-speculative-API rule.

### 2.2 Rustdoc specifics `005` does not yet state

- **The first sentence is the module listing and the search result.** One
  sentence, ending in a period, about 15 words at most. `005` asks for one
  sentence but not the length discipline.
- **Section order**: summary, body, `# Examples`, `# Errors`, `# Panics`,
  `# Safety`. `005` mandates three of these but not the order.
- **`#[doc(hidden)]`** for items `pub` for technical reasons only, such as the
  support items `wost::entry!` needs in scope.
- **`#[doc(auto_cfg)]` is on at crate level by default**, so feature-gated
  items get their `cfg` badge without `#[doc(cfg(...))]`: free documentation
  for the `selftest` feature. `#[doc(auto_cfg = false)]` opts out.
- **The `no_std` intra-doc-link trap.** A `no_std` crate cannot resolve an
  intra-doc link to a `std` item; the workaround is
  `#[cfg(doc)] extern crate std;`. With `broken_intra_doc_links = "deny"` and
  `cargo xtask doc` as a gate step, it presents as a CI failure whose cause is
  not obvious.

### 2.3 One place `005` is more restrictive than it needs to be

`005` rejects C-EXAMPLE outright: "these crates are `no_std` and
cross-compiled, so doctests cannot run and unrunnable examples rot." That is
true of `wost` and `kernel`, **not** of `libs/`, which carry
`#![cfg_attr(not(test), no_std)]` so `cargo test` builds them for the host,
where doctests compile and run under the bare `cargo test` that
`default-members` scopes.

*Proposal:* split the rule. Examples stay optional in `wost` and `kernel`; in
`libs/` they are encouraged, because there they are executable checks rather
than rot. Page-table bit packing is the kind of API where a three-line example
is the clearest statement of the encoding.

### 2.4 Which unsafe document is actually the authority

Worth settling before the page-table crate and frame types bring `wost` its
largest batch of new `unsafe` since the split. The **Unsafe Code Guidelines
Reference** describes itself as largely historical and advisory and documents
open questions as much as rules, so it is the wrong one to treat as binding. The
practical anchor is the **Nomicon's sound-wrapper principle**: safe code must
not be able to trigger UB through your abstraction, however it is called. That
is one testable question for every new `wost` export, and it is the standard
`005` already applies when it says an abstraction must make the unsafe operation
*checkable*, not merely *wrapped*.

Because `MmioRegion` and the frame types hand out pointers into memory something
else can change, the aliasing model matters: **Tree Borrows** has replaced
Stacked Borrows as the working aliasing model (it fixes overeager `&mut`
uniqueness and the confinement of pointers to the size of the type they were
created with; Stacked Borrows rejected ~54% more test code). Neither model is
normative yet, and neither can be run against this kernel (Miri cannot execute a
cross-compiled `no_std` binary), so the discipline has to come from the API
shape: the argument for the untyped-memory typing in §3.2 rather than for
`Miri`.

*Proposal:* name the Nomicon's sound-wrapper principle in `005` as the
standard `wost` exports are held to, and note the UCG's advisory status so
nobody cites it as a rule later.

---

## 3. Phase 3, decision by decision

### 3.0 The Limine base revision — the live risk, and it is cheaper to fix than it looks

The one finding that changes a *premise* rather than a choice, and it lands on
slice 1. **The protocol is at base revision 6, and revisions 0 to 5 are marked
deprecated** ◆. The revision history matters more than the deprecation:

| Revision | What it changes, relative to 3 |
|---|---|
| 3 (current pin) | HHDM covers **only** Usable, Bootloader-reclaimable, Executable-and-modules, Framebuffer |
| **4** | HHDM **adds Reserved (Mapped), ACPI reclaimable, and ACPI NVS**; introduces the `RESERVED_MAPPED` memory-map type; guarantees ACPI tables are mapped |
| 5 | SMBIOS mapped as `RESERVED_MAPPED`; stricter x86-64 control-register handoff; additional I/O APIC entries masked; IOMMUs disabled; local APIC initialised to a defined state on every processor |
| 6 | Drops identity mapping; aarch64/RISC-V/loongarch register-state guarantees (nothing x86-64-relevant) |

**Base revision 4 is the answer to D-33.** `wost/src/boot/mod.rs:146` currently
carries this comment on `BootInfo`'s deleted direct map: *"under base revision
3 it covers only the regions the memory map lists — not the device holes — so
nothing in this phase could use it."* Base revision 4 lifts exactly that
limitation, and it guarantees the ACPI tables slice 5 reads to find the I/O
APIC through the MADT are mapped.

**And the crate pin does not block it** ✓, contrary to the obvious reading that
it waits on the `limine` 0.6.x upgrade `001-boot.md` lists as open:

- `limine` 0.6.5 still carries `#![feature(ptr_metadata)]` and **fails to
  compile on stable 1.97.1** ✓ (confirmed by building it; `ptr_metadata` is
  still unstable, rust#81513 ✓). The pin to `=0.5.0` remains correct, and
  `001-boot.md`'s open item stands as written.
- The base revision is a *protocol* parameter, not a crate version. The pinned
  0.5.0 crate exposes `BaseRevision::with_revision(u64)` as a public
  `const fn` ✓; `BaseRevision::new()` is merely `with_revision(3)`.

So requesting base revision 4 is a one-argument change at
`wost/src/boot/mod.rs:195`, inside the single file `005` designates as the
whole of the bootloader dependency. The crate-version blocker and the
protocol-revision choice are independent, and only the first is blocked.

*Caveats to carry into the slice, not reasons to defer:*

- The **vendored Limine v10 binary** must support the revision asked for.
  `is_supported()` is already checked and logged (`base_revision_supported` in
  `BootInfo`), so this fails loudly, but it is a boot test and should be the
  first thing slice 1 does.
- The 0.5.0 crate does not know the `RESERVED_MAPPED` constant, so memory-map
  handling must treat unknown types defensively: the `#[non_exhaustive]`
  discipline `005` mandates for error enums, applied to a foreign enum.
- Even with device holes HHDM-covered, Phase 3 should map device MMIO itself
  through `MmioRegion` with an explicit memory type. Base revision 4 removes
  the wall; it does not remove the reason for the door. This matches §3.3's
  direct-map contract, which excludes device holes deliberately.

*Proposal:* make the base-revision posture an explicit ADR, including the
decision **not** to go past 4 or 5 if that is the call, and why. The repo
requests revision 3 only because `BaseRevision::new()` happens to default to
it, which is not a decision.

### 3.1 Frame allocator shape — and the `wost`/`kernel` boundary

`007` asks how much allocator logic lives kernel-side as safe code. Asterinas
answers it, and the answer has a name: **Safe Policy Injection** ◆. The
framework defines the *mechanism* and the invariant; the *policy* is safe Rust
outside the TCB. Asterinas has three injection points: task scheduler
(`Scheduler`), frame allocator (`FrameAlloc`) and slab allocator
(`Slab`/`HeapSlot`). The framework retains only the invariant-enforcing
conversion, `Frame::from_unused()`, which makes their Inv. 1 hold: *any
newly-allocated `Frame` or `Segment` originates from currently unused memory*.
Buddy trees, per-CPU free lists and sizing heuristics above it cannot violate
the invariant because they can only speak in `Frame`s.

Their frame system is worth copying in shape: `Frame<M>` (one page) and
`Segment<M>` (contiguous run), generic over per-frame metadata held in **one
static array sized at bootstrap**, holding refcounts; a drop to zero returns
the frame to the allocator ◆.

*Proposal for the ADR:* `wost` owns `Paddr`, `Frame`, `Segment`, the metadata
array and `from_unused`; `kernel` owns the allocator policy as safe Rust behind
a `wost`-defined trait. The same boundary `port::claim` draws: the framework
verifies exclusivity, the caller chooses what to claim.

**Bitmap versus buddy, and why the phase forces the answer.** A bitmap is one
bit per frame (32 KiB per GiB) and linear to search; a buddy handles contiguous
multi-frame allocation. The direct map wants 2 MiB pages (§3.3) and Phase 8's
DMA wants contiguous runs, so a single-frame bitmap will be outgrown *inside
Phase 3*. Staged (bump-until-heap, then real) remains right, but the real stage
should be order-aware from the start.

**A verified one exists to read first.** *Formal Verification of a Rust-Based
Buddy Physical Memory Allocator* (TASE 2026) verifies UtManager, a bitmap-based
buddy, in Verus: invariants preserved by allocation and free, postconditions,
bounds safety, overflow freedom and termination, at zero runtime overhead, and
the verification **found a previously unknown control-flow defect** in the
original.

### 3.2 Untyped-retype — the evidence says reject the mechanism, adopt the discipline

`002`'s open question conflates two ideas that share the word "untyped".

**seL4's untyped/retype (the accounting mechanism) has structural costs.** Each
untyped capability carries one allocation cursor; retype moves it to the new
object's alignment and then past its size. A 4 KiB object then a 16 KiB one
leaves 12 KiB of padding unreclaimable until *both* children are revoked. The
community critique goes further: fixing the allocation policy inside the kernel
contradicts seL4's own principle of keeping policy out of the kernel, because
the kernel will refuse a gap that would satisfy the alignment. With `003`'s
finding that revocation is long-running and needs abortable sub-operations for
"incremental consistency", the mechanism is expensive in three separate ways.

**Asterinas's untyped memory (the type discipline) is a different thing and it
carries.** ◆ Safe Rust references cannot uphold their guarantees for
*externally modifiable* memory (user-mapped, DMA-capable, MMIO), so such memory
gets `UFrame`/`USegment`, whose read/write interface permits only copies of
plain-old-data types. Consequences:

- `VmSpace` (the user address space) accepts **only** `UFrame`/`USegment`,
  which structurally prevents mapping kernel memory into userspace. Not a
  check; a type.
- `DmaCoherent` and `DmaStream` likewise accept only untyped memory. Initially
  *no* memory is DMA-accessible.

*Proposed position for the ADR:* **reject seL4-style untyped-retype as the
kernel-memory accounting mechanism; adopt Asterinas-style untyped memory as a
type discipline in `wost` from the first frame type.** Kernel-memory DoS is
then answered at the handle layer in Phase 5 by per-process quotas, a revisable
policy, rather than by making every caller donate memory, which is a permanent
ABI decision. Record the naming collision so it is not re-litigated. Getting
the discipline right early costs near zero; retrofitting it is the cost `002`'s
pillar 5 and Asterinas's own lesson warn of.

### 3.3 Table access — direct map, with an explicit contract

The direct map with 2 MiB pages is the modern standard: Linux maps 32 GiB in
**132 KiB of page tables** with 2 MiB pages, each TLB entry covering 2 MiB.
Recursive mapping costs an address-space slot and awkward invariants for a
benefit the HHDM already hands over.

*Proposal for the ADR's contract:* the direct map covers **usable RAM only**,
NX and writable, 2 MiB pages wherever alignment permits, 4 KiB at the edges.
Device holes are deliberately *not* in it: they are mapped through
`MmioRegion` with their own memory type (§3.4), which is also what makes the
sensitivity discipline in §4.1 expressible.

*Debt to record now:* once SMP arrives, unmapping anything from the direct map
requires a TLB shootdown, an IPI to every CPU. Single-core makes it free today;
writing it down now is cheaper than discovering it in Phase 12.

### 3.4 Framebuffer memory type — and why it is the same decision as the back-buffer

Write-combining is not a marginal optimisation. Linux's MTRR documentation
puts it at "2.5 times or more" for
image writes; field reports on framebuffer redraw run far higher. PAT
supersedes MTRR on modern x86; Linux uses both only because non-PAT systems
exist, and for a UEFI-only target under QEMU and VMware PAT is always present,
so the MTRR half is dead weight.

**WC makes writes fast and reads catastrophic.** `007` schedules the WC
decision (slice 2/3) and the console back-buffer (slice 4) separately, the
back-buffer motivated by `001`'s "console reads back uncached memory to
scroll". They are one decision: the back-buffer is the precondition that makes
WC safe to adopt, not a complement to it, because after WC the framebuffer must
never be read again. Adopt them together, and make "the console never reads the
framebuffer" an asserted property rather than a habit.

### 3.5 Heap allocator — the interface is the decision, the crate is not

Surveyed: `talc` (fast, no_std, aimed at kernels and WASM),
`good_memory_allocator` (a dlmalloc-style linked list, one `usize` overhead
per allocation, prioritising runtime efficiency while claiming good
utilisation), `slab_allocator_rs` (slabs by size class with a buddy above
4096), `linked_list_allocator` (the phil-opp default). The sources agree there
is no best design, because each kernel's workload is unique.

Asterinas answers with a boundary rather than a crate ◆: `Slab` and `HeapSlot`,
where the **type conversions are the safety-critical part** (unused page → slab
→ free slot → heap object), and a `Slab` tracks its live slot count so it
cannot be dropped under active objects. Policy (per-CPU free lists, size
classes) is injected safe code.

*Proposal:* `wost` owns the page→slab→slot conversions and their invariants;
the kernel-side policy is swappable. `007`'s hand-rolled versus vetted crate
question then becomes a revisable policy choice, the only form in which it is
worth an ADR, and the door stays open for `Allocator` when it stabilises (§1).

### 3.6 KASLR

The relocation work costs the same. Two things sharpen the timing: CET shadow
stacks and LASS (§6) both assume an address layout the kernel controls, and the
linker script accumulates hardcoded assumptions as the phase adds mappings. If
it is deferred again, the deferral should name what it will cost later.

---

## 4. Kernel design and infrastructure

### 4.1 The framekernel, extracted properly

The paper's API design, beyond the abstract `003` adopted ◆:

**OSTD's API in three categories**: safe user-kernel interaction (`UserMode`,
`UserContext`, `VmSpace`); safe kernel logic (`SpinLock`, `Rcu`, `Mutex`,
`WaitQueue`, `CpuLocal`, `LinkedList`, `RbTree`); safe kernel-peripheral
interaction (`IrqLine`, `IoMem`, `IoPort`, `DmaCoherent`, `DmaStream`). A
concrete target list for `wost`'s eventual surface.

**Sensitivity labelling is the pattern `wost` is missing.** OSTD labels I/O
memory and port ranges *sensitive* or *insensitive* from architecture
knowledge, firmware (ACPI tables) and core driver information, and
`IoMem`/`IoPort` instantiate only over insensitive regions. `wost::port::claim`
verifies exclusivity: the claim registry answers "may two drivers have this?",
sensitivity answers "may *anyone* have this?" They are complementary and `wost`
has only the first.

*Proposal:* when device mappings arrive in Phase 3 slice 5, `MmioRegion`
acquisition should consult a sensitivity list alongside the claim registry,
designed in then, per the Asterinas lesson `002` already adopted, rather than
retrofitted after the first driver maps something
it should not.

**A guard-page hazard the planned check will miss.** Asterinas pairs stack
guard pages with **compile-time stack usage analysis, specifically to prevent
bypass** ◆. A stack frame larger than the guard page can step over it and
corrupt what lies beyond without touching the guard, so nothing
faults. `007`'s planned check ("a guard page faults — checked structurally by
read-back") verifies the mapping exists; it cannot catch a frame that jumps
the gap. Either size guard regions against the
largest frame the kernel builds, or note the limitation in the slice.

**Their seven invariants**: kernel-mode CPU state protected from clients and
peripherals; sensitive memory isolated from clients, user programs and
devices; sensitive I/O memory and ports restricted to authorised access. `wost`
documents obligations per abstraction; a short invariant list at the crate
root would say what they collectively guarantee.

### 4.2 Klint — the bug class no lint here can currently catch

Klint performs compile-time **preemption-count analysis** ◆. Functions and
types carry `#[klint::preempt_count]` annotations declaring the *adjustment* a
call makes and the *expected range* at entry: `spin_lock` can be called from
any context but adjusts on return; sleepable functions adjust by zero and
require exactly zero. Analysis across call sites (monomorphising generics,
warning on indirect calls) then finds sleep-in-atomic-context.

It matters beyond deadlock: `synchronize_rcu()` is sound only if nothing sleeps
inside an RCU read side, or it can return early and free memory before
`rcu_read_unlock()`. That makes sleep-in-atomic-context **actual memory
unsafety, which Rust's type system alone does not prevent**.

**This is live here.** `003` adopted "no allocation in IRQ handlers" for the
Phase 5 executor. That rule *is* a preemption-count property, and it is
currently enforced by nobody. Phase 3 brings it forward: with the heap,
allocating under a spinlock or from an interrupt handler becomes possible.

Klint is a custom driver outside D-01's stable-only posture, but the *idea* is
expressible in safe stable Rust as a context token: a value only a sleepable
context can produce, threaded through APIs so `alloc`-taking functions require
one and IRQ handlers cannot obtain one. That is the
`SpinLock::force_unlock(PanicToken)` pattern `005` holds up as house style: an
obligation discharged by the type system rather than a comment. Worth an ADR
when the heap lands.

### 4.3 Pinned initialization — the problem Phase 4 will hit

Many kernel structures cannot change address because external pointers to them
exist: intrusive lists, wait queues, run queues. Pinning is Rust's answer, and
in-place constructors make pinned structures *initialisable* without first
building them on a stack that cannot hold them. Rust for Linux's `pin-init` is
`no_std` and the upstream kernel's answer. Phase 4 builds run queues and wait
queues; `002`'s `Pin` discipline for the executor is the same constraint
arriving one phase earlier, in a form easy to mistake for a design mistake
rather than a language limitation.

### 4.4 Scheduler datapoints for Phase 4

- **Redox replaced its scheduler with EEVDF in 2026** and reports average
  context switch falling from ~1.150 µs to ~250 ns.
- **Linux EEVDF mechanics:** each task accrues virtual runtime producing a
  *lag*; eligible tasks get a virtual deadline; earliest deadline runs.
  `sched_latency_ns` and `sched_min_granularity_ns` gave way to one
  `sched_base_slice_ns` when EEVDF merged in 6.6. Lag *decays* for sleeping
  tasks, because otherwise a task games the scheduler by sleeping briefly to
  clear negative lag, and long sleepers eventually have lag reset entirely.
  That anti-gaming rule is the non-obvious part and easy to omit.
- **sched_ext** demonstrates pluggable policy at EEVDF parity with
  crash-fallback. The lesson is not to clone it but that policy should stay
  **separable and observable, with a fallback path**: a constraint on the
  Phase 4 interface, free if designed in.

### 4.5 Forward-looks that update `002`

**The queue ABI's austerity rules, made concrete.** `003` established why
(FlexSC's batching/locality argument; io_uring's kernelCTF record). The rules
both passes converged on ⊕, for the Phase 5 queue-layout doc as a checklist:
versioned ring layouts; a minimal opcode set with no privileged opcodes; **a
rights check against the handle on every submission**, not at ring setup;
defensive re-read/copy of every user-ring entry before use (never trust a value
read twice from shared memory); per-queue quotas; and strong diagnostics on
rejection. The last two are the ones that get skipped and that turn a bug into
a DoS.

**WASI 0.3 is official as of 2026-06-11**, with completion-based async in the
Component Model, which `002` already targets; the ratification is now a fact
rather than a projection. But `002` pins **wasmi 1.0**, and wasmi is at 1.1.0,
and a specific minor version named three phases ahead is the kind of detail that
rots. *Proposal:* soften `002`'s wasmi reference to "wasmi 1.x, pinned when
Phase 7 starts".

**WizFS needs no change** ⊕: bcachefs-style key-versioned snapshots rather
than whole-tree cloning, checksums beside the pointers, journaled tree updates,
host fuzzing of the format crate, and BetrFS's write-optimized clones as
inspiration, explicitly *not* its full-path keying, which makes rename and
delete expensive.

---

## 5. Verification — newly cheap enough to be a real option

`003` listed "seL4 verification tooling (Kani/Verus)" as a gap.

- **Atmosphere** (SOSP '25, Mars Research): a verified microkernel in Rust with
  Verus, with functional correctness by refinement, verified memory
  allocation, verified intrusive data structures and page-table proofs.
  **Proof-to-code ratio 3.32:1**, with 6K lines of executable code verified in
  **under 2.5 person-years**, far cheaper than the seL4-era numbers. (`010` §9
  corrects the 7.5:1 figure inherited from the earlier KISV'23 workshop paper.)
- **Asterinas × CertiK** are verifying OSTD's **page tables** with Verus and
  found real bugs, including a race that could free a page-table node
  prematurely: the same structure, language and framework discipline.
- **The buddy allocator result** (§3.1).
- **Kani**: bounded model checking, adoptable incrementally; zero annotations
  already buys panic-freedom checking, and it runs 16,000+ harnesses per change
  in the Rust standard library verification campaign. Kani is limited to
  properties expressible in BMC or inductive contracts; Verus expresses richer
  invariants with ghost state at higher annotation cost.

*Proposal.* Kani fits this repo's culture with almost no new ceremony. `libs/`
crates are pure, `no_std`, host-testable arithmetic (page-table bit packing,
frame-index math, later ring indices and the WizFS format), BMC's strong suit:
a harness proves the *absence* of overflow and panics where host tests show a
few correct examples. It extends D-30 (encodings host-tested against
transcribed SDM tables) rather than competing with it. Verus should stay a
named option, not a plan, but if the frame allocator ever wants an invariant
proof, the precedent, the tooling and a published cost figure now exist.

---

## 6. Hardware security — the roadmap after W^X

`003` flagged CHERI/CET/PKS as an unresearched gap.

**CET shadow stacks.** Kernel-mode hardware stack protection ships in Windows;
Linux landed user-mode shadow stacks in 6.6; CET is present from 11th Gen Intel
Core onward. A kernel shadow stack is *per kernel stack*, so it is an
allocation decision, and Phase 3 slices 2 and 3 allocate and guard stacks.
Deciding the shape then costs nothing; retrofitting shadow stacks into an
existing stack allocator is materially worse. `000` has CET in Phase 12; the
stack allocator should not foreclose it.

**PKS (Protection Keys for Supervisor).** Domains of supervisor pages with
thread-specific restrictions, checked *in addition to* paging, and **changed
by an MSR write with no TLB flush**. Applied to §3.3: keep the direct map
non-writable by default and open a PKS window only while the kernel writes
through it, temporal protection over the largest writable mapping for one MSR
write. Availability is the catch: Sapphire Rapids / Xeon 6 class, absent from
typical consumer hardware, QEMU support unchecked. Posture: design the
abstraction so the direct map *can* be closed, gate on CPUID, and treat it as
free protection where present.

**LASS (Linear Address Space Separation).** Hardware user/kernel address
separation applied **before paging**, so software cannot derive paging-based
timing from TLBs or page walkers: it closes the side-channel class KPTI
addresses without KPTI's TLB cost. Slated for Linux 6.19; hardware from Sierra
Forest and Xeon 6, plus Core Ultra 200. Its discipline (never touch user
addresses except through sanctioned sequences) is the one SMAP already imposes
here, so most of the cost has been paid since Phase 2. Same posture as PKS:
CPUID-gated, cheap now, expensive if the address layout ignores it.

**CHERI.** Two industrial implementations (Arm's Morello, Microsoft's CHERIoT),
with UKRI's Digital Security by Design programme pushing CHERI-RISC-V toward
market. Hardware tags every pointer with bounds, permissions and a validity bit;
violations fault. CHERI is a memory-safety story *for C and C++*; a Rust
framekernel already has the property it is bought for, and `002` pillar 5 is the
software analogue. Treat it as a **future architecture port target** (`arch/`
exists for this), not a phase item. (2026 work on CheriBSD/Morello Linux
security analysis and on VM-porting pitfalls exists if that ever changes.)

---

## 7. `003`'s gaps — closure status

| Gap flagged in `003` | Status |
|---|---|
| CHERI / CET / PKS hardware security pass | **Closed** (§6) |
| seL4 verification tooling (Kani/Verus) | **Closed**, and the cost has dropped sharply (§5) |
| Redox retrospective | **Partial** — 2026 Redox covered (EEVDF numbers, `rustc`/`cargo` self-hosting on single- and multi-core since January 2026, NLnet/NGI Zero grant toward server workloads) |
| Managarm, SerenityOS retrospectives | **Not covered** — no pass made |
| Independent replication of the Asterinas numbers | **Still nothing found.** The 14.0% TCB figure and the LMbench/Nginx/Redis/SQLite ratios remain author-measured. `002` pillar 5 rests on a single unreplicated paper — worth keeping visible |
| Framekernel SMP scalability | **Still unquantified** — the paper defers multi-core and nothing has filled the gap |
| Memory tiering / folios | **Not pursued** — judged low relevance before Phase 6; recording the deliberate skip rather than leaving it as an open gap |
| The 14 Tier-2 claims in `003` | **Not re-verified.** This pass added breadth, not verification depth |

---

## 8. Proposed actions

Proposals, not decisions. Each becomes an ADR only if taken.

**Five ADRs are owed before slice 2 writes a page-table entry** ⊕, because each
changes an API later slices build on:

1. **Limine base-revision posture** (§3.0), the only item with a live risk;
   revision 3 is an inherited default.
2. **Frame allocator shape and the `wost`/`kernel` boundary** (§3.1):
   Safe Policy Injection, order-aware.
3. **The untyped position** (§3.2): reject seL4 retype, adopt Asterinas
   untyped memory as a type discipline, quotas at Phase 5's handle layer. The
   decision `007` says the phase must make ⊕.
4. **Direct-map contract** (§3.3), recording the SMP TLB-shootdown debt.
5. **Heap policy** (§3.5), decided as an *interface*.

**Then build `libs/x86-page-tables` first** — it was built, under the name
`libs/x86-paging` — with a host test for every bitfield, before `wost` is
allowed to switch CR3 ⊕. It is what D-30 prescribes, and §5's verification
options attach to it directly.

**Slice-level, as the slices arrive:**

6. **Merge the WC and back-buffer decisions**, asserting the console never
   reads the framebuffer (§3.4).
7. **Sensitivity labelling for `MmioRegion`** beside the claim registry D-39
   just sharpened, in slice 5 (§4.1).
8. **Guard-page sizing against the largest kernel stack frame**, or a note that
   the check cannot catch frame-jump bypass (§4.1).
9. **A context token for allocation-safety** in the `PanicToken` idiom when the
   heap lands (§4.2).
10. **Kani harnesses in the new page-table crate**, extending D-30 (§5).
    Verus stays a named option with a published cost figure, not a plan.
11. **A CET/PKS/LASS note in the stack and direct-map slices** (§6).

**Convention edits to `005`, independent of the phase:**

12. **The unsafe taxonomy** as `wost`'s admission test (§2.1) and **the
    sound-wrapper principle** as the standard for its exports, the UCG noted
    as advisory (§2.4).
13. **Split the C-EXAMPLE rejection** by crate kind (§2.3).

**Deferred, recorded so they are not rediscovered:**

14. **Naked functions** for Phase 4's context switch at minimum; decide
    separately whether to convert the existing `global_asm!` trampoline (§1).
15. **Soften `002`'s wasmi 1.0 pin** to "wasmi 1.x, pinned when Phase 7
    starts"; note WASI 0.3's ratification as fact (§4.5).
16. **Carry the queue-austerity checklist** into the Phase 5 queue-layout doc
    (§4.5).

---

## Sources

**Rust language, conventions, documentation**
- [Pragmatic Rust Guidelines — Safety](https://microsoft.github.io/rust-guidelines/guidelines/safety/index.html) and [Documentation](https://microsoft.github.io/rust-guidelines/guidelines/docs/)
- [Rust API Guidelines — Documentation](https://rust-lang.github.io/api-guidelines/documentation.html)
- [Edition Guide — unsafe_op_in_unsafe_fn](https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-op-in-unsafe-fn.html), [unsafe attributes](https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-attributes.html)
- [`doc_auto_cfg`](https://doc.rust-lang.org/stable/unstable-book/language-features/doc-auto-cfg.html); [RFC 3631 rustdoc cfg handling](https://rust-lang.github.io/rfcs/3631-rustdoc-cfgs-handling.html)
- [Intra-doc links in `no_std` crates (rust#76252)](https://github.com/rust-lang/rust/issues/76252)
- [Rustonomicon — Working with Unsafe](https://doc.rust-lang.org/nomicon/working-with-unsafe.html) (the sound-wrapper principle); [Unsafe Code Guidelines Reference](https://rust-lang.github.io/unsafe-code-guidelines/) (advisory, not normative)
- [`ptr_metadata` tracking issue (rust#81513)](https://github.com/rust-lang/rust/issues/81513) ✓
- [This Month in Rust OSDev: July 2026](https://rust-osdev.com/this-month/2026-07/) ◆, [April 2026](https://rust-osdev.com/this-month/2026-04/)
- [Tree Borrows (PLDI 2025)](https://iris-project.org/pdfs/2025-pldi-treeborrows.pdf); [ralfj.de: From Stacks to Trees](https://www.ralfj.de/blog/2023/06/02/tree-borrows.html)

**Framekernel and OSTD**
- [Asterinas, USENIX ATC '25 (arXiv HTML)](https://arxiv.org/html/2506.03876) ◆
- [The Asterinas Book — OSTD](https://asterinas.github.io/book/ostd/index.html) ◆; [`ostd::mm` on docs.rs](https://docs.rs/ostd/latest/ostd/mm/index.html) ◆
- [Kernel Memory Safety: Mission Accomplished](https://asterinas.github.io/2025/06/04/kernel-memory-safety-mission-accomplished.html) ◆

**Verification**
- [Atmosphere: Practical Verified Kernels with Rust and Verus (SOSP '25)](https://dl.acm.org/doi/10.1145/3731569.3764821); [project page](https://mars-research.github.io/projects/atmo/)
- [Formal Verification of a Rust-Based Buddy Physical Memory Allocator (TASE 2026)](https://link.springer.com/chapter/10.1007/978-3-032-30693-7_5)
- [Towards Practical Formal Verification for a General-Purpose OS in Rust](https://asterinas.github.io/2025/02/13/towards-practical-formal-verification-for-a-general-purpose-os-in-rust.html)
- [Kani: A Model Checker for Rust](https://arxiv.org/html/2607.01504); [Verus (SOSP '24)](https://dl.acm.org/doi/10.1145/3694715.3695952)

**Boot protocol**
- [Limine boot protocol, `limine-protocol` repo](https://github.com/limine-bootloader/limine-protocol/blob/trunk/PROTOCOL.md) ◆ — base revisions, HHDM coverage per revision, memory-map types
- `limine` crate 0.5.0 source (`BaseRevision::with_revision`) and 0.6.5 build failure on stable ✓ — checked locally, not cited

**Capabilities and the async data path** (reinforcing `003`, not superseding it)
- [Fuchsia kernel objects](https://fuchsia.dev/fuchsia-src/reference/kernel_objects/objects); [Zircon concepts](https://fuchsia.dev/fuchsia-src/concepts/kernel/concepts)
- [`io_uring` man page](https://man7.org/linux/man-pages/man7/io_uring.7.html); [`io_uring_enter(2)`](https://man7.org/linux/man-pages/man2/io_uring_enter.2.html); [kernelCTF rules](https://google.github.io/security-research/kernelctf/rules.html)
- [Bytecode Alliance — WASI 0.3](https://bytecodealliance.org/articles/WASI-0.3); [wasmi on docs.rs](https://docs.rs/wasmi)

**Memory management**
- [seL4 untyped tutorial](https://docs.sel4.systems/Tutorials/untyped.html); [seL4 Reference Manual](https://sel4.systems/Info/Docs/seL4-manual-latest.pdf)
- [Linux: Page Tables](https://docs.kernel.org/mm/page_tables.html); [Transparent Hugepage Support](https://docs.kernel.org/admin-guide/mm/transhuge.html)
- [Linux: MTRR control](https://docs.kernel.org/arch/x86/mtrr.html)
- [talc](https://github.com/SFBdragon/talc); [good_memory_allocator](https://docs.rs/good_memory_allocator); [slab_allocator_rs](https://lib.rs/crates/slab_allocator_rs)

**Kernel infrastructure and concurrency**
- [Klint: Compile-time Detection of Atomic Context Violations](https://www.memorysafety.org/blog/gary-guo-klint-rust-tools/) ◆
- [The Safe Pinned Initialization Problem](https://rust-for-linux.com/the-safe-pinned-initialization-problem); [pin-init](https://rust-for-linux.com/pin-init)
- [`kernel::sync`](https://rust-for-linux.github.io/docs/kernel/sync/index.html)
- [Linux EEVDF documentation](https://docs.kernel.org/scheduler/sched-eevdf.html); [Completing the EEVDF scheduler (LWN)](https://lwn.net/Articles/969062/); [Linux sched_ext documentation](https://docs.kernel.org/scheduler/sched-ext.html)
- [This Month in Redox — April 2026](https://www.redox-os.org/news/this-month-260430/), [January 2026](https://www.redox-os.org/news/this-month-260131/)

**Hardware security**
- [Linux: CET Shadow Stack](https://docs.kernel.org/arch/x86/shstk.html); [Windows kernel-mode hardware stack protection](https://learn.microsoft.com/en-us/windows-server/security/kernel-mode-hardware-stack-protection)
- [PKS: Add Protection Key Supervisor support (LWN)](https://lwn.net/Articles/850006/)
- [x86: Enable Linear Address Space Separation support (LWN)](https://lwn.net/Articles/1028881/); [Intel: What is LASS?](https://www.intel.com/content/www/us/en/support/articles/000098739/processors/intel-xeon-processors.html)
- [CHERI: Hardware-Enabled C/C++ Memory Protection at Scale (IEEE S&P)](https://www.cl.cam.ac.uk/research/security/ctsrd/pdfs/20240419-ieeesp-cheri-memory-safety.pdf); [CHERI project](https://www.cl.cam.ac.uk/research/security/ctsrd/cheri/)
