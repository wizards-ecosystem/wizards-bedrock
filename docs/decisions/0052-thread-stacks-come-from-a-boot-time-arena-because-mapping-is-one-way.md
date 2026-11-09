# D-52 Thread stacks come from a boot-time arena, because mapping is one-way

Status: Accepted, 2026-08-22.

**Context.** D-45 made `activate` consume the address space, so nothing can
map after the CR3 switch; D-49 deferred how the address space grows to
Phase 5, deliberately. Phase 4 needs stacks with guard pages for threads
that do not exist at boot.

**Options.** Stacks in the direct map (no hole can be punched for a guard
without unmapping, which D-45 forbids, and D-12 requires the guard); a
post-activation mapping API (weakens D-45 and pre-empts the decision D-49
reserved); a fixed arena mapped while the tables are being built.

**Amended by D-113**, which raised the stack to 32 KiB: the arena is sixteen
slots of a 32 KiB stack and costs 512 KiB, not the 256 KiB this entry works
through below. The reasoning is unchanged and the numbers here are left as they
were decided — the pointer is what D-45 and D-51 carry beside it, and this one
did not (D-146).

**Decision.** An arena: sixteen slots of a 16 KiB stack above a 4 KiB gap, at
`0xffff_fe00_0000_0000` — clear of the direct map, the device window and the
image. Every stack is committed, zeroed and mapped writable-and-NX in
`build`; every gap is simply never mapped. The arithmetic is
`libs/stack-layout`, host-tested for the property the design rests on: no
two slots touch, and every stack has its guard immediately below. `wost`
keeps the allocation and the map calls, and hands back a `StackArena` that
is neither `Copy` nor `Clone` — the evidence, on D-49's `DeviceMapping`
pattern, that `task::init` may fabricate on exactly these ranges.

The boot thread keeps the bootloader's stack. Moving it would mean a one-way
jump onto a new stack mid-`kernel_main`, and the only thing that buys today
is a guard under a stack that has never needed one; slice 4's idle thread is
the natural moment to retire it, if it is retired at all.

**Rationale.** Sixteen stacks cost 256 KiB of a machine with hundreds of
megabytes, and a cap that is a constant in one file is cheaper than a growth
mechanism designed before its constraints are known. An absent mapping is the
strongest guard there is and the cheapest.

**Consequences.**

- A real cap on threads until Phase 5's growth design, recorded as such.
- One more layout assumption for D-46's ledger: the arena's address is fixed
  and KASLR would have to move it.
- `008` §4.1's caveat is named rather than hidden: a frame larger than 4 KiB
  can jump the gap. The kernel has no `alloca` and no SSE spills, so the
  frames it builds are small; if that changes, the gap grows.
- `task::init` walks every stack in the *live* tables before accepting the
  arena — the arena says they were mapped, the walk says the CPU agrees.
- The check is structural: the guard is absent at both ends and the stack is
  present, writable and NX at both ends, for all sixteen slots. Mapping the
  guard turned it red.
