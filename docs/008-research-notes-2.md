# 008 — Deep Research II: Rust Practice, Kernel Design & the State of the Art

> **Historical pre-Phase-3 snapshot.** `010` re-checks this pass and records the
> current conclusions. In particular, it corrects Atmosphere's proof-to-code
> ratio, replaces the WASI target, and stages verification tooling differently.
> Normative choices live in `004-decision-log.md`.

Produced 2026-08-22, opening Phase 3. Where `003-research-notes.md` surveyed
architecture before the code existed, this pass is aimed: it closes the gaps
`003` flagged, and it arms the decisions `007-memory.md` holds open.

**Method and its honest limit.** Two independent research passes — one by
Claude, one by Codex — merged into one document. ~25 searches, 8 primary
documents fetched, every claim carrying its source. This is **not** the
adversarial multi-vote protocol that produced `003`'s Tier 1, but the two
passes were run without sight of each other, so where they agree the
agreement is worth something. Marks used below:

- **◆** — a primary source (paper, kernel doc, protocol spec, official book)
  was fetched directly rather than summarised from search results.
- **✓** — verified locally by compiling against this repo's pinned toolchain,
  or by reading the pinned dependency's own source.
- **⊕** — both passes reached this independently. Not proof, but the closest
  thing to confirmation this document has.

Everything unmarked is `003`'s Tier 2: sourced, plausible, cite-checked, and
to be re-read at the source before an ADR leans on it.

**Where the two passes agreed** ⊕ — recorded once here rather than repeated:
keep the framekernel as the core architecture (Asterinas/OSTD, ~14% TCB);
keep the API Guidelines + Rust for Linux convention base; keep capability
handles with rights-attenuating duplicate/replace and channels as the sole
delegation path; keep async submission/completion queues as the native data
path but keep them austere; keep EEVDF-inspired tickless scheduling with
policy kept separable; keep WizFS's bcachefs-style key-versioned snapshots
and checksums-beside-pointers. Phase 3's shape drew the same answer twice
as well: a host-tested page-table encoding crate first, typed frame
ownership, an explicit direct-map contract, W^X from the first owned CR3,
guard pages, zero-on-allocate, and structural selftests that walk the tables
after the switch. **None of the existing architecture needs to change.** The
findings below are additions, sharpenings, and one live risk.

---

## 1. The Rust you are actually allowed to use

Pinned at 1.97.1, edition 2024, stable-only (D-01). Several things have
landed since the conventions were written that change what is possible here.

Rows marked ✓ were compiled against this repo's pinned toolchain for
`x86_64-unknown-none` under edition 2024, not merely read about.

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
builds the interrupt trampoline with `global_asm!`. A `#[unsafe(naked)]` fn
carries a Rust signature, a symbol the type system knows about, `///` docs the
lints can enforce, and a `# Safety` section — all of which a `global_asm!`
string cannot have. The blob is currently outside every mechanism `005`
mandates. Phase 4's context switch is the natural place to adopt the pattern;
whether to convert the existing trampoline is a judgement call about churn.

**The allocator API is still not stable, and Phase 3 must design around that.**
A draft PR proposes stabilising the `Allocator` trait together with `Global`,
`System`, and the `*_in` constructors (`Box::new_in`, `Vec::new_in`), plus a
`GlobalAllocator` marker trait so `Allocator` implementors can be installed
globally — but it still needs FCP and has not landed. So Phase 3's heap goes
in through `#[global_allocator]` and the `alloc` crate, which is stable, and
the design should keep the *policy* separable so that adopting `Allocator`
later is a type change rather than a rewrite (§3.5).

**Edition-2024 items worth re-checking against the code.** `unsafe_op_in_unsafe_fn`
is already `deny` in the workspace lint table — ahead of the curve, since it
is only `warn`-by-default in the edition. Two others land in Phase 3's path:
unsafe attributes (`unsafe(no_mangle)`, `unsafe(export_name)`,
`unsafe(link_section)`) matter wherever the boot glue or linker script names a
symbol; and `static_mut_refs` means the per-frame metadata array (§3.1) cannot
be a `static mut` — it needs `wost::cell` or equivalent from the first line.

---

## 2. Conventions and documentation — proposed additions to `005`

### 2.1 A third standard worth adopting: Microsoft's Pragmatic Rust Guidelines

`005` names two authorities (Rust API Guidelines, Rust for Linux). A third has
appeared that is more specific than either about `unsafe` admission, and it
maps onto the `wost` boundary almost exactly.

