# Statements

What must be true of each core piece of the kernel. Code is built against these statements, audits
attack them, and the Verus proofs at each milestone in `docs/roadmap.md` section 3 are stated
against them. A statement changes only together with the code it governs.

## Frames

- **Ownership.** Every managed frame has one record. Its owner is free, the kernel, a page table, a
  device or one live arena. An arena's frames are exactly the frames whose record names it, and the
  owner counts sum to the managed count.
- **Colour.** A frame's colour is its frame number modulo the colour count, which is fixed at boot.
  An arena allocates only colours in its mask, and a child's mask lies within its parent's. For each
  colour the unused reservations of all arenas never exceed the free frames of that colour, so a
  reserved frame can always be allocated.
- **W^X per frame.** A frame is writable or sealed. In every address space, the direct map included,
  no mapping of a writable frame is executable and no mapping of a sealed frame is writable. A frame
  becomes sealed only when no writable mapping of it remains in any table or TLB, and writable again
  only after it is released and no mapping of it remains.
- **Zero on crossing.** A frame that passes from one owner to another holds only zeros at that
  moment. Memory that ring 3 or a device can reach is built only from frames known to be zero.
- **Boot provenance.** Only the memory map the bootloader handed over can start the allocator or
  open firmware memory. No safe caller can build a value that does either.

## Randomness

- **Seeding.** No random output exists before the generator has absorbed 256 bits from `RDSEED`,
  or from `RDRAND` on a processor without `RDSEED`. Ring 3 does not open before that.
- **Erasure.** Every output is produced under a key that is overwritten before the output is
  returned, so captured generator state reveals no earlier output.
- **World keys.** Each world holds a key drawn fresh from the generator and never derived from its
  parent. A value derived from the key is stored once and never recomputed. A copied world gets a
  new key, so two copies never share randomness produced after the copy.

## Clock

- **Trap.** On every return to ring 3, `CR4.TSD` is clear exactly when the running world holds the
  raw time right. A clock instruction from any other world faults and retires its thread.
- **Served time.** The time a world is served never goes backwards. In normal mode it is a multiple
  of the world's precision. In recorded mode it changes only inside a kernel entry and equals the
  logged value.
- **Limit.** A processor cannot trap `RDRAND`, `RDSEED` or `RDPID` outside a virtual machine, and a
  world with two threads or a device can count time. Only a world on a virtual processor is denied
  every such source, and only such a world's replay is exact by construction.

## Kernel entry

- **Context token.** A `ThreadContext` exists only on the running thread's own kernel stack while
  the trap depth is zero. `wost` mints one only at boot entry, at thread start and at the handoff
  from a ring 3 entry. Trap code cannot name one, so it cannot block, and every allocation for a
  user request takes one.
- **User frame.** While a thread serves a ring 3 entry, its user registers are the frame at the top
  of its own kernel stack, touched only by the entry and exit assembly and that thread's handler.
  Ring 3 resumes exactly the frame the return check accepted, with interrupts off from the check to
  the return.
- **Preemption.** A handler for a ring 3 entry starts with no lock held and may be switched out at
  any interrupt it takes outside a spin lock. Trap code never blocks and never switches except at
  the exit of a ring 0 interrupt.
- **Barrier.** On a processor that offers IBPB, one executes between the last ring 3 instruction
  under one user address space and the next ring 3 instruction under a different one.

## The ABI and its witnesses

- **One source.** Every value that crosses ring 3 is defined once, in `libs/native-abi`, and the
  kernel and every program name it from there. Each layout carries a version. A change a reader can
  see bumps that version in the same commit, and a program refuses a version it was not built for.
- **Verdict by record.** The gate decides a boot only from boot records and the exit code, and an
  unparsable or unknown record fails it. Only the kernel writes records, and no bytes a program
  controls can form one.
- **Unsafe.** Outside `wost`, `unsafe` appears only in the ring 3 runtime `witness/rt`, and each of
  its exports is sound for any caller.
