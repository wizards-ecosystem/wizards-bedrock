# D-214 Hardware-retained pointers need irrevocable mappings, and unsafe review needs an exact revision

Status: Accepted, 2026-08-28. Amends D-41, D-125 and D-139; closes PW-506.

**Context.** The final `wost` review covered 169 unsafe block expressions and
38 unsafe declarations across eighteen files. The 368-line measurement stayed
below the unchanged 380-line budget, but a total alone did not say which
revision of the surrounding safe wrappers and `// SAFETY:` grounds had been
read. Adding one unsafe block while deleting another could preserve both the
budget and the measured total.

The adversarial caller pass found two safe doors whose sole current callers
were careful but whose types were not. `set_double_fault_stack` accepted a
`MappedStack`, checked the mapping once and stored its top in `TSS.IST1`.
Afterward `AddressSpace::unmap` could remove that exact owned range while the
processor retained its address. D-125 fixed evidence that was stale *before*
installation; it did not keep the installed mapping alive afterward. A later
double fault would fault while constructing its own frame and reset the
machine.

`open_fast_syscall_doors_for_test` returned with `EFER.SCE` set and relied on
the kernel self-test to call the matching closer immediately. D-139 checked
every normal ring-3 entry, but a safe public operation should not rely on an
arbitrary future caller preserving a hazardous interval, especially if it is
ever reached from a trap path that resumes an existing program.

**Decision.** `AddressSpace::map_pinned_stack` maps the guarded stack and the
paging module publishes its checked root, base and length before returning
`PinnedStack`. It is available only for the address space this CPU is currently
walking, which prevents an old token becoming valid again after a reclaimed
physical root is reused. Keeping the one hardware pin beside the one global
active-root record also leaves `AddressSpace`'s size unchanged; debug builds
had no stack headroom for another inline extent. `unmap` refuses every range
that overlaps the recorded half-open stack extent, including a strict subrange
or a request extending across either edge.
`segment::set_double_fault_stack` now accepts only `PinnedStack`. The active
space itself cannot be reclaimed, so safe code has no operation that can
invalidate the address retained by the TSS.

The overlap arithmetic lives in `libs/stack-layout::Slot::stack_overlaps`, is
host tested at both adjacent edges, inside the extent and on overflow, and
fails closed. This does not implement PW-703: ordinary `MappedStack` evidence
still re-walks only its top page, and complete dynamic-stack validation remains
stopped until stacks can actually grow or shrink.

The fast-syscall probe is one operation now. It excludes interrupts, records
the initially shut state, opens `SCE`, observes it, runs the private closing
path, reads back both doors and restores the prior interrupt state. No safe
caller can return, forget a guard or resume ring 3 during the open interval.

`xtask::unsafety` now parses Rust syntax for the line budget instead of looking
for the word `unsafe`; comments, strings and unsafe declarations cannot be
mistaken for block expressions. Beside the budget it records the normalized
SHA-256, syntactic unsafe-block count and unsafe-declaration count of every
unsafe-bearing `wost` source. The complete file is fingerprinted deliberately:
changing a safe wrapper or the prose that grounds its operation requires the
same explicit review as changing the block.

**Rationale.** Hardware does not retain Rust lifetimes. Once a pointer is
copied into a TSS, IDT, DMA descriptor or similar table, validating it at the
store is insufficient; the mapping must be non-revocable for at least the
hardware lifetime. A distinct token makes that stronger fact visible at the
only setter, while the address-space record makes it true after the token is
dropped.

Per-file fingerprints are intentionally coarser than per-block hashes. The
sound-wrapper rule is about every safe path surrounding an unsafe operation,
and a narrow block hash would miss a removed bound or widened public
constructor elsewhere in the same file. Eighteen reviewed records are a small
cost for making such drift a gate failure.

**Alternatives.** Re-walking `IST1` before a double fault is impossible: the
processor selects the stack before ordinary handler code runs. Consuming a
`MappedStack` at the setter would remove one Rust value but would not stop a
separate safe `unmap` call. Leaking the frames without recording the virtual
extent would keep physical memory allocated while still allowing the live PTE
to disappear. A `Drop`-based syscall transition was rejected because safe
`mem::forget` can skip `Drop`; a closed function leaves no obligation behind.
Per-block fingerprints and the former textual counter were rejected because
both omit relevant safe-wrapper changes.

**Consequences.** Release and debug QEMU now report 212/212, including `the
double-fault stack cannot be unmapped`. The first inline-`Slot` implementation
made the already-deep debug settlement path overflow its guard; the full gate
caught it, and moving the singleton record beside `ACTIVE_ROOT` restored the
unchanged `AddressSpace` layout before this decision landed. Removing the
active-root refusal let an inactive
space mint a token; removing the overlap refusal unmapped the live IST stack;
both produced exactly that one failed check. Changing strict half-open overlap
to include an adjacent guard edge failed only its host arithmetic test.
Zeroing the syntactic span accumulator failed only the unsafe-counter fixture,
and making the raw MMIO constructor safe failed only the reviewed-fingerprint
test. All five transformations were restored and persist, bringing the
mutation registry to 200.

The semantic pass found no further current defect in the instruction/register,
descriptor/trap/context, page/frame/heap, boot-ABI, port/MMIO, user-copy,
extended-state or synchronization families. PW-401 remains the explicit
VERIFY row for the shared-page compiler and future SMP memory-order contract;
this UP audit does not pretend to close it. PW-703 remains trigger-only as
described above. The unsafe allowance did not grow: it remains 368 measured
lines against 380.
