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
Limine protocol (memory map and HHDM features, base revision 4 — D-42); Asterinas
OSTD's frame-ownership model (the framekernel precedent D-13 follows); seL4
untyped/retype (the accounting question `002` poses); OSDev paging and
higher-half pages.

## What earlier phases left owing

Every debt below is on the record elsewhere; this table is where a future
session finds them collected.

| Debt | Where recorded | What Phase 3 does |
|---|---|---|
| W^X for every mapping | D-12, D-32 ("waits for Phase 3") | Build the tables W^X-correct from the first CR3 write; NX has been on since slice 1 precisely so this is enforceable |
| Guard pages under all stacks, zero-on-allocate | D-12 | Zero-on-allocate lands here. Guarded ordinary thread stacks land in Phase 4 (D-52); the boot and double-fault stacks remain Phase-5 debt |
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
| 1 | Boot handoff grows the memory map + HHDM; physical frame allocator | ✅ 2026-08-22 |
| 2 | Page-table encoding in `libs/`; `wost` builds its own tables (kernel image, framebuffer, direct map), W^X-correct; CR3 switch. Guarded thread stacks followed in Phase 4 (D-52) | ✅ 2026-08-22 |
| 3 | Protection completion: W^X verified by read-back, zero-on-allocate; KASLR decision executed or deferred on the record | ✅ 2026-08-22 |
| 4 | Kernel heap (`alloc` enabled); console back-buffer retires the scroll read-back | ✅ 2026-08-22 |
| 5 | Device mappings: I/O APIC up, IRQ 1 routed, polled keyboard drain retires (D-37) | ✅ 2026-08-22 |
| 6 | Framebuffer write-combining, on slice 4's back buffer (D-48) | ✅ 2026-08-22 |

Memory *objects* — the kernel-object substrate Phase 5's handles will name —
were moved to the opening of Phase 5 when D-44 answered the untyped-memory
question. Slice 6 here became write-combining instead.

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
every writable one NX (walked and read back, not assumed); allocation returns
zeroed frames; heap alloc/free round-trips; and the I/O APIC delivers IRQ 1 end
to end, inheriting D-38's QMP keystroke harness. Guard-page read-back moved with
ordinary thread stacks to Phase 4; the boot and double-fault guards remain due
in Phase 5.

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

## Slice 1 — the handoff and the frames (2026-08-22)

Recorded as built, per the charter above.

**The base revision moved first, and it was not the change it looked like.**
The kernel had been asking for Limine base revision 3 by accident —
`BaseRevision::new()` is `with_revision(3)` — and this file's own debt table
inherited a consequence of that default as though it were a fact about the
bootloader: *device holes unmappable, the wall itself*. Base revision 4 is the
revision that lifts it, and reaching it turned out not to require the `limine`
0.6.x upgrade `001-boot.md` lists as open: that crate still needs nightly, but
the base revision is a protocol parameter the pinned 0.5.0 crate already
exposes. One argument, in the one file allowed to name the bootloader (D-42).
Verified by boot, which is the only way it could be.

**The handoff.** `BootInfo` grew `memory_map` and `direct_map`, both
`Result`s — a kernel that cannot allocate is still worth booting far enough to
say why. `MemoryMap` **copies** the bootloader's entries rather than borrowing
them, because the map lives in bootloader-reclaimable memory and a kernel that
means to reclaim that memory cannot also hold a reference into it. Region types
are translated in `limine_glue` against the crate's own constants, so a value
moving underneath us is a compile error; anything unrecognised becomes
`Unknown(n)` and is reported, which is how base revision 4's new type would
announce itself. On QEMU with OVMF it has not appeared — every region so far
is one of the eight the pinned crate names.

**The allocator** is D-43: `libs/frame-space` holds the arithmetic and knows
only frame indices, `wost::mm` holds the lock and the physical access and hands
out `Frame` and `Segment` handles whose `Drop` releases them. Nothing computes
the kernel image's extent — only `Usable` regions are inserted, so the image,
the bootloader's structures and the ACPI tables exclude themselves, the
megabyte of `.bss` bitmap along with them. First boot: 459 MiB usable in
117,734 frames, direct map at `0xffff800000000000`.

**What the checks caught.** The first version of "allocation returns a zeroed
frame" passed with the zeroing removed. It short-circuited on an address
comparison, and the allocator's hint means a released frame is rarely the next
one handed out, so the escape hatch was always taken. D-28's rule — break every
new check once — is the only reason that was found. Its replacement writes
`0xAA`, requires the kernel to see it, zeroes, and requires the kernel to see
that; removing `Frame::fill`'s write turns the harness red. The lesson is
narrower than "test your tests": a check whose failure path can be reached by
an *early return* is not a check, and both of those returns looked like
defensive programming.