Its distinctive contribution is a **closed taxonomy of valid reasons to write
`unsafe`**: novel abstractions (a new smart pointer or allocator), performance
(`get_unchecked`), and FFI/platform calls. Anything else is a defect. `005`
currently says "an operation that cannot be expressed safely", which is a
judgement; the taxonomy is a checklist a reviewer can apply. It also requires
plain-text reasoning for *calling* unsafe as well as for *providing* it — the
`// SAFETY:` / `# Safety` split `005` already took from Rust for Linux — and
explicitly recommends `unsafe_op_in_unsafe_fn`, already `deny` here.

*Proposal:* add the taxonomy to `005`'s "What may go in `wost`" section as the
admission test, alongside the existing no-speculative-API rule.

### 2.2 Rustdoc specifics `005` does not yet state

- **The first sentence is load-bearing** — it is the module listing and the
  search result. Convention: one sentence, ends in a period, rule of thumb
  ≤ 15 words. `005` says "a single sentence saying what the item does"; the
  length discipline is the part that is actually hard.
- **Section order** is conventional and expected: summary → body →
  `# Examples` → `# Errors` → `# Panics` → `# Safety`. `005` mandates the
  content of three of these but not the order.
- **`#[doc(hidden)]`** for items that are `pub` for technical reasons only.
  The support items `wost::entry!` needs in scope are the case here.
- **`#[doc(auto_cfg)]` is enabled at crate level by default** in current
  rustdoc: feature-gated items get their `cfg` badge without
  `#[doc(cfg(...))]`. This is free documentation for the `selftest` feature.
  `#[doc(auto_cfg = false)]` opts out.
- **The `no_std` intra-doc-link trap.** A crate that is `no_std` cannot
  resolve an intra-doc link to a `std` item; the workaround is
  `#[cfg(doc)] extern crate std;`. With `broken_intra_doc_links = "deny"` in
  the workspace and `cargo xtask doc` as a gate step, this will present as a
  CI failure whose cause is not obvious. Worth writing down before it bites.

### 2.3 One place `005` is more restrictive than it needs to be

`005` rejects C-EXAMPLE outright: "these crates are `no_std` and
cross-compiled, so doctests cannot run and unrunnable examples rot." That is
true of `wost` and `kernel`. It is **not** true of `libs/`, which carry
`#![cfg_attr(not(test), no_std)]` precisely so `cargo test` builds them for
the host — where doctests *do* compile and run, and are covered by the bare
`cargo test` that `default-members` already scopes.

*Proposal:* split the rule. Examples stay optional in `wost` and `kernel`;
in `libs/` they are encouraged, because there they are executable checks
rather than rot. This matters most for the crate Phase 3 is about to add:
page-table bit packing is exactly the kind of API where a three-line example
is the clearest possible statement of the encoding.

### 2.4 Which unsafe document is actually the authority

Worth settling before the page-table crate and the frame types are written,
because `wost` is about to grow its largest batch of new `unsafe` since the
split.

The **Unsafe Code Guidelines Reference** is the document people reach for and
is the wrong one to treat as binding: it describes itself as largely
historical and advisory, and it documents open questions as much as settled
rules. The practical anchor is the **Nomicon's sound-wrapper principle** —
safe code must not be able to trigger UB through your abstraction, no matter
how it is called. That is a single testable question to ask of every new
`wost` export, and it is exactly the standard `005` already applies in
practice when it says an abstraction must make the unsafe operation
*checkable*, not merely *wrapped*.

Two other aliasing-model notes, relevant because `MmioRegion` and the frame
types both hand out pointers into memory something else can change:
**Tree Borrows** has replaced Stacked Borrows as the working aliasing model
(it fixes overeager uniqueness for `&mut` and the confinement of pointers to
the size of the type they were created with — Stacked Borrows rejected ~54%
more test code). Neither model is normative Rust yet, and neither can be run
against this kernel (Miri cannot execute a cross-compiled `no_std` binary) —
so the discipline has to come from the API shape, which is the argument for
the untyped-memory typing in §3.2 rather than a reason to reach for `Miri`.

*Proposal:* name the Nomicon's sound-wrapper principle in `005` as the
standard `wost` exports are held to, and note the UCG's advisory status so
nobody cites it as a rule later.

---

## 3. Phase 3, decision by decision

### 3.0 The Limine base revision — the live risk, and it is cheaper to fix than it looks

This is the one finding that changes a *premise* rather than sharpening a
choice, and it lands squarely on slice 1.

**The protocol has moved to base revision 6, and revisions 0–5 are marked
deprecated** ◆. The revision history matters far more than the deprecation
notice does:

