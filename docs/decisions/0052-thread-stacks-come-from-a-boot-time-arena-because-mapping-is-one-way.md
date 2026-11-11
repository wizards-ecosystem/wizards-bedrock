# D-52 Thread stacks come from a boot-time arena, because mapping is one-way

Status: Accepted, 2026-08-22.

**Amended by D-113**, which raised the stack to 32 KiB: the arena is sixteen
slots of a 32 KiB stack and costs 512 KiB, not the 256 KiB this entry works
through below. The reasoning is unchanged and the numbers stay as decided;
D-45 and D-51 carry such a pointer, and this one did not (D-146).

**Decision.** An arena mapped while the tables are built: sixteen slots of a
16 KiB stack above a 4 KiB gap, at `0xffff_fe00_0000_0000`, clear of the
direct map, the device window and the image. Every stack is committed,
zeroed and mapped writable-and-NX in `build`; every gap is never mapped. The
arithmetic is `libs/stack-layout`, host-tested for the property the design
rests on: no two slots touch, and every stack has its guard immediately
below. `wost` keeps the allocation and the map calls, and hands back a
`StackArena`, neither `Copy` nor `Clone`: on D-49's `DeviceMapping` pattern,
the evidence that `task::init` may fabricate on exactly these ranges.

The boot thread keeps the bootloader's stack: moving it is a one-way jump
mid-`kernel_main` for a guard it has never needed.

**Rationale.** D-45 forbids mapping after the switch, and the direct map
cannot hold the guard hole D-12 requires. A constant cap in one file is
cheaper than a growth mechanism designed before its constraints are known,
and an absent mapping is the strongest and cheapest guard there is.

**Consequences.**

- A cap on threads until Phase 5's growth design, recorded as such.
- One more layout assumption for D-46's ledger: the arena's address is fixed
  and KASLR would have to move it.
- `008` §4.1's caveat is named: a frame larger than 4 KiB can jump the gap.
  The kernel has no `alloca` and no SSE spills, so its frames are small; if
  that changes, the gap grows.
- `task::init` walks every stack in the *live* tables before accepting the
  arena: the arena says they were mapped, the walk says the CPU agrees.