**Still owed by this slice's neighbours.** The direct map is used here only to
reach frames; the *contract* for the one the kernel builds itself — what is
mapped, what is deliberately not, huge pages or not — is slice 2's, and D-42
does not settle it. Base revision 4 removes the wall in front of device holes;
it does not remove the reason to map devices through `MmioRegion` with an
explicit memory type.

## Slices 2 and 3 — the kernel's own tables (2026-08-22)

**The encoding went first, and told the truth about itself.** `libs/x86-paging`
carries the entry bits, the index each level takes from a virtual address, and
the canonical-form test, transcribed from SDM 3A §4.5. Its tests restate every
bit position *from the manual* rather than from the constants above them,
which is the only arrangement in which a transcription error fails instead of
agreeing with itself. Nineteen host tests and a doctest before any hardware saw
a table (D-30).

**The switch is a safe function**, and that is D-45's substance rather than its
packaging. `activate` walks the *new* tables for its own code and its own stack
before loading them, and refuses a table that would not survive. The one moment
this file warned "reports as a triple fault rather than a page fault" now
reports as a `Result`. It worked on the first boot, which is not evidence the
check was unnecessary — it is what the check was for.

**The direct-map contract changed under contact, and the change is the
lesson.** `008-research-notes-2.md` §3.3 proposed *usable RAM only*. That would
have triple-faulted immediately: the kernel is executing on a stack the
bootloader put in bootloader-reclaimable memory and logging through a
framebuffer region. The contract is now *everything that is memory* — usable,
bootloader-reclaimable, kernel-and-modules, framebuffer, and both ACPI kinds —
and *nothing that is not*, so device holes stay out and are mapped
deliberately later. The excluding half of the research recommendation was
right; the including half was written without asking what the kernel was
standing on.

**W^X closes D-12's oldest line.** Six linker symbols bound the three
protection classes, so the image is mapped by what each part may do: text
read-only and executable, rodata read-only and not, data writable and not.
There is no fourth combination. The check walks the live tables at three points
per section — a run of 2 MiB pages can be right at its edges and wrong between
them — and broke correctly when `.data` was mapped executable on purpose.

**Both slice-3 checks found bugs, and both bugs were in the checks.** The W^X
probe computed a section's midpoint as `(start + last) / 2`, which overflows
`u64` on a pair of higher-half addresses. And `activate` validated the tables
before testing its own already-active latch, so the second-switch check got a
correct refusal for the wrong reason. Neither would have been found by reading.

KASLR is deferred, on the record, with its cost named (D-46): it is the only
baseline item deferred as far as Phase 12, and every phase that adds a mapping
makes the relocation work more expensive. Ordinary thread-stack guards arrive
in Phase 4; the boot and double-fault guards remain Phase-5 debt.

## Slice 4 — the heap (2026-08-22)

The charter framed this as hand-rolled versus a vetted crate. D-45 answered it
first, and from an unexpected direction: `activate` consumes the address space
so nothing can map into a live table, which is what makes `AddressSpace::map`
safe — and a heap that grew by mapping pages would have to undo that.

So the heap lives in the direct map (D-47). It takes frames and addresses them
through a mapping that already exists, writable and never executable; a
physically contiguous run is contiguous there too, so a large allocation is one
`Segment` and one address. No page-table entry is ever added for the heap, and
it grows to whatever the machine has rather than to whatever a boot-time
reservation guessed. Hand-rolled won, but on the boundary rather than on the
no-dependency principle: none of the surveyed crates would have known to
allocate this way, because none of them knows this kernel made mapping
one-way.

`libs/slab-layout` carries the class arithmetic — eleven host tests, including
one that asserts the invariant the whole design rests on, over every slot of
every class: alignment is met by *choosing a class*, never by padding.

**The check that had to be rewritten.** "A freed slot is reused" was first
written the obvious way — allocate, note the address, free, allocate again,
require the same address. It failed. Adding a `debug!` between the two
allocations made it pass. That is the signature of a check measuring the
optimiser: nothing obliges the compiler to keep two allocator calls in source
order when the values between them are dead. The replacement counts rather
than compares — `live` moves and returns, `slab_frames` stands still — and
depends on neither addresses nor codegen.

That is twice this phase that the defect was in a check rather than in the code
under test, and both times the check was found by D-28's break-it-once rule
rather than by reading. The pattern in both: an assumption that looked like
defensive programming (an early return, an identity comparison) quietly made
the failure path unreachable.

**Still open in this slice.** The console back-buffer, and with it the
write-combining decision that `008-research-notes-2.md` §3.4 argues is the same
decision — WC makes framebuffer writes fast and reads catastrophic, so the
back-buffer is the precondition that makes WC adoptable rather than a
complement to it. Neither is done; both are the first thing slice 4's remainder
should take.

### Slice 4, completed — the back buffer (2026-08-22)