| Revision | What it changes, relative to 3 |
|---|---|
| 3 (current pin) | HHDM covers **only** Usable, Bootloader-reclaimable, Executable-and-modules, Framebuffer |
| **4** | HHDM **adds Reserved (Mapped), ACPI reclaimable, and ACPI NVS**; introduces the `RESERVED_MAPPED` memory-map type; guarantees ACPI tables are mapped |
| 5 | SMBIOS mapped as `RESERVED_MAPPED`; stricter x86-64 control-register handoff; additional I/O APIC entries masked; IOMMUs disabled; local APIC initialised to a defined state on every processor |
| 6 | Drops identity mapping; aarch64/RISC-V/loongarch register-state guarantees (nothing x86-64-relevant) |

**Base revision 4 is the answer to D-33.** `wost/src/boot/mod.rs:146` currently
carries this comment on `BootInfo`'s deleted direct map: *"under base revision
3 it covers only the regions the memory map lists — not the device holes — so
nothing in this phase could use it."* That sentence is a statement of a base
revision 3 limitation, and base revision 4 is precisely the revision that
lifts it. It also hands slice 5 something it would otherwise have to build:
the I/O APIC is discovered through the MADT, which means reading ACPI tables,
which base revision 4 guarantees are mapped.

**And the crate pin does not block it** ✓. The obvious reading — that this is
gated behind the `limine` 0.6.x upgrade `001-boot.md` lists as an open item —
is wrong, and worth stating plainly because it would be easy to defer on. Two
checks against the pinned dependency:

- `limine` 0.6.5 still carries `#![feature(ptr_metadata)]` and **fails to
  compile on stable 1.97.1** ✓ (confirmed by building it; `ptr_metadata` is
  still unstable, rust#81513 ✓). The pin to `=0.5.0` remains correct, and
  `001-boot.md`'s open item stands as written.
- But the base revision is a *protocol* parameter, not a crate version. The
  pinned 0.5.0 crate exposes `BaseRevision::with_revision(u64)` as a public
  `const fn` ✓ — `BaseRevision::new()` is merely `with_revision(3)`. The tag
  is three `u64`s in a static.

So requesting base revision 4 is a one-argument change at
`wost/src/boot/mod.rs:195`, inside the single file `005` designates as the
whole of the bootloader dependency. The crate-version blocker and the
protocol-revision choice are independent, and only the first is blocked.

*Caveats to carry into the slice, not reasons to defer:*

- The **vendored Limine v10 binary** must actually support the revision asked
  for. `is_supported()` is already checked and logged (`base_revision_supported`
  in `BootInfo`), so this fails loudly rather than silently — but it is a boot
  test, not a literature question, and should be the first thing slice 1 does.
- The 0.5.0 crate does not know the `RESERVED_MAPPED` memory-map type
  constant. Memory-map entry-type handling must therefore treat unknown types
  defensively — which is the `#[non_exhaustive]` discipline `005` already
  mandates for error enums, applied to a foreign enum.
- Even with device holes HHDM-covered, Phase 3 should still map device MMIO
  itself through `MmioRegion` with an explicit memory type rather than reach
  through the bootloader's mapping. Base revision 4 removes the wall; it does
  not remove the reason for the door. This is consistent with §3.3's proposed
  direct-map contract, which excludes device holes deliberately.

*Proposal:* make the base-revision posture an explicit ADR rather than an
inherited default — including the decision **not** to go past 4 or 5 if that
is the call, and why. Right now the repo requests revision 3 because
`BaseRevision::new()` happens to default to it, which is not a decision.

### 3.1 Frame allocator shape — and the `wost`/`kernel` boundary

`007` asks how much allocator logic lives kernel-side as safe code. Asterinas
answers it directly, and the answer has a name: **Safe Policy Injection** ◆.

The framework defines the *mechanism* and the invariant; the *policy* is
injected as safe Rust outside the TCB. Asterinas has exactly three injection
points — task scheduler (`Scheduler` trait), frame allocator (`FrameAlloc`
trait), and slab allocator (`Slab`/`HeapSlot`). The framework retains only the
invariant-enforcing conversion, `Frame::from_unused()`, which is what makes
their Inv. 1 hold: *any newly-allocated `Frame` or `Segment` originates from
currently unused memory*. Everything above it — buddy trees, per-CPU free
lists, sizing heuristics — is ordinary safe Rust that cannot violate the
invariant because it can only speak in `Frame`s.

Their frame system is worth copying in shape: `Frame<M>` (one page) and
`Segment<M>` (contiguous run), both generic over per-frame metadata, with all
metadata living in **one large static array sized at bootstrap**, holding
refcounts; drop to zero returns the frame to the allocator ◆.

*Proposal for the ADR:* `wost` owns `Paddr`, `Frame`, `Segment`, the metadata
array, and the `from_unused` conversion; `kernel` owns the allocator policy as
safe Rust behind a `wost`-defined trait. This is the same boundary `wost`
already draws with `port::claim` — the framework verifies exclusivity, the
caller chooses what to claim.

**Bitmap versus buddy — and why the phase forces the answer.** A bitmap is one
bit per frame (32 KiB per GiB of RAM) and linear to search. A buddy handles
contiguous multi-frame allocation. The decision looks deferrable until it
isn't: the direct map wants 2 MiB pages (§3.3), and DMA buffers in Phase 8
want physically contiguous runs. A single-frame bitmap will be outgrown
*inside Phase 3*, not later. Staged (bump-until-heap, then real) remains the
right shape, but the "real" stage should be order-aware from the start.

**And there is now a verified one to read first.** *Formal Verification of a
Rust-Based Buddy Physical Memory Allocator* (TASE 2026) verifies UtManager, a
bitmap-based buddy allocator, in Verus: state invariants and function
specifications proving allocation and free preserve the invariants and satisfy
their postconditions, with bounds safety, overflow freedom, and termination —
and the verification **found a previously unknown control-flow defect** in the
original. Zero runtime overhead. This is the closest published thing to what
Phase 3 is about to write.

### 3.2 Untyped-retype — the evidence says reject the mechanism, adopt the discipline

Two different ideas share the word "untyped", and `002`'s open question
conflates them. Separating them resolves it.

**seL4's untyped/retype (the accounting mechanism) has structural costs.**
Each untyped capability carries a *single watermark*; retype moves it first to
the new object's alignment and then past its size. Create a 4 KiB object then
a 16 KiB one and the 12 KiB of alignment padding between them is
unreclaimable until *both* children are revoked. The community critique goes
further: fixing the allocation policy inside the kernel contradicts seL4's own
principle of keeping policy out of the kernel — the kernel will refuse a gap
that would satisfy the alignment, because it cannot express using it. Add
`003`'s already-recorded finding that revocation is long-running and needs
abortable sub-operations for "incremental consistency", and the mechanism is
expensive in three separate ways.

**Asterinas's untyped memory (the type discipline) is a different thing and it
carries.** ◆ Its premise: safe Rust references cannot uphold their guarantees
for *externally modifiable* memory — user-mapped, DMA-capable, or MMIO. So
such memory gets `UFrame`/`USegment`, a read/write interface permitting only
copies of plain-old-data types. The consequences are the interesting part:

- `VmSpace` (the user address space) accepts **only** `UFrame`/`USegment` —
  which is what structurally prevents mapping kernel memory into userspace.
  Not a check; a signature.
- `DmaCoherent` and `DmaStream` likewise accept only untyped memory. Initially
  *no* memory is DMA-accessible; drivers request mappings only over untyped.

*Proposed position for the ADR:* **reject seL4-style untyped-retype as the
kernel-memory accounting mechanism; adopt Asterinas-style untyped memory as a
type discipline in `wost` from the first frame type.** Kernel-memory DoS is
then answered at the handle layer in Phase 5 by per-process quota accounting —
a policy decision, revisable — rather than by making every caller donate
memory, which is an ABI decision and permanent. Record the naming collision so
this is not re-litigated in three phases.

The cost of getting the discipline right early is near zero; the cost of
retrofitting it is the whole point of `002`'s pillar 5 and Asterinas's own
stated lesson.

### 3.3 Table access — direct map, with an explicit contract

The direct map with 2 MiB pages is the modern standard, and the numbers are
stark: Linux maps 32 GiB of physical memory in **132 KiB of page tables**
using 2 MiB pages, with each TLB entry covering 2 MiB instead of 4 KiB.
Recursive mapping costs an address-space slot and awkward invariants for a
benefit the HHDM already hands over.

*Proposal for the ADR's contract:* the direct map covers **usable RAM only**,
NX and writable, 2 MiB pages wherever alignment permits, 4 KiB at the edges.
Device holes are deliberately *not* in it — they are mapped separately through
`MmioRegion` with their own memory type (§3.4), which is also what makes the
sensitivity discipline in §4.1 expressible.

*Debt to record now:* a direct map means that unmapping anything from it
requires a TLB shootdown once SMP arrives — an IPI to every CPU. Single-core
makes this free today. Writing it down now is cheaper than discovering it in
Phase 12.

### 3.4 Framebuffer memory type — and why it is the same decision as the back-buffer

Write-combining is not a marginal optimisation. Linux's own MTRR documentation
puts it at "2.5 times or more" for image writes; field reports on framebuffer
redraw run far higher. PAT supersedes MTRR on modern x86 and Linux's guidance
is to use both only because non-PAT systems still exist — for a UEFI-only
target running under QEMU and VMware, PAT is universally present and the MTRR
half is dead weight.

**The finding that links two slices:** write-combining makes writes fast and
reads catastrophic. `007` currently schedules the WC decision (slice 2/3) and
the console back-buffer (slice 4) separately, with the back-buffer motivated
by `001`'s "console reads back uncached memory to scroll". They are one
decision. The back-buffer is not a nicety that WC happens to complement — it
is the precondition that makes WC safe to adopt at all, because after WC the
framebuffer must never be read again. Adopt them together, and make "the
console never reads the framebuffer" an asserted property rather than a habit.

### 3.5 Heap allocator — the interface is the decision, the crate is not

Surveyed candidates: `talc` (fast, flexible, no_std, actively
performance-tuned, aimed at kernels and WASM), `good_memory_allocator`
(dlmalloc-lineage linked list, one `usize` overhead per allocation,
prioritising runtime efficiency while claiming good utilisation),
`slab_allocator_rs` (slabs by size class with a buddy above 4096),
`linked_list_allocator` (the phil-opp default). The consensus in the sources
is unhelpfully honest: there is no best design, because each kernel's workload
is unique.

Asterinas again answers with a boundary rather than a crate ◆: `Slab` and
`HeapSlot`, where the **type conversions are the safety-critical part** —
unused page → slab → free slot → heap object — and a `Slab` tracks its live
slot count so it cannot be dropped out from under active objects. Policy
(per-CPU free lists, size classes) is injected safe code.

*Proposal:* make `wost` own the page→slab→slot conversions and their
invariants, and let the kernel-side policy be swappable. Then the dependency
question `007` poses — hand-rolled versus vetted crate — stops being
architectural and becomes a revisable policy choice, which is the only form in
which it is worth an ADR. It also leaves the door open for the `Allocator`
trait when it stabilises (§1).

### 3.6 KASLR

Nothing in this pass changes the cost of the relocation work. Two things
sharpen the timing argument: both CET shadow stacks and LASS (§6) assume an
address layout the kernel controls, and the linker script only accumulates
hardcoded assumptions as the phase adds mappings. If it is deferred again, the
deferral should name what it will cost later, not just that it was deferred.

---

## 4. Kernel design and infrastructure

### 4.1 The framekernel, extracted properly

`003` adopted the framekernel from the abstract. The paper's API design has
detail worth having ◆:

**OSTD's API in three categories** — safe user-kernel interaction (`UserMode`,
`UserContext`, `VmSpace`); safe kernel logic (`SpinLock`, `Rcu`, `Mutex`,
`WaitQueue`, `CpuLocal`, plus `LinkedList` and `RbTree`); safe kernel-peripheral
interaction (`IrqLine`, `IoMem`, `IoPort`, `DmaCoherent`, `DmaStream`). That
is a concrete target list for `wost`'s eventual surface.

