# D-47 The kernel heap lives in the direct map

Status: Accepted (2026-08-22) (the `HEAP` → `FRAMES` order it established is retired by D-85)

**Context.** `007-memory.md` frames the heap decision as hand-rolled versus a
vetted crate — "the first real test of the dependency policy against
`alloc`-ecosystem convention". `008-research-notes-2.md` §3.5 surveyed `talc`,
`good_memory_allocator`, `slab_allocator_rs` and `linked_list_allocator` and
concluded that the crate is the wrong question: what matters is the boundary,
because Asterinas's answer is a set of type conversions (unused page → slab →
free slot → object) rather than a search algorithm.

D-45 then made the question sharper in a way neither anticipated. `activate`
consumes the address space so that nothing can map into a table the CPU is
already walking — that is what makes `AddressSpace::map` a safe function. A
heap that grew by mapping fresh pages would have to undo it.

**Options.** (a) Reserve a fixed virtual heap range while the tables are being
built, and bump within it; (b) reopen mapping after activation behind a second
guarded API; (c) put the heap in the direct map, which is already mapped
writable and never executable.

**Decision.** (c). The heap takes frames from the frame allocator and addresses
them through the direct map. No page-table entry is ever added for it, so
D-45's one-way property is untouched, and the heap grows to whatever the
machine has rather than to whatever was reserved at boot.

The design falls out of that. Segregated free lists over slabs carved from
single frames, eight power-of-two classes from 16 to 2048 bytes; anything
larger takes a run of whole frames, which is contiguous in the direct map
precisely because it is contiguous physically. `libs/slab-layout` holds the
class arithmetic and is host-tested; `wost::mm::heap` holds the free lists,
which live *inside* the free slots and therefore need raw pointers.

Alignment is met by **choosing a class, never by padding**: a frame is
4096-aligned and a class's slots sit at multiples of the class size, so every
slot is aligned to its own class. A request needing 64-byte alignment takes
the 64-byte class whatever its size. That invariant is what the largest host
test asserts, over every slot of every class.

**Rationale.** Hand-rolled won, but not on the no-dependency principle — on
the boundary. None of the surveyed crates would have known to allocate through
the direct map, because none of them knows this kernel made mapping one-way.
The policy stays swappable: `slab_layout` decides where a request goes and
`heap.rs` is two hundred lines, so replacing first-fit-by-class with per-CPU
magazines is a change to one file (D-43's Safe Policy Injection, applied
again).

Slab frames are never returned. A slab whose slots are all free could be, but
knowing that needs a per-slab live count, and a kernel whose heap has stopped
growing has nothing to spend it on. `heap::stats` reports the numbers that
would justify adding one.

**Consequences.** `alloc` is enabled in the kernel; `Box` and `Vec` work. Three
on-machine checks, and the first of them is the lesson.

The obvious form of "a freed slot is reused" — allocate, note the address,
free, allocate again, require the same address — **is unsound**, and it failed
before passing for the wrong reason. Nothing obliges the compiler to keep two
allocator calls in source order when the values between them are dead, so the
second `Box` could be constructed before the first was released. Adding a
`debug!` between them made it pass. A check that changes verdict when a print
is added is measuring the optimiser, not the kernel.

The replacement counts instead of comparing: `live` moving and returning says
the round trip happened, `slab_frames` standing still says it was served from
the free list, and neither depends on where the slot is or on what the
compiler did with it. This is the second time in this phase that a check's
*early-exit or identity assumption* was the defect rather than the code under
test — the first was D-43's zeroing check — and both were found only because
D-28 requires breaking every new check once.

A lock-order note for Phase 4: `heap.rs` takes `FRAMES` while holding `HEAP`,
during a slab refill. Nothing yet allocates from the heap while holding the
frame lock, and nothing should — the inversion is the one deadlock this pair
can produce.
