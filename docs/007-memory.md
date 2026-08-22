# 007 — Memory (Phase 3)

The phase that gives the kernel its own memory: a physical frame allocator
over the bootloader's map, page tables it built itself, the W^X half of the
security baseline D-12 has been owed since boot, a heap, and — with device
pages finally mappable — the modern halves of two Phase 2 compromises.

Phase 2 left a kernel that takes interrupts and keeps time but still lives
entirely inside Limine's page tables: it cannot map a device, cannot protect
a mapping, cannot allocate. Everything after this phase (threads, userspace,
handles) allocates kernel objects as a matter of course, so memory is the
last phase that can be built without itself.

This file is the phase's charter, written before its code per D-19. As with
`006-cpu-fundamentals.md`, each slice's design is recorded below **as it is
built**, so the file ends the phase as the description of what exists; the
slice table is the dependency-forced proposal, refined when the phase
starts.

References: Intel SDM Vol. 3A ch. 4 (paging), ch. 12 (memory types, PAT);
Limine protocol (memory map and HHDM features, base revision 3); Asterinas
OSTD's frame-ownership model (the framekernel precedent D-13 follows); seL4
untyped/retype (the accounting question `002` poses); OSDev paging and
higher-half pages.

## What earlier phases left owing

Every debt below is on the record elsewhere; this table is where a future
session finds them collected.

| Debt | Where recorded | What Phase 3 does |
|---|---|---|
| W^X for every mapping | D-12, D-32 ("waits for Phase 3") | Build the tables W^X-correct from the first CR3 write; NX has been on since slice 1 precisely so this is enforceable |
| Guard pages under all stacks, zero-on-allocate | D-12 | Arrives with the frame allocator and the stack mappings |
| KASLR | D-12; `001-boot.md` known limitation | Blocked on a relocatable image (`linker.ld` hardcodes the base); decide in-phase whether to take the relocation work now or record the deferral |
| The direct map, deleted with its false bound | D-33 ("Phase 3 adds it back with the code that can use it") | `BootInfo` grows the memory map and HHDM; the direct map returns as the mechanism for table-walking and frame access |
| Device holes unmappable — the wall itself | D-33 | Once mappable: the I/O APIC becomes reachable; x2APIC remains the local-APIC choice on its own merits |
| Keyboard polled from the tick | D-37 ("Phase 3 revisits with the I/O APIC") | Route IRQ 1 through the I/O APIC; the polled drain call in the idle loop retires, the driver and decoder carry over unchanged |
| Framebuffer console reads back uncached memory to scroll | `001-boot.md` known limitation | A software back-buffer, the heap's first consumer |
| Kernel-memory accounting: untyped-retype or not | `002-architecture.md` open questions | Take a position this phase — it shapes what a frame allocator's API owes Phase 5's handle model |
| Page-table bit packing as a host-tested crate | `005-code-conventions.md` names it; D-30 is the pattern | A new `libs/` crate, tested against SDM ch. 4's entry layouts before any table is walked |

Not this phase's debt, recorded so nobody mistakes it: the handle-table and
rights-taxonomy doc and the queue-layout doc are owed **before Phase 5**
(`002`, open questions), and the 100 Hz tick stays until Phase 4's tickless
scheduler (D-15).

## Order of work (proposal)

The order is forced the same way Phase 2's was: nothing can build tables
without frames, nothing should switch CR3 until the new tables reproduce
every mapping the kernel is standing on, and nothing can allocate until the
switch has survived.

| Slice | Contents | Status |
|---|---|---|
| 1 | Boot handoff grows the memory map + HHDM; physical frame allocator | |
| 2 | Page-table encoding in `libs/`; `wost` builds its own tables (kernel, stacks with guard pages, framebuffer, direct map), W^X-correct; CR3 switch | |
| 3 | Protection completion: W^X verified by read-back, zero-on-allocate; KASLR decision executed or deferred on the record | |
| 4 | Kernel heap (`alloc` enabled); console back-buffer retires the scroll read-back | |
| 5 | Device mappings: I/O APIC up, IRQ 1 routed, polled keyboard drain retires (D-37) | |

Memory *objects* — the kernel-object substrate Phase 5's handles will name —
are scoped at phase start: either a sixth slice here or the opening of
Phase 5, decided when the untyped-retype question is answered.

## Decisions the phase must make (with their tensions)

Held open deliberately; each becomes an ADR when taken.