**Sensitivity labelling is the pattern `wost` is missing.** OSTD labels I/O
memory and port ranges *sensitive* or *insensitive*, deriving the labels from
architecture knowledge, firmware (ACPI tables), and core driver information —
and `IoMem`/`IoPort` will only instantiate over insensitive regions. Compare
`wost::port::claim`, which records ownership so exclusivity is *verified*: the
claim registry answers "may two drivers have this?", sensitivity answers "may
*anyone* have this?" They are complementary and `wost` has only the first.

*Proposal:* when device mappings arrive in Phase 3 slice 5, `MmioRegion`
acquisition should consult a sensitivity list alongside the claim registry —
designed in then, per the Asterinas lesson `002` already adopted, rather than
retrofitted when the first driver maps something it should not have.

**A guard-page hazard the planned check will miss.** Asterinas pairs stack
guard pages with **compile-time stack usage analysis, specifically to prevent
bypass** ◆. A stack frame larger than the guard page can step clean over it
and corrupt whatever lies beyond — the guard page is never touched, so nothing
faults. `007`'s planned check ("a guard page faults — checked structurally by
read-back") verifies the mapping exists; it cannot catch a frame that jumps
the gap. Worth either sizing guard regions against the largest frame the
kernel actually builds, or noting the limitation explicitly in the slice.

**Their seven invariants** are stated as: kernel-mode CPU state protected from
clients and from peripherals; sensitive memory isolated from clients, user
programs, and devices; sensitive I/O memory and ports restricted to authorised
access. The framing is worth stealing — `wost` currently documents obligations
per-abstraction, and a short invariant list at the crate root would say what
the abstractions collectively guarantee.

### 4.2 Klint — the bug class no lint here can currently catch

