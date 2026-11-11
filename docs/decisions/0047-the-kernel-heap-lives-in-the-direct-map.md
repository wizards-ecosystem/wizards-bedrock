# D-47 The kernel heap lives in the direct map

Status: Accepted, 2026-08-22. The `HEAP` → `FRAMES` order it established is retired by D-85.

**Decision.** The heap takes frames from the frame allocator and addresses
them through the direct map, already mapped writable and never executable.
No page-table entry is ever added for it, so D-45's one-way property is
untouched, and the heap grows to whatever the machine has, not to what was
reserved at boot.

Segregated free lists over slabs carved from single frames, eight
power-of-two classes from 16 to 2048 bytes. Anything larger takes a run of
whole frames, contiguous in the direct map because it is contiguous
physically. `libs/slab-layout` holds the class arithmetic and is
host-tested; `wost::mm::heap` holds the free lists, which live *inside* the
free slots and so need raw pointers.

Alignment is met by **choosing a class, never by padding**. Frames are
4096-aligned and slots sit at multiples of the class size, so every slot is
aligned to its own class; a request needing 64-byte alignment takes the
64-byte class whatever its size.

Slab frames are never returned: that needs a per-slab live count, and a
kernel whose heap has stopped growing has nothing to spend it on.
`heap::stats` reports the numbers that would justify adding one.

**Rationale.** Hand-rolled won on the boundary, not on the no-dependency
principle: none of the surveyed crates (`008-research-notes-2.md` §3.5)
would have known to allocate through the direct map, because none knows this
kernel made mapping one-way. The policy stays swappable: replacing
first-fit-by-class with per-CPU magazines is a change to one file (D-43's
Safe Policy Injection).

**Consequences.** `alloc` is enabled in the kernel; `Box` and `Vec` work.
The freed-slot check counts (`live`, `slab_frames`) instead of comparing
addresses, because the compiler need not keep two allocator calls in source
order when the values between them are dead. A check whose verdict changes
when a print is added measures the optimiser, not the kernel.

A lock-order note for Phase 4: `heap.rs` takes `FRAMES` while holding
`HEAP`, during a slab refill. Nothing yet allocates from the heap while
holding the frame lock, and nothing should; the inversion is the one
deadlock this pair can produce.