`Framebuffer` now keeps a shadow of itself in RAM: one word per pixel at the
device's own stride, written alongside every device write and read *instead of*
the device. `001-boot.md`'s oldest known limitation is retired. Attaching seeds
the shadow from the screen, so the lines already printed survive, and that read
is the last one the design permits.

**Write-combining does not land with it, and the reason is a boundary the
research pass did not have.** `008-research-notes-2.md` §3.4 argued the two are
one decision — correctly, because WC makes reads catastrophic and so depends on
the back buffer. But WC needs the kernel to map the framebuffer *itself* with
PAT bits, and D-45 deliberately keeps device holes out of the direct map so
that devices are mapped on purpose. That mapping is slice 5's. The coupling was
identified correctly and the sequencing loosely: the dependency runs
back-buffer → WC, so the back buffer alone leaves nothing wrong, while WC alone
would have made every scroll far worse. WC moves to slice 5 with its argument
intact and its precondition now satisfied.

## Slice 5 — the wall comes down (2026-08-22)

D-33 called the unmappable device hole "the wall itself". It is down: the I/O
APIC is mapped, IRQ 1 is routed, and the keyboard interrupts rather than being
polled from the tick. D-37 is retired.

**The ordering was forced, not chosen.** D-45 made mapping one-way, so a
device's window has to be known while the tables are still being written — but
devices are discovered by reading ACPI. The two only reconcile one way: read
ACPI *before* building, while the bootloader's direct map still covers the
tables, and redeem the claim on the window after the switch. That constraint
turned out to be a feature. A device the kernel has not decided to map is not
merely unmapped; it is unmappable, and `DeviceMapping` — neither `Copy` nor
`Clone` — is what makes handing out an `MmioRegion` for one a safe function.

**Firmware is the least trustworthy input the kernel has**, so `libs/acpi-tables`
validates before it trusts and refuses rather than panics: seventeen host tests
including a zero-length entry that would otherwise loop forever, an entry
running past its table, and a truncated entry of a known kind. Interrupt source
overrides are honoured — assuming `irq == gsi` is the classic way to program an
I/O APIC that then delivers nothing.

**The check that had never failed.** "An injected keystroke arrives" passed at
every step of this work, including the steps where nothing was being delivered
by interrupt at all — because `await_key` polled the controller itself, so it
proved a keystroke arrived and said nothing about *how*. Rewritten to watch
what the handler delivered, it went red at once: the line was routed, the
vector was right, and the i8042 had been configured by Phase 2 to raise no
interrupts (D-37, correctly, at the time).

That is the third check this phase whose shape hid a defect — after D-43's
identity comparison and D-47's dependence on allocator call ordering — and the
three together are the phase's most transferable lesson. **A check that has
never failed is not evidence; it is an untested claim.** Two of the three were
caught by D-28's break-it-once rule. The third was caught by asking what a
passing check would still permit.

## Slice 6 — write-combining (2026-08-22)

The page attribute table's second entry becomes write-combining — it held
write-through, which this kernel never asks for — and the framebuffer's one
direct-map alias selects it (D-50, amended by D-77). The audit rejected the
original second mapping: two simultaneous cache types for one physical page
can violate the processor's aliasing rules. The transition changes only PAT
entry 1, reloads the tables, and restores the previous entry if activation
fails.

The shadow still matters: after attachment, no console read goes to the
write-combining device mapping.

The check has two halves and needs both. The PAT entry must actually be
write-combining — a hypervisor may decline the MSR write, so it is read back
rather than remembered — and the mapping must select it. Either alone leaves
writes crossing the bus one at a time while the log says otherwise.

## The implementation, closed; the VMware record missing

Every debt this file collected was either paid or explicitly transferred. W^X,
zero-on-allocate, the direct map, device holes, the polled keyboard, the scroll
read-back, and the page-table crate landed here. D-44 answers untyped memory and
moves memory objects to Phase 5. Ordinary thread-stack guards land in Phase 4;
the boot and double-fault guards remain Phase-5 work. **KASLR remains deferred**
to Phase 12 with its cost named by D-46.

The close recorded QEMU checks but no dated VMware result. D-18 still makes a
VMware boot part of the definition of done; the roadmap therefore labels this
phase's implementation complete while preserving that documentary evidence
gap rather than inventing a verification record after the fact.

The phase's most transferable lesson is not about memory. Three separate checks
hid defects in their own shape: D-43's zeroing check short-circuited on an
identity comparison, D-47's reuse check depended on the compiler keeping two
allocator calls in source order, and D-49's keystroke check measured the polled
path while the interrupt path delivered nothing. All three looked reasonable.
Two were caught by D-28's break-it-once rule; the third by asking what a
passing check would still permit. **A check that has never failed is not
evidence; it is an untested claim.**
