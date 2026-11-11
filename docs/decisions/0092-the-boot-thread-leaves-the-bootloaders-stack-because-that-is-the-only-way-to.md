# D-92 The boot thread leaves the bootloader's stack, because that is the only way to guard it

Status: Accepted, 2026-08-24. Completes D-12's stack baseline.

**Decision.** The boot thread moves onto its own stack, mapped and guarded like
every other, as soon as the tables are live. `kernel_main` ends at that switch
and everything after it runs in `boot_continue`.

The primitive is `task::continue_on_stack`, and it is **safe** because its
argument is the proof. `AddressSpace::map_stack` allocates the frames, zeroes
them, maps them writable and never executable, then *verifies the guard pages
are absent*. No caller could tell a mapped guard from the return value, so the
check sits where the fact is known. The `MappedStack` it returns cannot be made any other way.
`is_installed` confirms it belongs to the space this CPU walks, since stack
addresses collide across spaces.

What the boot thread carries across goes in a static: the old stack is
abandoned, so a pointer into it would point at memory nothing owns.

**The alignment.** The continuation is entered by `jmp`, not `call`, so no
return address is pushed, and the ABI wants `rsp + 8` sixteen-byte aligned at a
function's first instruction. The eight bytes are subtracted at the load
instead. D-76 found the same off-by-one word in the fabricated first frame.

**Rationale.** Exceptions to a security baseline decay into the baseline if
left long enough, and the bootloader's stack, inside the two-megabyte direct
map, cannot be guarded without splitting it, which would trade a measured
property for an unmeasured one.

**The check runs on the stack it is about**: the stack pointer under it must be
inside the mapped region, so a switch that silently did not happen fails, where
a check reading only the tables would pass.

**Alternatives.** Split the direct map (rejected: D-88 refuses it, D-45 and
`010` §8 say keep it whole). Leave the boot thread parked so its stack stops
growing (rejected: it guards nothing, it only makes an overrun less likely).
Make `continue_on_stack` `unsafe` (rejected: `kernel` may not contain an unsafe
block (D-13), so the evidence has to be in the type or the switch is unreachable
from where it is needed).

**Consequences.**

- `002`'s security baseline has **no stack exceptions left**. Every stack this
  kernel runs on (boot, thread, double-fault) has an unmapped page beneath it,
  and each has a check that goes red without it.
- `AddressSpace::map_stack` is the one place a guarded stack is made.
- Three windows derive from the thread arena's end instead of being
  hardcoded, so none of them changes D-46's KASLR ledger.
- A kernel that cannot map the stack keeps the bootloader's and says so. Every
  earlier phase shipped that kernel, so declining costs a guard, not a boot.
