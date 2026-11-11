# D-214 Hardware-retained pointers need irrevocable mappings, and unsafe review needs an exact revision

Status: Accepted, 2026-08-28. Amends D-41, D-125 and D-139; closes PW-506.

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

**Rationale.** `set_double_fault_stack` had checked a `MappedStack` once and
stored its top in `TSS.IST1`, after which `AddressSpace::unmap` could remove the
range the processor still retained; D-125 fixed evidence stale *before*
installation, not the mapping's life afterward.
`open_fast_syscall_doors_for_test` returned with `EFER.SCE` set, relying on its
caller to close it; D-139 checked every normal ring-3 entry, but a safe
operation should not rely on a future caller preserving a hazardous interval.
And a 368-line unsafe total against the 380-line budget did not say which
revision of the safe wrappers and `// SAFETY:` grounds had been read. Hardware
does not retain Rust lifetimes. Once a pointer is copied into a TSS, IDT, DMA
descriptor or similar table, validating it at the store is insufficient; the
mapping must be non-revocable for at least the hardware lifetime. A distinct
token makes that stronger fact visible at the only setter, while the
address-space record makes it true after the token is dropped.

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

**Consequences.** The machine check `the double-fault stack cannot be
unmapped` covers the active-root and overlap refusals; host tests cover the
half-open overlap edge, the syntactic span accumulator and the reviewed
per-file record. All five have persistent mutations.

The semantic pass found no further current defect in the instruction/register,
descriptor/trap/context, page/frame/heap, boot-ABI, port/MMIO, user-copy,
extended-state or synchronization families. PW-401 remains the explicit
VERIFY row for the shared-page compiler and future SMP memory-order contract;
this UP audit does not pretend to close it. PW-703 remains trigger-only as
described above. The unsafe allowance did not grow: it remains 368 measured
lines against 380.
