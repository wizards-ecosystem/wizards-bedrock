# D-94 The mid-phase audit: a refused mapping freed frames it had mapped

Status: Accepted, 2026-08-24.

**Context.** D-77 established that a completed phase's green is a claim to
attack. This audit applied that *mid*-phase to slices 1 to 4, through a pass
told to break them. The baseline was green: 86 machine checks and every host
test passing.

It still found six defects crossing a soundness or evidence boundary, a
tautology in a check written four commits earlier, and six claims in prose that
the code had outrun.

**The worst was mine, introduced by the ADR closing the same class of hole.**
D-87 gave `map_owned` this contract:

> On failure nothing was mapped — the whole range is preflighted before any
> entry is written (D-77) — so `run` is dropped and its frames go back to the
> allocator.

**There was no preflight.** `map_range` wrote each entry as it walked, so a
range failing part-way left the earlier pages mapped. `map_owned` trusted its
doc and returned the `Segment`'s frames to the allocator while the space still
mapped them `PRESENT | WRITABLE`. The next allocation hands out memory a live
mapping can write, and a later `unmap` frees it a second time. Safe code could
reach it in the live kernel space through a public `wost` export.

The sentence came from D-77's summary, where it was true of the code then.
Restating a property in a new place is not the same as holding it, and a doc
comment is not a check.

**Decision: the six soundness and evidence fixes.**

1. **`map_range` rolls back.** A failed mapping clears every entry it wrote, so
   "nothing was mapped" is true rather than claimed. Tables allocated on the way
   stay as zeroed frames pointing nowhere; reclaiming one means proving no
   sibling survives.
2. **`unmap` refuses non-canonical addresses.** A walk masks bits 48 and above,
   so the low-half alias of a higher-half address reached the same tables. The
   `invlpg` that followed named the non-canonical form, which invalidates
   nothing. Clearing a live mapping through its alias and returning the frames
   left a stale writable TLB entry, the hazard `Unmapped` exists to prevent.
3. **`set_double_fault_stack` takes `&MappedStack`.** It took a bare `u64` and
   checked only alignment. Any safe caller could point IST1 at a `Vec`, at
   kernel data, or at a hole, on the one stack used only when the machine is in
   trouble. Four hundred lines away, `continue_on_stack` already refused to
   stand on anything without evidence.
4. **The store is the eight-byte entry, not the whole segment.** Its SAFETY
   comment argued from the field's width while the code wrote the entire
   `TaskState`, a memcpy that can be observed torn.
   `TaskState::interrupt_stack_offset` now names the offset, host-tested against
   SDM figure 8-11.
5. **`guard_the_fault_stack` goes through `map_stack`.** It mapped by hand and
   never verified the guard, which only the self-test build checked, while `002`
   claimed every stack had one.
6. **`map_owned` maps four-kilobyte pages only.** A large page carrying `OWNED`
   is one `unmap` must refuse for ever, since it does not split (D-45): an
   unreachable leak. A few more entries is the cheaper mistake.

Two smaller fixes rode along. `cpuid(7, 0)` was read without a max-leaf guard, a
`#GP` on pre-2012 hardware. `continue_on_stack` abandoned a stack without
asserting that no lock guard was live on it, the rule `prepare` already enforces
for the analogous case.

**Decision: the checks.** *the double-fault stack is guarded* ended on
`stack_top() >= layout.base()`, which is true of every layout ever built. That
is D-77's own corrected failure mode, reappearing in the commit that added it.
*unmapping refuses what it does not own* claimed to test the direct map's large
pages but used the kernel's linked text address, and accepted either of two
errors, so it could not say which mechanism fired. It now checks six distinct
refusals, with the direct-map address *probed* rather than assumed, because the
first two megabytes are mapped in small pages. *an unmapped range returns its
frames* counted free frames before a mapping that could allocate a page table,
so it passed only because an earlier check had built that table.
`libs/slab-layout`'s new test asserted `n*b >= n*b`, true by algebra; it now
compares against counts and offsets transcribed by hand.

**Decision: the prose.** `011`'s lock-order section claimed taking two spin
locks is `E0499`. It is not: `ThreadContext::claim` mints a token wherever the
interrupt flag says thread context, so a second one can be minted inside a live
guard. The compile-time half is a strong default, not a proof: one visible,
greppable line defeats it, audited like an `unsafe` block. The section now says
so. It also referenced `libs/lock-rank`, which D-85 explains was never written.
`009` claimed this kernel holds no lock across a call inside `wost`;
`mm::with_memory_map` does. Four stale `SAFETY` comments and module docs in
`paging.rs` still said nothing unmaps.

**Rationale.** Reading the code against its own documentation reached every
finding, and the gate found none, because the checks had drifted toward
restating the implementation. That is why the adversarial pass is a *pass*, told
to break the code; a balanced review would have found none of this.

**Consequences.**

- *a refused mapping leaves no trace* is the check the first defect earned. It
  went red when the fix was reverted, the only way to know it catches its bug.
- Ninety-four named machine checks.
- **Three findings are recorded and not fixed**, because each is a design
  decision rather than a repair. The closure-under-guard hole
  (`with_memory_map`, `with_io_apic`, `with_active_space`) needs a guard type
  that forbids calls. `mem::forget` on a `SpinLockGuard` strands the interrupt
  flag, which `Unmapped`'s own doc says no soundness argument may rest on.
  `MappedStack` can be invalidated by `unmap` while the evidence is still live.
  They are in `011`'s open list.
- **A property restated in a second place is not held in both.** D-87's contract
  was true where D-77 wrote it and false where D-87 repeated it. Only reading
  the two together showed that.