Klint performs compile-time **preemption-count analysis** ◆. Functions and
types carry `#[klint::preempt_count]` annotations declaring two things: the
*adjustment* a call makes to the preemption count, and the *expected range* at
entry. `spin_lock` can be called from any context but adjusts on return;
sleepable functions adjust by zero and require exactly zero. Static analysis
across call sites — monomorphising generics, warning on indirect calls through
fn pointers and trait objects — then finds sleep-in-atomic-context.

Why it matters beyond deadlock: the soundness of `synchronize_rcu()` depends
on no sleeping inside an RCU read side. If one happens, `synchronize_rcu()` can
return early and free memory before `rcu_read_unlock()`. That lifts
sleep-in-atomic-context from a liveness bug to **actual memory unsafety —
which Rust's type system alone does not prevent**.

**This is live here, sooner than it looks.** `003` already adopted the rule "no
allocation in IRQ handlers" for the Phase 5 executor. That rule *is* a
preemption-count property, and it is currently enforced by nobody. Phase 3
brings it forward: the heap arrives, and allocating under a spinlock or from
an interrupt handler becomes possible for the first time. Phase 2's IRQ-safe
locking already established the shape of the problem.

Klint itself is a custom driver and sits outside the stable-only posture of
D-01. But the *idea* is expressible in safe stable Rust: a context token —
a marker value threaded through APIs that only a sleepable context can
produce, so `alloc`-taking functions require one and IRQ handlers cannot
obtain one. That is precisely the `SpinLock::force_unlock(PanicToken)` pattern
`005` already holds up as the house style: an obligation discharged by the
type system rather than by a comment. Worth an ADR when the heap lands.

### 4.3 Pinned initialization — the problem Phase 4 will hit

Rust for Linux's `pin-init` exists because of a problem that has no safe
expression otherwise: many kernel data structures cannot change address, since
external pointers to them already exist and would be invalidated. Intrusive
lists, wait queues, run queues. Pinning is Rust's answer, and in-place
constructors are what make pinned structures *initialisable* without first
building them on a stack that cannot hold them.

Phase 4 builds run queues and wait queues. Knowing that `pin-init` exists,
is `no_std`, and is the upstream kernel's answer is much cheaper than
rediscovering the safe-pinned-initialization problem halfway through a
scheduler. `002` already notes `Pin` discipline for the executor; this is the
same constraint arriving one phase earlier and in a form that is easy to
mistake for a design mistake rather than a language limitation.

### 4.4 Scheduler datapoints for Phase 4

- **Redox replaced its scheduler with EEVDF in 2026** and reports average
  context switch falling from ~1.150 µs to ~250 ns. A Rust-microkernel
  datapoint on the same design `000` already commits to.
- **Linux EEVDF mechanics:** each task accrues virtual runtime producing a
  *lag*; eligible tasks get a virtual deadline; earliest deadline runs.
  `sched_latency_ns` and `sched_min_granularity_ns` were deprecated in favour
  of a single `sched_base_slice_ns` when EEVDF merged in 6.6. Lag *decays* for
  sleeping tasks — otherwise a task games the scheduler by sleeping briefly to
  clear negative lag, and long sleepers eventually have lag reset entirely.
  That anti-gaming rule is the non-obvious part and is easy to omit.
- **sched_ext** continues to demonstrate pluggable policy at EEVDF parity with
  crash-fallback. The lesson to take is not to clone it: it is that policy
  should stay **separable and observable, with a fallback path**. That is a
  constraint on the Phase 4 interface, and it is free if designed in.

### 4.5 Forward-looks that update `002`

Not Phase 3's business, but `002` states things that have since moved. Cheap
to correct now, expensive to discover mid-phase.

**The queue ABI's austerity rules can be made concrete.** `003` established
*why* (FlexSC's batching/locality argument; io_uring's kernelCTF record as the
warning). The operational rules both passes converged on ⊕, worth carrying
into the Phase 5 queue-layout doc as a checklist rather than a principle:
versioned ring layouts; a minimal opcode set with no privileged opcodes;
**a rights check against the handle on every submission**, not at ring setup;
defensive re-read/copy of every user-ring entry before use (never trust a
value read twice from shared memory); per-queue quotas; and strong diagnostics
on rejection. The last two are the ones that get skipped and are the ones that
turn a bug into a DoS.

**WASI 0.3 is official as of 2026-06-11**, with completion-based async in the
Component Model — `002` already targets it, and the ratification is now a
fact rather than a projection. But `002` pins **wasmi 1.0**, and wasmi is at
1.1.0. The pin should be re-evaluated when Phase 7 arrives rather than baked
in now; naming a specific minor version in an architecture doc three phases
ahead is the kind of detail that rots. *Proposal:* soften `002`'s wasmi
reference to "wasmi 1.x, pinned when Phase 7 starts".

