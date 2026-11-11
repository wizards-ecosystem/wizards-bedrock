# D-89 The double-fault handler gets a guarded stack; the boot thread's waits, with its cost

Status: Accepted, 2026-08-24.

**Decision.** After the switch, a guarded region is mapped and the double-fault
handler is pointed at it. `segment::set_double_fault_stack` rewrites the
interrupt-stack table entry; the `.bss` array stays where it is and is no
longer used. Until D-86 nothing could be done: mapping was one-way, and the IST
stack has to exist before paging does.

The region's geometry comes from the thread arena's: one slot of the same size
with one guard gap below, placed at `StackLayout::end()`. **This adds no new
hardcoded window**, so it adds no entry to D-46's KASLR ledger.

`set_double_fault_stack` is **safe** because of the width, not the timing. The
entry is a single naturally-aligned eight-byte field, so the store is atomic on
x86-64. A double fault arriving mid-update sees either the old stack or the new
one, and both are mapped and writable; the old one is never unmapped.

**The boot thread's stack is not fixed, and this is what that costs.** It is the
bootloader's stack, inside the two-megabyte direct map (D-45, confirmed by
`010` §8). Guarding it means splitting a large page there, which D-88 refuses,
or moving the boot thread onto its own stack, which needs a naked function that
never returns and interacts with slice 4's trap entry rework. **It is
therefore slice 4's, not this slice's.** Waiting costs one thread's stack
overflow going unnoticed on a kernel that is not yet running untrusted code.

**Rationale.** Half a debt paid with a check that can go red beats a whole one
deferred, and this half matters more: a double-fault stack overrun corrupts its
neighbour *while the fault handler is running*, the context least able to
report it. This is also the first shipped caller of `with_active_space`. Until
now D-86's capability was reached only by a boot-time check, and an
unsafe-boundary change whose only user is its own test deserves suspicion.

**Alternatives.** Split the direct map to guard the boot stack (rejected: D-88
refuses splitting, and D-45 plus `010` §8 say keep it that way). Move the fault
stack into the thread arena as an extra slot (rejected: `task::init` would count
it as a thread slot, and the two lifetimes differ). Leave both until slice 4
(rejected: this half costs one mapping and closes the worse gap).

**Consequences.**

- The check is structural, as every guard check here is (D-52): the fault is
  terminal, so what is asserted is what the tables say. The interrupt-stack
  entry points at the top of the region, every page of it is writable and never
  executable, and every page below it is absent.
- `with_active_space` stops being `#[cfg(feature = "selftest")]`.
