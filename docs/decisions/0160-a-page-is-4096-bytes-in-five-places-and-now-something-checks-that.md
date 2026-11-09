# D-160 A page is 4096 bytes in five places, and now something checks that

Status: Accepted (2026-08-25)

**Context.** Having merged the two ELF readers (D-158), the obvious next
question is how many other tables this tree states twice. Sweeping every
`const NAME: type = literal` across `libs/`, `wost` and `kernel` for a name
defined more than once turned up one answer that matters and two that do not.

**The page size is stated five times.** `frame-space` and `slab-layout` call it
`FRAME_SIZE`, `stack-layout` and `x86-paging` call it `PAGE_SIZE`, and
`kernel/src/mem.rs` has its own `FRAME_SIZE`. Four crates and a kernel module
with no dependency between them, each declaring 4096, and nothing comparing
them.

This is the most load-bearing constant in the kernel. Every layout is computed
from one of these: the frame allocator hands out frames, the slab allocator
divides them, the stack allocator strides between guard pages, and the page
tables map them. A frame allocator handing out 4096-byte frames to a slab
allocator that believes they are 2048 does not fail a test — it fails at the
second slab, in a way that looks like heap corruption.

**Options.** Make one crate own it and have the rest depend on that crate;
leave it and rely on the number being obvious; or discover the sites and
compare them.

**Decision.** The third. `xtask/src/machine.rs` scans `libs/`, `wost/src` and
`kernel/src` for every definition of a page-size constant and requires them all
to be the same value, and to be 4096.

Not the first option, because the dependency it would add is the wrong shape:
`stack-layout` and `slab-layout` are pure arithmetic parameterised by a size,
and making them depend on `x86-paging` to learn what a page is would put an
architecture crate under two that have no architecture in them. Their
independence is a property worth keeping; what was missing was not a dependency
but a comparison.

**The sites are discovered, not listed** (D-157). What the module names is the
*fact* — which constant names mean "the size of a page" — and it then goes and
finds every definition. A sixth transcription is covered the day it is written,
without touching the check.

**Rationale.** Three details make the check able to fail rather than merely
able to pass:

- **It refuses to find nothing.** Fewer than five definitions is a failure, not
  a pass, because a scan that stops matching reports success with nothing
  compared — the same failure shape D-157 found in the model's `INVARIANTS`
  block.
- **It refuses a computed value.** `PAGE_SIZE = 1 << 12` is not a literal this
  can evaluate, and a page size it cannot evaluate is a page size it cannot
  compare. Failing is honest; skipping would be a hole shaped exactly like the
  thing being checked.
- **It asserts 4096, not merely agreement.** Five files agreeing is also what
  five files being wrong together looks like.

**Consequences.**

- `xtask/src/machine.rs`, three host tests, broken twice: one crate told a page
  is 2048 (red, naming both sites), and the scan made to match nothing (red).
- One real duplication fixed: `CR4_OSXSAVE` was a function-local constant in
  *two functions of one file* — the one that sets the bit and the one that
  reads it back — with the same value and the same one-line doc. Hoisted.
- One duplication deliberately left. `DELIVERY_FIXED = 0` appears twice in
  `libs/x86-apic`, in `command` and in `local_vector`, and they are not one
  fact: the ICR's delivery-mode field and an LVT entry's delivery-mode field
  are different fields of different registers that both encode "fixed" as zero.
  Merging them would name two facts once. The doc comments differ because the
  facts do.
- `INTERRUPT_FLAG = 1 << 9` also appears twice, in `kernel/src/thread.rs` and
  in the self-test. That one stays on purpose and is the general rule: **a
  check does not import the constant it is checking.** The self-test asking
  about bit 9 independently is what makes it a check rather than a restatement.
