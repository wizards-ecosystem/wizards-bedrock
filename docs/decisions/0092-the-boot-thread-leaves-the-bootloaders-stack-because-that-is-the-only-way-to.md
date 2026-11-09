# D-92 The boot thread leaves the bootloader's stack, because that is the only way to guard it

Status: Accepted, 2026-08-24. Completes D-12's stack baseline.

**Context.** D-89 guarded the double-fault stack and named the one that was
left: the boot thread's. It is the bootloader's, it has nothing below it, and
it sits inside the direct map — which is mapped in two-megabyte pages
deliberately (D-45), confirmed as right by measurement (`010` §8), and which
D-88 refuses to split. So it cannot be guarded where it is. The choice was to
leave it bare or to leave *it*.

**Decision.** The boot thread moves onto a stack of its own, mapped and guarded
like every other, as soon as the tables are live. `kernel_main` ends at that
switch and everything after it runs in `boot_continue`.

The primitive is `task::continue_on_stack`, and it is **safe** because its
argument is the proof. `AddressSpace::map_stack` allocates the frames, zeroes
them, maps them writable and never executable, and then *verifies the guard
pages are absent* — a stack whose guard is mapped is not a guarded stack, and
no caller could tell from the return value, so it is checked where it is known.
The `MappedStack` it hands back cannot be made any other way, and
`is_installed` confirms it belongs to the space this CPU actually walks, since
stack addresses collide across spaces.

What the boot thread carries across goes in a static, not an argument: the old
stack is abandoned, so a pointer into it would point at memory nothing owns.

**The alignment is the part with history.** The continuation is entered by
`jmp`, not `call`, so no return address is pushed — and the ABI wants
`rsp + 8` sixteen-byte aligned at a function's first instruction. The eight
bytes are subtracted at the load instead. That is the same off-by-one word
D-76 found in the fabricated first frame, where it was symptomless without SSE
and invisible to every register check; writing it down here is cheaper than
finding it twice.

**Rationale.** `000-vision.md` claims "ordinary thread stacks guarded" and
`002` carried two exceptions for two phases. Exceptions to a security baseline
decay into the baseline if they sit long enough, and this one had a fix that
was three phases of infrastructure away and is now one function. The
alternative — splitting the direct map to guard a stack — would trade a
measured property for an unmeasured one.

**The check runs on the stack it is about**, which is what makes it more than
structural. The stack pointer under it must be inside the mapped region, so a
switch that silently did not happen fails, where a check reading only the
tables would pass on their strength alone. The break is to decline the switch,
and it goes red.

**Alternatives.** Split the direct map (rejected: D-88 refuses it, D-45 and
`010` §8 say keep it whole); leave the boot thread parked so its stack stops
growing (rejected: it does not guard anything, it just makes an overrun less
likely); make `continue_on_stack` `unsafe` (rejected: `kernel` may not contain
an unsafe block (D-13), so the evidence has to be in the type or the switch
cannot be reached from where it is needed).

**Consequences.**

- `002`'s security baseline has **no stack exceptions left**. Every stack this
  kernel runs on — boot, thread, double-fault — has an unmapped page beneath
  it, and each has a check that goes red without it.
- `AddressSpace::map_stack` is the one place a guarded stack is made now.
  D-89's fault stack keeps its own mapping call for the moment; folding it in
  is tidying, not a decision.
- Three windows are now derived from the thread arena's end rather than
  hardcoded, so D-46's KASLR ledger is unchanged by any of them.
- A kernel that cannot map the stack keeps the bootloader's and says so. That
  is the kernel every earlier phase shipped, so declining costs a guard rather
  than a boot.
- Eighty-six named machine checks.
