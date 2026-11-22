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

## Worlds

- **Tree.** A world is live, dying or dead. Its path lists its ancestors and is at most 16 long. Its
  descendant set holds exactly the worlds whose path contains it. Creating a world enters it into
  every ancestor's set or into none. Once a world is dying nothing joins its descendants, and every
  descendant is dying before the world is dead.
- **Kill.** Killing a world takes a number of steps bounded by what that world holds, whatever any
  other world does. No step can refuse, and one teardown never waits on another. Each frame returns
  once, zeroed, to the nearest live ancestor.

## Arenas

- **Containment.** Every kernel object made for a world's request lives in a frame that world's
  arena owns, and such a frame holds only that world's objects and data. Objects name each other by
  index and generation, never by address, so an arena's contents are a value. A reference into
  another world is weak: it never keeps that world or its objects alive, except the hold a
  processor's queue keeps on a runnable thread, which that world's kill removes in bounded steps.
- **Refusal.** An arena never uses more frames of a colour than it has reserved. A request past the
  reservation returns a refusal to its caller and changes no state, in that world or any other.
- **Separation.** Nothing a world does allocates from another world's arena or lowers its free
  reservation, with one exception: a spawn into a descendant world is charged to that descendant.
  What a foreign world may cause to be recorded in an object is prepaid by the object's owner at a
  bound fixed when the object is made.
- **Accounting.** A world's charge is its frame count times the frame size, and no byte a world
  causes lies outside its frames. For each colour, the reservations of all arenas plus the kernel's
  own frames never exceed the managed frames.

## Processes

- **Life.** A process is running, then exited with one status, then released. The status is written
  once, before any observer wakes, and every accepted wait completes once with it. Its threads,
  address space and handle table are each released exactly once, by steps that cannot refuse.

## Handles

- **Table.** Each process has one handle table, in its world's arena. A handle resolves only in the
  table that issued it. A handle made from another handle to the same object never has a right its
  source lacks; a handle to a new object, such as an opened file or a sealed memory object, has
  exactly the rights its operation names. A handle value is a function of the world's key, the
  process, the slot and the generation, and a slot retires rather than repeat a generation.

## Memory objects

- **State.** A memory object is writable, sealed read-only or sealed executable, and moves only from
  writable to sealed. A mapping of its frames is writable only while the object is writable and
  executable only when it is sealed executable, in every address space, the direct map and every
  IOMMU table.
- **Seal.** A seal succeeds only on the object's single handle, with no live write reference, and
  for execution only with quota that covers its pages. A write reference is released only after
  every TLB that may hold its translation has been flushed. Success consumes the handle and returns
  exactly one new one; a refusal changes nothing.
- **Execute.** The execute right exists only on a handle that a seal for execution produced, or on
  a narrowed copy of one. The live sealed executable pages charged to a quota never exceed its
  limit.
- **Mappings.** Every present user page-table entry is the image of one mapping record. Unmap and
  protect only remove permission. A frame returns to its arena only when its object has no handle,
  mapping or pin and every flush that covers its removed translations has completed.
- **Loading.** A user thread starts only inside an executable mapping of a sealed executable object
  in its own address space. The kernel parses no executable format.

## Channels

- **Conservation.** A handle moved in a message is in exactly one place: the sender's table, one
  inbox entry, the staging of the caller awaiting a reply, or the receiver's table. A refused send
  leaves every handle in the sender's table with its value and rights. Once a message is committed
  its handles never return to the sender: a handle in a discarded entry, a cancelled call or a dead
  table is destroyed with it.
- **Order.** Each committed message is read once, in the order its endpoint committed it, or is
  destroyed with the endpoint. Calls and messages share that order.
- **Call.** Each call has exactly one outcome: a reply, abandoned, peer closed, cancelled or timed
  out. A reply right names one call and succeeds at most once. A call the server received is never
  reported as peer closed.
- **Cost.** An inbox is paid for by its holder when the endpoint is made, and a send to a full
  inbox is refused and changes nothing. An endpoint moved in a message stays in its sender's arena
  until the receiver reads it into its own.
- **Donation.** While a server serves a synchronous call, its running time is charged to the
  caller.

## Processors

- **Per-processor state.** In ring 0 the `GS` base names this processor's entry area, and in ring 3
  it is zero. A thread runs on at most one processor. While a thread is on a processor, no other
  processor picks it, frees its stack or reuses its switch record.
- **Lock order.** Every lock has a rank. A processor takes a lock only above every rank it holds,
  and two locks of one rank only in ascending order of their objects. Under a leaf guard no
  caller-supplied work, block, flush wait, interrupt wait or user copy runs.
- **Wakeup.** Each thread has one wake word. If a waker claims a thread's wait after the thread
  registered, the thread is not left blocked. A blocked thread is always linked where a waker can
  reach it, and a wait completes once.
- **Shootdown.** A frame or table returns to its pool only after every flush that covers a removed
  translation has completed on every processor that could hold it. No processor waits for an
  acknowledgement with interrupts off or a spin lock held.
- **Queues.** A runnable thread is on exactly one queue or one processor, except inside one move
  made with interrupts off. A thread is picked only by the processor whose queue holds it.
- **Grace period.** A dead world's frames are zeroed and reused only when its threads are off every
  processor, no pin or kernel-held count on it remains, and every flush of its address spaces has
  completed.
- **Panic stop.** Before a panic releases any lock by force, every other processor has acknowledged
  a stop and never resumes. If one does not answer in bounded time, no lock is forced.
- **Clock publication.** One processor owns the clock. A reader on any processor sees a rate and
  origin that were published together.
