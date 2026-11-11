# D-160 A page is 4096 bytes in five places, and now something checks that

Status: Accepted, 2026-08-25.

**Decision.** The page size is stated five times with no dependency between the
sites: `FRAME_SIZE` in `frame-space`, `slab-layout` and `kernel/src/mem.rs`, and
`PAGE_SIZE` in `stack-layout` and `x86-paging`. `xtask/src/machine.rs` scans
`libs/`, `wost/src` and `kernel/src` for every definition of a page-size
constant and requires them all to be the same value, and to be 4096. **The
sites are discovered, not listed** (D-157): the module names which constant
names mean "the size of a page" and finds every definition, so a sixth is
covered the day it is written. The check:

- **refuses to find nothing**: fewer than five definitions fails, because a
  scan that stops matching reports success with nothing compared, the same
  failure shape D-157 found in the model's `INVARIANTS` block;
- **refuses a computed value**: `PAGE_SIZE = 1 << 12` cannot be evaluated, and a
  page size it cannot evaluate is one it cannot compare;
- **asserts 4096, not merely agreement**, because five files agreeing is also
  what five files wrong together looks like.

One crate does not own the constant: `stack-layout` and `slab-layout` are pure
arithmetic parameterised by a size, and depending on `x86-paging` would put an
architecture crate under two with no architecture in them. What was missing was
a comparison, not a dependency.

**Rationale.** It is the most load-bearing constant in the kernel: frames, slabs,
guarded stacks and page tables are all computed from it, and a slab allocator
that believes a frame is 2048 bytes fails at the second slab in a way that looks
like heap corruption.

**Consequences.**

- `CR4_OSXSAVE`, a function-local constant in the two functions of one file that
  set and read the bit, is hoisted.
- `DELIVERY_FIXED = 0` stays twice in `libs/x86-apic`: the ICR's delivery-mode
  field and an LVT entry's are different fields of different registers that
  both encode "fixed" as zero, and merging them would name two facts once.
- `INTERRUPT_FLAG = 1 << 9` stays in both `kernel/src/thread.rs` and the
  self-test, by the general rule: **a check does not import the constant it is
  checking.**