- **Frame allocator shape.** Bitmap, buddy, or staged (bump-until-heap, then
  real)? And the D-13 boundary question: table *mutation* is `wost`'s, but
  allocator *policy* is arithmetic — how much lives kernel-side as safe code
  over a `wost` frame type, on the Asterinas OSTD precedent?
- **Table access mechanism.** Direct-map walking (the modern standard, and
  what HHDM hands us) vs recursive mapping. The direct map's return is
  already decided (D-33); its exact contract — what is mapped, what is
  deliberately not, huge pages or not — is not.
- **Heap allocator.** Hand-rolled (the sha256/QMP no-dependency precedent)
  vs a vetted crate (`linked_list_allocator` et al.) — the first real test
  of the dependency policy against `alloc`-ecosystem convention. Whichever
  wins, the choice is an ADR with the other on the record.
- **KASLR now or later.** Needs the image relocatable; weigh the linker
  work now against carrying the D-12 debt another phase.
- **Framebuffer memory type.** The console currently writes through
  Limine's mapping; our tables must reproduce or improve it (write-combining
  via PAT is the conventional answer, and measurably changes scroll cost).
- **Untyped-retype.** Adopt, adapt, or reject seL4-style caller-donated
  kernel memory — the answer shapes every kernel-object API after this.
- **Migration hazard to design around, not decide:** at the CR3 switch the
  kernel is executing from, logging through, and drawing on mappings the
  old tables provided. The new tables must reproduce the kernel image,
  stacks (including the double-fault stack), framebuffer, and nothing else
  writable+executable — and the switch is the one moment a mistake reports
  as a triple fault rather than a page fault. The fault path from slice 1
  of Phase 2 is the safety net; order the enable so it stays one.

## How it will be checked

The Phase 2 pattern carries over whole: encodings host-tested against
transcribed SDM tables before hardware sees them (D-30); every new
mechanism lands with an on-machine check in `kernel/src/selftest.rs`
(D-28), each broken once to prove it can go red; QEMU is the loop and a
VMware boot ends the phase (D-18). Expected machine checks, refined per
slice: the loaded CR3 is ours; every executable mapping is read-only and
every writable one NX (walked and read back, not assumed); a guard page
faults — checked structurally by read-back, since the fault itself is
terminal; allocation returns zeroed frames; heap alloc/free round-trips;
the I/O APIC delivers IRQ 1 end to end, which inherits the QMP keystroke
harness of D-38 unchanged.

## Where the code changes (breadcrumbs for the next session)

- `wost/src/boot/mod.rs` — `BootInfo` is deliberately minimal (base
  revision + framebuffer); the memory-map and HHDM requests are added to
  `entry!`, `BootInfo`, and `limine_glue.rs` only. The deleted
  `direct_map` field's history is D-33.
- `wost/src/arch/x86_64/cpu.rs` — `read_cr4`/`write_cr4` are private and
  their comment says why; CR3 ownership lands beside them in `wost`
  (D-13: paging is TCB).
- `wost/src/mmio.rs` — `MmioRegion` is the checked-MMIO abstraction the
  framebuffer already uses; device mappings should hand out regions
  through it, paired with a claim discipline like `port::claim`'s — which
  since D-39 means covering the span an access *reaches*, not the address it
  names. The bookkeeping for that lives in `libs/port-space`, where it is
  host-tested; a physical-address equivalent belongs beside it rather than
  inside `wost`.
- `wost/src/arch/x86_64/apic.rs` — module doc records the x2APIC decision;
  the I/O APIC is a *separate* controller and a new driver, not a revisit
  of this one.
- `kernel/src/main.rs` — the idle loop's `input::poll()` line is the
  polled-keyboard call site D-37 retires; `start_interrupts` is the
  ordering pattern to extend, not rearrange.
- `kernel/src/console.rs` — `Console::scroll`'s read-back is the
  back-buffer's motivation (`001-boot.md`).
- `libs/` — the new page-table crate joins `members`, `default-members`,
  `[workspace.dependencies]`, and `HOST_PACKAGES` in `xtask/src/paths.rs`,
  like `x86-pit`, `ps2-keys` and `port-space` before it.
- `kernel/linker.ld` and `xtask/src/elf.rs` — the script now names its own
  orphans and the build checks the result (D-40). Phase 3's W^X work has a
  head start there: the image is already three segments with no
  writable-executable one, asserted after every build, so what the new page
  tables must reproduce is written down and machine-checked rather than
  inferred. KASLR still waits on making the image relocatable, and that is a
  change to this script.