**WizFS needs no change** ⊕. Both passes independently reached the same
reading of the same evidence: bcachefs-style key-versioned snapshots rather
than whole-tree cloning, checksums beside the pointers, journaled tree
updates, host fuzzing of the format crate, and BetrFS's write-optimized clone
ideas as inspiration — explicitly *not* its full-path keying, which makes
rename and delete expensive. `002` already says all of this.

---

## 5. Verification — newly cheap enough to be a real option

`003` listed "seL4 verification tooling (Kani/Verus)" as a gap. It has moved a
lot, and two of the results are about *exactly* the subsystem Phase 3 builds.

- **Atmosphere** (SOSP '25, Mars Research) — a full verified microkernel in
  Rust with Verus: functional correctness by refinement from a high-level
  specification, verified memory allocation, verified intrusive data
  structures, and proofs about page tables. **Proof-to-code ratio 3.32:1**, with
  6K lines of executable code verified in **under 2.5 person-years**. The
  ratio is the headline — far cheaper than the seL4-era numbers that made
  verification a research-programme-sized commitment. (`010` §9 corrects the
  7.5:1 figure inherited from the earlier KISV'23 workshop paper.)
- **Asterinas × CertiK** are verifying OSTD's **page tables** with Verus, and
  the work found real bugs — including a race that could free a page-table
  node prematurely. The most directly relevant precedent available: the same
  structure, the same language, the same framework discipline.
- **The buddy allocator result** (§3.1) — Verus, bitmap buddy, invariants plus
  bounds safety, overflow freedom, and termination; found a control-flow
  defect; zero runtime overhead.
- **Kani** — bounded model checking, adoptable incrementally: zero annotations
  already buys panic-freedom checking. Running at 16,000+ harnesses per change
  in the Rust standard library verification campaign. The trade against Verus
  is explicit in the sources: Kani is limited to properties expressible in BMC
  or inductive contracts; Verus expresses richer invariants with ghost state,
  at higher annotation cost.

*Proposal.* Kani is the tool that fits this repo's existing culture with
almost no new ceremony. `libs/` crates are already pure, `no_std`,
host-testable arithmetic — page-table bit packing, frame-index math, and later
ring-index arithmetic and the WizFS format. Those are exactly BMC's strong
suit, and a Kani harness proves the *absence* of overflow and panics where the
existing host tests demonstrate the presence of a few correct examples. It
extends D-30 (encodings host-tested against transcribed SDM tables) rather
than competing with it.

Verus is the heavier commitment and should stay a named option, not a plan —
but Atmosphere and the TASE result together mean that if the frame allocator
ever wants a real invariant proof, the precedent, the tooling, and a published
cost figure all now exist.

---

## 6. Hardware security — the roadmap after W^X

`003` flagged CHERI/CET/PKS as an unresearched gap. Closing it, with the
posture each deserves.

**CET shadow stacks.** Kernel-mode hardware-enforced stack protection ships in
Windows; Linux landed user-mode shadow stacks in 6.6; CET is present from 11th
Gen Intel Core onward. The design consequence is specific: a kernel shadow
stack is *per kernel stack*, which makes it an allocation decision. Phase 3
slices 2 and 3 are where stacks get allocated and guard-paged. Deciding the
shape then costs nothing; retrofitting shadow stacks into an existing stack
allocator is materially worse. `000` already has CET in Phase 12 — worth
carrying a note into Phase 3 that the stack allocator should not foreclose it.

**PKS (Protection Keys for Supervisor).** Domains of supervisor pages with
thread-specific permission restrictions, checked *in addition to* normal
paging protections, and — the interesting part — **changed by an MSR write
with no TLB flush**. The natural application here is directly on §3.3's
decision: keep the direct map non-writable by default and open a PKS window
only for the moments the kernel actually writes through it. That is temporal
protection over the single largest writable mapping in the kernel, at a cost
of one MSR write. Availability is the catch — Sapphire Rapids / Xeon 6 class,
absent from typical consumer hardware, and QEMU support needs checking. The
posture is: design the abstraction so the direct map *can* be closed, gate the
enable on CPUID, and treat it as free protection where the hardware offers it.

**LASS (Linear Address Space Separation).** Hardware enforcement of user/kernel
address separation applied **before paging**, so software cannot derive
paging-based timing information from TLBs or page walkers — i.e. it closes the
class of side channels KPTI addresses, without KPTI's TLB cost. Slated for
Linux 6.19; hardware from Sierra Forest and Xeon 6, plus Core Ultra 200
series. The discipline it imposes on the kernel — never touch user addresses
except through explicit sanctioned sequences — is the same one SMAP already
imposes here, so most of the cost has been paid since Phase 2. Same posture as
PKS: CPUID-gated, cheap now, expensive if the address layout is chosen without
it in mind.

**CHERI.** Two industrial implementations exist — Arm's Morello and
Microsoft's CHERIoT — with UKRI's Digital Security by Design programme pushing
CHERI-RISC-V toward market across several processor scales. Hardware tags
every pointer with bounds, permissions, and a validity bit; violations fault.
The honest relevance assessment: CHERI is a memory-safety story *for C and
C++*. A Rust framekernel already has the property CHERI is bought to provide,
and `002` pillar 5 is the software analogue of the same idea. The right
treatment is as a **future architecture port target** — `arch/` exists for
exactly this — and not as a phase item. (2026 work on CheriBSD/Morello Linux
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

**Five ADRs are owed before slice 2 writes a page-table entry.** Both passes
landed on the same list ⊕, and the ordering argument is that each one changes
an API that later slices are built on:

1. **Limine base-revision posture** (§3.0) — the only item with a live risk
   attached, and the only one whose answer might change slice 1's shape.
   Currently revision 3 by inherited default, not by decision.
2. **Frame allocator shape and the `wost`/`kernel` boundary** (§3.1) —
   Safe Policy Injection: `wost` owns `Frame`/`Segment`, the metadata array,
   and `from_unused`; `kernel` owns policy behind a `wost` trait. Order-aware,
   not single-frame bitmap, because the direct map's 2 MiB pages force
   contiguous allocation inside this phase.
3. **The untyped position** (§3.2) — reject seL4 untyped-retype as the
   accounting mechanism; adopt Asterinas untyped memory as a type discipline
   from the first frame type; defer kernel-memory quotas to Phase 5's handle
   layer. This is the decision `007` says the phase must make, and both passes
   reached the same adaptation independently ⊕.
4. **Direct-map contract** (§3.3) — usable RAM only, NX, 2 MiB where aligned;
   device holes deliberately excluded and mapped through `MmioRegion` with
   explicit memory types. Record the SMP TLB-shootdown debt.
5. **Heap policy** (§3.5) — decided as an *interface* (`wost` owns the
   page→slab→slot conversions; policy is swappable kernel-side), which turns
   hand-rolled-vs-crate into a revisable choice rather than an architectural
   one, and leaves the door open for `Allocator` when it stabilises.

**Then build `libs/x86-page-tables` first**, with a host test for every
bitfield, before `wost` is allowed to switch CR3 ⊕. That is the cleanest
research-into-code bridge available, it is what D-30 already prescribes, and
§5's verification options attach to it directly.

**Slice-level, as the slices arrive:**

6. **Merge the WC and back-buffer decisions** into one slice, and assert that
   the console never reads the framebuffer (§3.4).
7. **Sensitivity labelling for `MmioRegion`**, alongside the claim registry
   D-39 just sharpened, when device mappings land in slice 5 (§4.1).
8. **Guard-page sizing against the largest kernel stack frame**, or an explicit
   note that the structural check cannot catch frame-jump bypass (§4.1).
9. **A context token for allocation-safety**, in the `PanicToken` idiom, when
   the heap lands — the safe-stable expression of klint's preemption-count
   property (§4.2).
10. **Kani harnesses in the new page-table crate**, extending D-30 (§5).
    Verus stays a named option with a published cost figure, not a plan.
11. **Carry a CET/PKS/LASS note into the stack and direct-map slices** so the
    layout does not foreclose them (§6).

**Convention edits to `005`, cheap and independent of the phase:**

12. **The Pragmatic Rust Guidelines' unsafe taxonomy** as `wost`'s admission
    test (§2.1), and **the Nomicon's sound-wrapper principle** named as the
    standard `wost` exports are held to, with the UCG's advisory status noted
    so nobody cites it as a rule later (§2.4).
13. **Split the C-EXAMPLE rejection**: keep it for `wost`/`kernel`, encourage
    doctests in `libs/` where they actually execute (§2.3). Cheapest item
    here, and it applies to the crate Phase 3 is about to create.

**Deferred, recorded so they are not rediscovered:**

14. **Naked functions** for Phase 4's context switch at minimum; decide
    separately whether to convert the existing `global_asm!` trampoline (§1).
15. **Soften `002`'s wasmi 1.0 pin** to "wasmi 1.x, pinned when Phase 7
    starts"; note WASI 0.3's ratification as fact (§4.5).
16. **Carry the queue-austerity checklist** — per-submission rights checks,
    defensive re-reads, per-queue quotas, rejection diagnostics — into the
    Phase 5 queue-layout doc (§4.5).

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
