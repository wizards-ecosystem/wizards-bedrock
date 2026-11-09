# D-89 The double-fault handler gets a guarded stack; the boot thread's waits, with its cost

Status: Accepted, 2026-08-24.

**Context.** `002-architecture.md`'s security baseline has recorded two
exceptions since Phase 4: *"Two stacks remain unguarded: the boot thread's,
which is the bootloader's, and the double-fault IST stack in `.bss` — both owed
to Phase 5's stack story."* Ordinary thread stacks have had guards since D-52.
The double-fault stack is the worst of the three to leave bare, because it is
the stack the CPU switches to when the kernel is *already* in trouble, and an
overrun there runs into whatever the linker placed next — silently.

Until D-86 there was nothing to be done about it: mapping was one-way, and the
IST stack has to exist before paging does.

**Decision.** After the switch, a guarded region is mapped and the handler is
pointed at it. `segment::set_double_fault_stack` rewrites the interrupt-stack
table entry; the `.bss` array stays where it is and is simply no longer used.

The region's geometry comes from the thread arena's — one slot of the same
size with one guard gap below, placed at `StackLayout::end()`. **This adds no
new hardcoded window**, so it adds no entry to D-46's KASLR ledger, and the
arithmetic is `libs/stack-layout`'s, which already has tests.

`set_double_fault_stack` is **safe**, and the reason is the width rather than
the timing: the entry is a single naturally-aligned eight-byte field, so the
store is atomic on x86-64. A double fault arriving mid-update sees either the
old stack or the new one, and both are mapped and writable — the old one is in
the kernel image and is never unmapped, so there is no window in which the
losing value is invalid.

**The boot thread's stack is not fixed, and this is what that costs.** It is the
bootloader's, and it sits inside the direct map, which is deliberately mapped in
two-megabyte pages (D-45, confirmed by `010` §8). Guarding it would mean either
splitting a large page there — which D-88 refuses on purpose, and which is the
first step toward losing the property `010` measured as worth keeping — or
moving the boot thread onto a stack of its own, which means switching `RSP`
under a running call chain. The second is the real answer and it is not free:
it needs a naked function that never returns, and it interacts with the trap
entry rework slice 4 is about to do. **It is therefore slice 4's, not this
slice's**, and the cost of waiting is one thread's stack overflow going
unnoticed on a kernel that is not yet running untrusted code.

**Rationale.** Half a debt paid with a check that can go red beats a whole one
deferred. And the half taken is the one that matters more: a thread stack
overrun corrupts a neighbour, while a double-fault stack overrun corrupts
whatever is next to it *while the fault handler is running*, which is the
context least able to report it.

This is also the first shipped caller of `with_active_space`, which matters for
its own reason: D-86's capability was until now reached only by a boot-time
check, and an unsafe-boundary change whose only user is its own test is a
change one should be suspicious of.

**Alternatives.** Split the direct map to guard the boot stack (rejected: D-88
refuses splitting, and D-45 plus `010` §8 say keep it that way); move the fault
stack into the thread arena as an extra slot (rejected: `task::init` would then
count it as a thread slot, and the two lifetimes are not the same); leave both
until slice 4 (rejected: this half costs one mapping and closes the worse gap).

**Consequences.**

- `002`'s security baseline loses one of its two exceptions; the remaining one
  now names slice 4 and says what waiting costs.
- The check is structural, as every guard check here is (D-52): the fault is
  terminal, so what is asserted is what the tables say — the interrupt-stack
  entry points at the top of the region, every page of it is writable and never
  executable, and every page below it is absent. The break is to map the region
  starting at the guard, and it goes red.
- `with_active_space` stops being `#[cfg(feature = "selftest")]`.
- Eighty-five named machine checks.
