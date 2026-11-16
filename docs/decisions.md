# Decisions

`docs/decisions.md` is Bedrock's decision log: every decision that still governs the kernel, one
entry each, grouped by subsystem. Ids are stable; a gap is a retired decision, kept in git history
at tag `v0.3.0`. A new decision takes the next free
number and is added here, as one entry or as an amending id on the entry it changes.

## Language and boundaries

- **D-01** The kernel is Rust on the stable channel with no nightly feature, because the borrow
  checker removes use-after-free and data races in ring 0.
- **D-13** All `unsafe` lives in `wost`, the one privileged crate (boot, paging, MMIO, port I/O,
  DMA, context switch, interrupt entry), and every other crate is safe Rust over its abstractions,
  so the memory-safety trusted base stays small. `kernel/` carries `deny(unsafe_code)`, not
  `forbid`, because `wost::entry!` expands `allow(unsafe_code)` boot glue into it; that glue,
  `linker.ld` and `build.rs` stay in the binary crate because a linker may drop `#[used]` statics in
  an rlib (D-21).
- **D-14** Faults are contained by restarting a service under a supervisor, never by unwinding: a
  kernel panic reports and halts, and address spaces and rings are the isolation boundary for user
  code.
- **D-30** Bit layouts and arithmetic with no hardware behind them live in host-tested `libs/`
  crates and `wost` only loads the result, because a wrong descriptor bit shows up only as a triple
  fault; `libs/x86-tables` holds the GDT, IDT and TSS encodings.
- **D-41** `unsafe` enters `wost` for three reasons only: a novel abstraction, a measured
  performance need, or FFI and the platform; every export is sound against any safe caller. A safe
  instruction adapter takes the narrowest typed operand that proves its precondition, the raw
  instruction is visible only to its sibling module, and `wost` has no unrestricted `pub unsafe fn`
  (D-266).
- **D-480** The ring 3 runtime `witness/rt` is the second crate that may write `unsafe`, beside
  `wost` and not a `wost` feature, because ring 3 needs `_start`, the `int 0x80` gate, the ring view
  and probe instructions but none of the kernel's statics, Limine glue or locks. Each export is sound
  for any caller: a probe is the instruction that ends the program, the ring view is handed out
  once, and a jump is taken only into the kernel half. Its `entry!` expands `allow(unsafe_code)`
  glue into the witness, which therefore carries `deny`, not `forbid` (amends D-13).
- **D-593** `witness/rt` sets a user-half thread base only to a `&'static u64`, and its `call`
  faults on any other user-half `ThreadBase`, rather than making `thread_word` unsafe, so the
  witness stays free of `unsafe` and `fs:0` never reads memory a live reference covers.
- **D-183** Code that runs in kernel context is Rust, and no language runtime, interpreter or
  compiler enters the kernel beyond the extension checker `docs/roadmap.md` plans. First-party
  software that runs on the kernel is Wizzy, with exceptions only for minimal runtime shims, host
  tooling, third-party ports and foreign runtimes.
- **D-206** Malformed external input (ring entries, images, firmware tables, filesystem and device
  data) ends in a typed error, never a panic, and `unwrap` and `expect` are denied in `kernel`,
  `wost` and the parsing crates. An internal contradiction that breaks ownership or an unsafe
  precondition fails stopped in every build through `assert!`, `panic!` or `unreachable!`, never
  `debug_assert!`, and an operation that can refuse is `#[must_use]` (D-249, D-356).
- **D-248** A mechanism ships only when a current caller uses it, and the shipped `Channel` is one
  opaque bootstrap identity. Test weights, fault injection, observers and the writes that feed them
  compile only under the `selftest` feature, so test code never widens the shipped API (D-290).
- **D-366** Bedrock owns kernel mechanisms, `wost`, hardware drivers, the kernel ABI and diagnostic
  boot images; Realm owns the operating environment, platform semantics, Linux adapters, Wizzy
  services, shell, desktop, launch policy and product images; Ink owns the language, compiler and
  runtime. Each project builds from its own committed inputs, and crate names, `wizard-platform-v1`,
  `x86_64-unknown-wizard` and wire identifiers change only by an ABI decision.

## Boot

- **D-03** The kernel boots UEFI-only through the vendored Limine in `boot/limine/`, because Limine
  hands over long mode, the higher-half load, the memory map, the framebuffer and modules
  identically on QEMU, VMware and hardware. It requests base revision 4, which puts ACPI tables and
  reserved regions in the direct map, and treats an unknown memory-map kind as reserved (D-42);
  there is no BIOS path and no bootloader of our own (D-219).
- **D-92** The boot thread moves to its own guarded stack through `task::continue_on_stack` as soon
  as the tables are live, and the double-fault handler runs on its own guarded IST1 stack (D-89),
  because a stack inside the large-page direct map cannot be guarded. Every kernel stack has an
  unmapped page beneath it, made only by `AddressSpace::map_stack` and
  `AddressSpace::map_pinned_stack`.
- **D-224** `libs/memory-map` accepts 1 to 128 regions and refuses a larger map whole, so the
  allocator and page tables are never built from a truncated map. A query consults every region
  holding the cursor and refuses wherever an unadmitted kind overlaps, so an overlapping or unsorted
  firmware map cannot change a verdict by its order (D-241).
- **D-468** The bootloader's memory map and direct-map offset reach the kernel as one `BootMemory`,
  minted once by `wost::boot` and neither `Copy` nor `Clone`. `EarlyFirmwareMemory` borrows it and
  `mm::init` consumes it, so a `MemoryMap` a safe caller builds starts no allocator and opens no
  firmware memory (D-64, D-251).

## CPU, traps and protections

- **D-12** Protections are on from the first boot: W^X for every mapping, NX, SMEP, SMAP, UMIP and
  `CR0.WP` and a guard page under every stack, because retrofitted mitigations end up with
  exceptions. They are enabled after the IDT is installed and read back, so a bad bootloader
  mapping becomes a reported fault (D-32, D-186), and `map_range`, the path under every dynamic
  mapping door, refuses a writable executable flag set before it touches a table (D-376).
- **D-29** Interrupt entry is hand-written assembly in `wost`, generated stubs that normalise the
  frame and a common tail that calls a Rust dispatcher, not nightly's `abi_x86_interrupt`, so the
  kernel stays on stable.
- **D-31** A fault taken in ring 0 is a kernel bug: it panics with the vector, error code, `RIP`,
  `CS`, `RFLAGS` and `RSP` and nothing is retried, while a breakpoint resumes. A trap handler
  never logs, because the interrupted code may hold the console lock; it records a number that the
  idle thread reports (D-122).
- **D-33** The local APIC runs in x2APIC mode only, through MSRs with no device page. Takeover
  software-enables the APIC before unmasking its error entry, masks and reads back every LVT entry
  it knows, and leaves an advertised entry beyond those as firmware set it (D-375, D-455), and the
  SVR must read back as the typed spurious
  encoding before readiness is published (D-257).
- **D-55** `wost` keeps a trap-depth count that follows each thread, and the heap refuses to run
  while it is non-zero, so nothing allocates in a handler that may have interrupted the allocator.
- **D-81** Trap entry has FRED's shape without FRED: the kernel dispatches on a `TrapCause` decoded
  by `libs/trap-cause` and handlers see an opaque frame. Enhanced IBRS is enabled at boot, because
  the return path is costly to change once mature.
- **D-82** Extended state is saved eagerly, never lazily, into a per-slot area sized at boot from
  `CPUID.0DH`, with `XSAVEC` where present and `XSAVE` otherwise, beside `CR3` and `RSP0` on every
  switch (D-108). `XCR0` enables the x87, SSE and AVX components the processor has, with x87 and SSE
  required (D-456), a fresh slot holds the
  architectural zero image, and process construction and the final ring-3 crossing ask one live
  predicate of `CR4.OSXSAVE` and `XCR0` (D-270); the kernel is built `+soft-float`.
- **D-460** Boot establishes its control state after the IDT loads instead of inheriting it:
  `CR0.NE` and `CR4.MCE` set, `CR0.EM` and `CR4.TSD`, `DE`, `PCE` and `FSGSBASE` clear and `RFLAGS`
  reset, each read back. Every bit the kernel does not own stays as the bootloader set it. Each
  thread switch reloads the data selectors, so one program's selector never reaches the next.
- **D-471** Boot sets `CR4.TSD` and writes zero to `IA32_TSC_AUX` where it exists (amends D-460),
  and process construction refuses while `TSD` reads clear. A world without the raw time right
  must not read the counter, so `RDTSC`, `RDTSCP`, `TPAUSE` and `UMWAIT` fault in ring 3 and the
  thread is retired. A trapped read is refused, never emulated, because an emulated read costs a
  kernel entry per call where a value on a clock page costs a load.
  `RDPID` cannot be trapped, so it reads a zero `IA32_TSC_AUX`.
- **D-91** `libs/x86-cpuid` decodes `IA32_ARCH_CAPABILITIES` and decides the speculation posture,
  and `wost` keeps only the `cpuid` and MSR reads; an absent capability word is never read as
  immunity.
- **D-465** Where the processor enumerates IBPB, `task::prepare` issues it on every change of user
  root, with no per-process opt-in, because worlds are mutually hostile by default and a selective
  barrier would make isolation a policy setting. Kernel-only threads do not count as a root, and a
  released root is forgotten so a recycled root frame is a new space.
- **D-96** Ring 3 enters the kernel through one DPL-3 interrupt gate at vector `0x80`, not
  `syscall`, which would need `swapgs` and a per-CPU block. `EFER.SCE` and `IA32_SYSENTER_CS` are
  cleared and `enter_user` refuses while either is set (D-139); `enter_user` walks every
  precondition and zeroes the general registers on the way out.
- **D-475** `task::enter_user` puts the program's argument in `rdi` and zeroes every other general
  register (amends D-96).
- **D-97** `copy_from_user` and `copy_to_user` are the only code that opens the SMAP window, for one
  copy. Each walks every page and requires effective `USER` permission before the window opens
  (D-279), checks ranges by comparison with the user limit, and recovers a fault inside its assembly
  at a linker fixup carried as an opaque target that only the trap adapter writes (D-120, D-252);
  every trap entry executes `clac` when SMAP is on (D-101).
- **D-98** The return to ring 3 executes `VERW` unless `IA32_ARCH_CAPABILITIES` reports
  immunity to every flaw it clears (MDS, TAA or no TSX, the MMIO stale-data classes and RFDS), on
  the first entry and on every return (D-101, D-457); an unreadable capability word never counts as
  immune.
- **D-102** A fault taken from ring 3, judged by the pushed `CS`, retires the faulting thread and
  the machine continues; a double fault, machine check or NMI stays terminal.
- **D-193** Ring 3 opens only when NX is enabled and read back, the clock is calibrated, the boot
  thread runs on a guarded stack, the service worker exists and extended state is isolated. Process
  construction checks each before allocating and returns a typed refusal, and the boot otherwise
  continues with diagnostics only (D-201, D-203).
- **D-467** Ring 3 also opens only once the kernel's random generator holds 256 credited bits
  (amends D-193), so no program runs before randomness can be served to it. Only 64-bit `RDSEED`
  is credited, and `RDRAND` only on a processor without `RDSEED`, because `RDRAND` is a conditioned
  generator's output rather than a seed and the 16-bit and 32-bit `RDSEED` forms can report success
  with a zero value.
  The boot log names the source.
- **D-205** The MADT inventory keeps at most sixteen I/O APICs and refuses a larger one whole. A
  window's version register must read 0x11 to 0x20 with at most 240 entries before any write
  (D-350), an entry is masked by writing and reading back the inert low word before its destination
  is touched (D-276), and every line stays masked and the keyboard is polled (D-372).
- **D-250** Ring 3 runs with the kernel higher half in its tables only for an affirmative `RDCL_NO`
  or the exact QEMU TCG hypervisor tuple, decided once in `libs/x86-cpuid`. Otherwise it enters only
  through a KPTI pair: a full root and a stripped root that shares the user subtrees and maps only a
  linker-bounded entry enclave; entry assembly loads the full root before any Rust, exit assembly
  loads the stripped root after the last secret access and a stack scrub, PCID stays off, and one
  `AddressSpace` owns both roots and returns them together when it is reclaimed (D-407).
- **D-251** Permission that depends on hardware state is a non-`Copy`, non-`Clone` token minted only
  from a fresh observation of that exact device: `ControllerStopped` after `CC.EN = 0` and `CSTS.RDY
  = 0`, `ExternalControllersMasked` after the I/O APICs are masked, `EoiToken` after the live ISR
  bit is latched and `DisarmedTimer` after a zero count (D-267), so a caller cannot restate the
  conclusion.
- **D-277** `libs/x86-apic` owns the only assignable-vector type, which excludes vectors 0 to 31 and
  the remapped 8259 range, and `kernel/src/arch/x86_64/vector.rs` holds the only production
  assignment; IDT, local APIC and I/O APIC programming take those typed values.
- **D-289** Common trap entry loads `KERNEL_DATA` into `SS` before entering Rust, because a
  privilege-changing interrupt leaves `SS` null and a context switched to from the trap would
  inherit it.
- **D-368** Every return to ring 3 is validated immediately before the common `iretq`: exact user
  selectors, an executable `RIP` and an `RSP` in the user half, and `RFLAGS` with `NT`, `VM`, `IOPL`
  and reserved bits clear. A refused return retires that program and is never attempted, because a
  faulting `iretq` is charged to ring 0.
- **D-426** Every GDT code and data descriptor is written with its accessed bit set, so the
  processor never writes a descriptor table that the stripped root maps read-only.
- **D-570** Each processor's GDT, TSS, paired-root route, transition stack and fault stacks live in
  an entry area at a fixed window plus 128 KiB per processor, mapped from frames allocated at
  bring-up and never unmapped, rather than in static arrays sized for a maximum processor count.
  `IA32_GS_BASE` names the area in ring 0 and is zero in ring 3, with `swapgs` and `lfence` on
  entry from ring 3 and `swapgs` just before `iretq` on return. A stripped root copies the areas
  that exist when it is built, so the first one closes the set (amends D-92 and D-250).
- **D-571** The NMI, double-fault and machine-check vectors run on IST2, IST1 and IST3 of their
  processor's area, decide `GS` by reading `IA32_GS_BASE` instead of the pushed `CS`, and never
  resume, so a half-swapped `GS` is never returned to. Only the transition stack is scrubbed on a
  paired entry and exit, because nothing that pushes onto a fault stack resumes ring 3.
- **D-573** Other processors start through Limine's MP request with x2APIC, only those the MADT
  also lists, and on a processor that is not immune to MDS-class leaks only thread zero of each
  core. Each copies the boot processor's PAT, control state, protections, extended state and
  speculation controls, refusing any difference, then parks with interrupts off until the scheduler
  can run threads on it. The online set closes when the first user root is built.
- **D-574** A panic sends an NMI to every other online processor by APIC ID, never the all-but-self
  shorthand, and forces locks only after every one acknowledged within a bounded poll; an NMI
  during a panic is a stop that never resumes (amends D-34).
- **D-575** The gate's release self-test and shipped boot run on two QEMU processors and require
  the `processors` boot record to count both, the debug self-test and the degraded boots run on
  one, and the VMware machines have four virtual CPUs.
- **D-576** Page-table entries are atomic words. A shootdown takes one global generation, sends a
  fixed vector by APIC ID to every other online processor for a kernel-half change and to the
  processors whose published root is the space's for a user-half one, and each target answers
  with a whole flush, so a `Flush` completes when every target recorded its generation. One
  generation and a published root suffice while PCID is off; per-space generations and slots come
  with PCID (U8).
- **D-577** A direct-map upgrade to writable sends no shootdown; a write through the stale entry
  faults and the kernel's fault path resumes it when the live leaf is writable. A wait for a
  shootdown happens only with interrupts on and no spin lock held, and a drop that cannot wait
  refuses by panic, never by releasing a frame or table early.
- **D-578** The local APIC's lifecycle and EOI claim are per processor. A parked processor
  configures its own APIC with the boot vectors, timer masked, and halts with interrupts on, so it
  answers kernel-half shootdowns; it never loads a user root.

## Memory

- **D-43** The frame allocator splits mechanism from policy: `wost::mm` owns `Frame`, `Segment` and
  the lock, and `libs/frame-space` owns the bitmap search. Only memory the bootloader marks usable
  is managed, and releasing a foreign or free frame is a visible refusal.
- **D-44** Memory that user space, DMA or a device can modify has its own type, `mm::USegment`, that
  admits only plain-data copies; `map_user` takes only a `USegment` and `map_owned` refuses `USER`
  (D-107). A `USegment` is zeroed before its frames return on every drop path, a run whose clear
  refuses is withheld, and a refused `map_user` returns it unchanged (D-331), so ring-3 bytes never
  reach the free pool.
- **D-469** A `USegment` is made only by `USegment::zeroed`, which zeroes the run it consumes, and
  the kernel writes a program's bytes through it before mapping. Zeroing at the conversion, rather
  than trusting each caller or zeroing every freed frame, keeps one place that proves ring 3 sees no
  earlier owner's bytes (amends D-44).
- **D-470** The cache colour count is the L2's page colours from `CPUID` leaf 4, bounded by the
  last-level cache's, rounded down to a power of two and capped at 64 (`libs/x86-cpuid`). A sliced
  last-level cache reports sets across every slice, and the slice hash consumes some of those set
  bits, so colouring by them is unsound; the private L2 is unsliced. Without an L2 there is one
  colour.
- **D-45** `activate` is safe: it walks the candidate tables for its own code, stack and direct map
  before writing `CR3`, and the kernel keeps the address space so mappings can be added to a live
  space, where a present entry is never rewritten (D-86, D-458). Activation refuses while
  `EFER.NXE` is clear, where a higher-half top-level entry is user-accessible, or where a
  framebuffer or firmware byte handed out at boot is not direct-mapped (D-458). The direct map
  covers memory only, in whole pages for framebuffer and firmware regions, at the bootloader's
  offset with 2 MiB pages, and the image is mapped by linker-symbol
  class so no page is writable and executable.
- **D-476** Every managed frame has one 8-byte record naming its owner (free, kernel, page table,
  device, or one arena with the frame's kind) and whether it is writable or sealed, in one table
  carved from managed memory at `mm::init`. The record carries no reference count or owner list:
  an arena's frames are exactly those whose record names it, so the invariant stays per frame and
  teardown finds them by a bounded scan (amends D-43).
- **D-477** Usable memory is direct-mapped with one 4 KiB leaf per frame and every direct-map leaf
  is non-executable, so sealing a frame rewrites one leaf that every root shares and the image has
  no executable alias. The page-table cost is one table frame per 2 MiB of usable memory (amends
  D-45).
- **D-478** A sealed frame is read-only in every mapping, the direct map included, and has no
  unseal: it becomes writable only by being released, record first, then leaf, then zeroing.
  Executable user memory is mapped only from a `SealedSegment`. Each direct-map change returns a
  `paging::Flush`, the one token Phase 8's shootdown completes (amends D-12).
- **D-479** An arena is a per-colour reservation, not a set of donated frames. The unused
  reservations of each colour never exceed that colour's free frames, kernel allocations take only
  unreserved frames, and an operation earmarks before it takes, so a reserved frame can always be
  taken and a refused request changes nothing. Huge-page and contiguous needs are reserved
  physically when they arrive.
- **D-495** `USegment::seal` and `ArenaFrame::seal` return an `mm::Sealing` that yields the
  `SealedSegment` or `SealedFrame` only once the seal's own flush has completed, and until then can
  only be read (amends D-478). The caller merges the seal's flush into its own, so a memory object
  seals every page under its object lock, waits once after the lock and finishes each page. A seal
  that waited before returning would wait under that spin lock.
- **D-496** Nothing under the kernel space's spin lock waits for a shootdown (amends D-577). A
  mapping links every table it needs before it writes a leaf, so a refusal leaves empty tables in
  the space until reclaim and has no leaf to clear; `map_stack` checks the guard before it maps;
  and `unmap_device` returns a `RetiringDevice` whose range the ledger keeps until the caller waits
  after the lock and releases it. Returning each operation's `Flush` with its frames instead would
  keep rollback but make every map path carry a pending free, and a kernel space behind a sleeping
  lock would let these paths wait but put a block under every kernel mapping.
- **D-497** Every frame is zeroed before it returns to the free pool, and `mm::init` zeroes all
  managed memory before seeding, so a free frame always holds zeros and no allocation path zeroes
  (amends D-44 and D-469). A frame then holds only zeros at each crossing, free included, as the
  Zero on crossing statement says. `USegment::zeroed` and `DmaSegment::zeroed`, the only ways to
  make memory ring 3 or a device reaches, still zero the run they take, since the kernel may have
  written it. Zeroing on allocation instead would leave kernel and table bytes in free frames, and
  a dirty bit per frame would add state to every record for a cost this pays once per free.
- **D-520** A kernel object lives in a slot of an object page, and a reference to it held by
  kernel code is a counted `Hold` rather than a lifetime: a slot with borrows cannot be removed or
  reached mutably, a page with borrows cannot close, and `pool::dissolve` refuses while any object
  page of the arena is open. Frame tokens kept in arena memory therefore return through their
  pages before an arena dissolves, so no safe caller can reclaim a frame a hold still reaches.
- **D-525** An arena counts its outstanding frame tokens, and `pool::dissolve` refuses while any
  token or earmark of it is outstanding (amends D-520, which counted only object pages). A token
  ends by release, which zeroes and credits its frame, or by `surrender`, which only lowers the
  count and leaves the frame charged for dissolve's budgeted scan. Teardown therefore visits each
  token once in constant time, and no safe caller can write a frame after it is reclaimed.
- **D-521** Each pool keeps its pages in its own radix directory of 64-way nodes held in object
  pages, growing in depth with its highest ordinal, rather than in one directory per world: a pool
  of one page needs no node frame, and a directory node holds typed page tokens.
- **D-522** A pool earmarks, for every object an operation may create, one page frame and one
  frame per directory level, takes the frames it needs before its leaf lock and releases the rest.
  The earmark is then exact under any interleaving, at the cost of refusing a creation that a free
  slot could hold when fewer than five frames of reservation remain.
- **D-523** A pool keeps one empty page and closes every other page that empties; the closed
  ordinal keeps the least generation above every one its page issued, and an ordinal whose page
  retired a slot is never reused, so one churning object cannot spend a pool's ordinals.
- **D-524** The world registry holds, per arena node, the record pool of that node's world, so the
  record lives in the world's own arena and is reached by its `Local`; a `NodeRef`'s generation is
  the arena node's generation.
- **D-47** The kernel heap takes frames from the frame allocator and addresses them through the
  direct map: a slab allocator in `libs/slab-layout` with eight classes from 16 to 2048 bytes, whole
  frames above that, and slab frames never returned.
- **D-64** There is no general safe physical read: `EarlyFirmwareMemory` and `FirmwareMemory` copy
  only from ACPI reclaimable and NVS regions, the ACPI reader bounds each table's length, and the
  ACPI parsers validate length, checksum and the whole entry stream first.
- **D-83** Page-table bits 62:59 are reserved and unused, and user pointers are range-checked
  against the user limit, never by shifting, so protection keys, LAM and five-level paging stay
  open.
- **D-87** No mapping call takes a bare physical address: `map_owned` consumes allocator frames,
  `map_boot` takes a `BootRegion` that the boot map covers, and `map_device` takes an admitted
  device range.
- **D-88** `unmap` returns a token that owns the frames until the TLB is flushed, so frames cannot
  return while a stale translation may exist; ownership is PTE software bit 9, ranges are
  preflighted and large pages are never split.
- **D-104** A user address space shares the kernel half by copying the top-level entries. Activation
  reserves the top-level table covering the whole device window and afterwards no new higher-half
  top-level table is made, so every user root shares one topology; root comparison ignores only
  `ACCESSED` (D-281).
- **D-185** `map_shared` keeps the `Shared` allocation alive in the `AddressSpace`, and `map_range`
  preflights the whole range and refuses it before any write unless every target is absent
  (D-458). The user and shared doors stay below the user ceiling in every space.
- **D-189** The running kernel address space is private to `kernel::mem`, which exports operations
  and never the `AddressSpace`, so a safe caller cannot reclaim tables the CPU is walking.
- **D-214** A stack address held by hardware comes from a pinned stack that only the active space
  maps and `unmap` refuses to overlap. A stack top is never stored as an integer: each TSS or switch
  write borrows a `KernelStackTop` from its retained identity and re-walks every leaf in the loaded
  `CR3` just before use (D-253, D-259).
- **D-243** Releasing an address space takes the linear `Bound` token its binding returned and
  yields `Released`, and an address space mints one `Binding` for one process lifetime (D-280), so
  no caller holds a release token beside something that can still install the root. `map_boot`
  refuses `USER`, because a boot region proves only that firmware described the bytes.
- **D-245** Cleanup refusals are typed and keep the authority to retry: ledgers never drop an entry,
  a zombie keeps its token and remaining mappings, `AddressSpace::Drop` never reclaims, a refused
  device retirement stays a linear owner in a bounded quarantine (D-285), and a `...Failure` wrapper
  that carries authority is `#[must_use]` (D-323). Kernel stacks are 32 KiB; the storage candidate
  runs as sibling stages so one stage frame sits on the boot stack at a time, and a deep path is
  restructured rather than given a bigger stack (D-359).
- **D-261** Every page-table entry access goes through one private `EntrySlot`; clearing a possibly
  cached entry walks the current `CR3` and executes `invlpg` only if the walk reaches that same
  slot, and `GLOBAL` is never admitted.
- **D-283** Every RAM mapping keeps the direct map's write-back type: the shared, owned and user
  doors refuse cache flags and `map_boot` uses the type the direct map gives its range (D-349),
  because conflicting memory types across aliases are undefined.
- **D-348** User mappings end at `USER_MAPPING_END`, one page below `USER_END`, in
  `AddressSpace::may_reach` and the loader, because an instruction on the last canonical page would
  push a non-canonical return address.
- **D-512** `memory_object::place` takes the caller's random words, one per counter value from the
  counter it is given, rather than calling `entropy::derive`, because a proven crate calls only
  verified code and `entropy` is not proven. The caller derives `PLACEMENT_SAMPLES` words before
  the call, so a placement costs that many derivations, not one per sample tried. Proving `entropy`,
  or one call per sample with the counter and loop in the kernel, would keep the lazy draw at the
  cost of a ChaCha and BLAKE2s proof or of unverified placement logic.

## Scheduling, locks and time

- **D-15** Scheduling is EEVDF-style virtual deadlines with tickless idle: the policy is pure
  arithmetic in `libs/eevdf` and `kernel/src/thread.rs` is the mechanism (D-53). The idle thread
  sits outside the policy and runs only when nothing else can (D-60), and if the policy ever returns
  nobody while every thread is a candidate the kernel takes the first candidate.
- **D-34** Taking a `SpinLock` clears the interrupt flag and the guard restores the flag it found,
  because a lock shared with a handler otherwise deadlocks on one CPU. Setting the flag or switching
  threads while a spin lock is held panics (D-66); a forgotten guard is an accepted liveness bug
  bounded to that lock, and only the panic path force-releases a lock (D-194).
- **D-36** An invariant TSC with a complete CPUID leaf 0x15 rate is the TSC rate, and only the APIC
  timer is measured against it. Otherwise both rates are measured against the 8254 PIT over 20 ms
  windows on counter 0 and accepted when two windows agree to one part in 64; leaf 0x16 is never a
  rate. The PIT stays claimed as a watchdog only when it was the calibration source, and two
  conclusive disagreements revoke the public clock while scheduling continues on a private epoch
  (D-225, D-459).
- **D-572** The processor that calibrated the clock owns it and its PIT watchdog. The conversion is
  published with Release after the origin and read with Acquire, and revocation is one
  compare-and-swap of a word that carries the last trusted reading, so no reader on any processor
  sees a rate without its origin or a half-made revocation. A processor whose `IA32_TSC_ADJUST`
  differs from the owner's, or whose counter steps backwards against the owner's under a shared
  lock at bring-up, is not brought online (amends D-36).
- **D-51** The context switch is a private naked function in `wost` that saves callee-saved
  registers and swaps stacks; the kernel names threads by slot and never holds a context value, so
  every stack pointer the switch loads came from `wost`.
- **D-52** Kernel thread stacks come from a fixed arena of guarded slots laid out by
  `libs/stack-layout`, and `task::init` walks every stack in the live tables before accepting the
  arena.
- **D-54** Preemption happens at trap exit, after end-of-interrupt, and only when the interrupted
  frame had interrupts enabled; since every spin lock clears that flag, critical sections need no
  preempt count.
- **D-57** The TSC is the clock and the APIC timer is a one-shot alarm armed only for the next slice
  end or sleeper deadline, so an idle kernel takes no timer interrupts; every wake arms it, trap
  context included (D-133). Time is nanoseconds through a multiply and shift in `libs/timebase`.
- **D-61** A `WaitQueue` is a bitmask of thread slots, so a wake from a handler allocates nothing; a
  waiter registers and rechecks under a clear interrupt flag to close the lost-wakeup race.
- **D-62** `ThreadContext` is a zero-sized, non-`Send`, non-`Sync` token that only thread context
  can mint, and every call that may block takes it, so blocking from a trap handler does not
  compile.
- **D-474** A ring 3 entry runs as its thread: after any trap phase, `dispatch` mints a
  `ThreadContext` and calls `on_user_entry` with `UserEntry::{Call, Fault, Interrupted}` and the
  frame as `UserRegisters`, with interrupts on (amends D-466). Calls that allocate, block or spawn
  need thread context, and one exit path for every ring 3 entry replaces the trap-context retire.
  Ring 3 is preempted in that handoff, so `switch_from_trap` serves only ring 0 frames, and the
  frame is validated with interrupts off until `iretq`. Wait queues are slot lists linked under the
  scheduler lock instead of an intrusive `wost` list, which keeps them free of `unsafe`.
- **D-466** `wost` mints `ThreadContext` only at boot entry and at thread start, and there is no
  runtime claim (amends D-62). A claim that checked the trap depth let any helper mint a token, so a
  blocking call's safety rested on where it was called from rather than on what it was given.
  Thread entries receive the token and one argument.
- **D-63** `ThreadLock` is the second lock kind, for long work such as console output: it leaves
  interrupts on and refuses trap context and any held `SpinLock`, because the preempted holder could
  then never run.
- **D-70** These rules hold now so that multiprocessor support is a bring-up: no guard lives across
  a context switch, `Relaxed` needs a written justification, handlers never block, and the
  lost-wakeup recheck runs under the queue lock that every waker takes. The handle table promises
  no lowest free handle, the ring no FIFO across submitters, and unmap, close and release no global
  visibility on return.
- **D-85** `SpinLock::lock` borrows the `ThreadContext` mutably for the guard's life, so no blocking
  call or second token lock compiles while it is held; `lock_leaf` is for take, mutate and return
  with no call-out and may nest under a token lock. A leaf-guarded static exposes no closure form
  (D-121, D-430), and the heap never holds its lock while taking a frame.
- **D-560** Every lock names a `libs/lock-order` level as a type parameter (amends D-85 and D-63).
  A token lock compiles only when its level is declared after the token's, and a held-rank word in
  the per-CPU block refuses at run time a spin lock at or below a held rank and any lock under a
  leaf guard, which also holds the top rank. Leaves stay run-time checked rather than token checked
  because trap paths take them with no token in reach. Thread locks keep the same word per thread
  slot, because a preempted holder keeps its lock, and rank below every spin lock.
- **D-99** Everything that follows a thread (`RSP0`, the address-space root, extended state,
  `FS_BASE`) is switched in `task::prepare`, the one function every switch passes through. The root
  is installed from a `Binding` that `AddressSpace::bind` mints only after comparing the higher half
  with the live kernel's (D-106), and `set_thread_base` refuses a value outside user space (D-118).
- **D-152** Monotonic time is a read-only page mapped into every program, with a write-once
  constants region and a seqlocked clock region whose `mode` word permits its use, so a program
  converts `rdtsc` without a trap (D-166). It is global until Phase 7 replaces ambient time with
  per-world clock modes.
- **D-472** The time page no longer publishes a counter conversion (amends D-152): its clock region
  stays zero, so its mode reads none and a program asks with `Uptime`. With `CR4.TSD` set
  (D-471) ring 3 has no counter to convert, and the page publishes a value again only once each
  world has its own clock page.
- **D-500** The time page is version 2 and carries a value, not a conversion (amends D-472): world
  time rounded down to the world's precision, one aligned `u64` that a compare-and-swap maximum
  only raises, so it needs no sequence lock and never goes back. The precision defaults to 1 ms,
  coarse enough to blunt timing measurement and fine enough for timeouts; finer time is the raw
  right's. `Uptime` and `Timer` serve the same rounded values, so neither bypasses the precision.
- **D-501** World time is host time minus the host time at which the world's clock started, so a
  world starts at 0 and never learns host uptime. The clock is a value spawn passes in, one root
  clock until worlds are objects, so a per-world clock replaces the value rather than a global.
- **D-502** While a thread with a world clock runs, the one-shot alarm is armed at least every
  max(precision, 1 ms), and the page is refreshed before every return to ring 3 (amends D-57).
  A program that never enters the kernel still sees time move, and a processor that runs no such
  thread arms nothing new, so an idle kernel still takes no timer interrupts.
- **D-503** Ring opcode 8 `Random` fills a range of the data page from the kernel generator and
  refuses `UNAVAILABLE` before it is seeded. It draws from the one kernel `Local` until a world
  key gives each world its own stream; a program never adds entropy, and no draw blocks.
- **D-590** The generator's periodic reseed runs on the keyboard poller, the one kernel thread that
  already wakes every 10 ms, rather than on a thread of its own or in the timer interrupt, which
  may not read seed words for long. The call costs a clock read and a leaf lock until the
  interval has passed.
- **D-591** `libs/entropy` takes the wipe as a function pointer that each key-holding value stores
  and calls when it drops, rather than a trait parameter on every type, because the kernel has
  one wipe and the host tests another. BLAKE2s keeps its chaining value as bytes so the same wipe
  clears it.
- **D-592** A seed word is credited only if it differs from every word credited since the last
  reseed, at most four, and the reseed wipes that record. Remembering the last four across reseeds
  would keep seed material that, with the guessable counter sample, rebuilds an earlier key.
- **D-240** An NVMe or VT-d wait is bounded by the clock, through `timebase::PollLimit` for NVMe,
  with a spin count only where no clock exists yet or the clock has stopped, because an iteration
  bound depends on processor speed.
- **D-293** Every `wost` module that owns a static lock exposes `force_unlock(&PanicToken)` covering
  each one, and `panic::reclaim` calls every module's door, so the panic reporter never deadlocks on
  a lock whose holder will not resume.

## Native ABI, handles and rings

- **D-06** Every kernel object is reached through a process-local handle with rights bits, and the
  data path is shared-memory submission and completion rings beside a small synchronous call set,
  for batching and structural asynchrony. Ring structures carry versions, there are no privileged
  opcodes, and everything read from shared memory is validated.
- **D-78** Handles live in one global arena; a handle value is the slot's generation and index,
  tagged and mixed with a per-table value and decoded by `libs/handle-space`, and generations refuse
  stale reuse. A table's identity is a non-wrapping `u64` (D-204), a foreign slot answers `Stale`
  like an empty one (D-222), and type codes are never renumbered.
- **D-79** There is one ring mechanism, on kernel-owned pages: every operation posts exactly one
  terminal completion, completion space is reserved at submission so overflow cannot happen, each
  entry is copied once and then validated, and cancellation is advisory.
- **D-80** A signal or other unsolicited notice is a completion-queue entry, never a kernel-built
  frame on the user stack, so there is one event plane; its record waits for ring version 2.
- **D-110** A channel carries bytes and handles over bounded inboxes: a full inbox returns `Full`,
  an orphaned end returns `PeerClosed`, and a dropped end wakes its peer's waiters (D-127). A handle
  moves as object and rights; the sender needs `TRANSFER` and may only narrow, and asking for more
  than was offered is refused, never clamped.
- **D-111** A new program's handle table holds exactly one entry at birth, its bootstrap identity,
  and every later handle arrives by channel receipt or a handle-authorized ring operation, never by
  ambient minting.
- **D-113** Ring pages are allocated and kept by the kernel and mapped into the program without
  ownership, and `serve` reads them once into a snapshot and acts only on copies.
- **D-116** The `0x80` gate dispatches on `rax`, an unknown call answers `-ENOSYS`, and only `exit`
  does not return; completion room is reserved at the doorbell before a submission is accepted.
  `RingEnter` returns every completion posted during the call (D-461).
- **D-150** Every operation that can wait has a ring form and none is reachable only through a
  blocking call, because a runtime that multiplexes work onto fewer threads cannot live with a
  blocking-only door; the synchronous call path in `docs/roadmap.md` sits beside the ring under this
  rule.
- **D-151** A submission that can produce a handle reserves a handle slot at admission with its
  completion slot, so refusal happens at submit. `HandleTable` works through `&self` with held and
  promised counts in one atomic word and a `SlotReservation` whose `Drop` returns the slot (D-163),
  and a table commits its quota at construction, outside trap context (D-182).
- **D-164** An operation that cannot finish at once keeps its admission-time completion slot,
  `RingWait` returns at once when nothing is in flight, and Cancel posts its own completion while
  the target still gets exactly one `STATUS_CANCELLED`.
- **D-173** Read and OpenAt are ring operations on a 32-byte submission with data through a third
  shared page, so no user pointer is validated; OpenAt derives rights, a file getting `READ` from
  `TRAVERSE` plus the parent's universal rights. OpenAt refuses a path that names its starting
  directory, which would copy a handle without `DUPLICATE`, and a device or volume failure
  completes with the `IO` status (D-461).
- **D-174** Every submission field is a value ring 3 chose, so the kernel narrows it with `try_from`
  and refuses or saturates; `as` at that boundary only widens.
- **D-181** The native ABI numbers only the calls, opcodes and rights the kernel decodes; descriptor
  numbers, cursors and working directories belong to a user-space adapter.
- **D-192** A pending ring operation stays in its slot, marked settling, until its terminal
  completion is posted into the room reserved at admission; ring pages are reclaimed only once the
  queue is quiet.
- **D-221** Each ring direction is single-producer, single-consumer, and the 32-bit index is the
  only synchronization edge: write the entry, then release-store the index; acquire-load the index,
  then copy the entry once. A `Shared` allocation becomes either `SharedBytes` or one `SharedRing`
  (D-269), and publication words are accessed by one aligned dword `mov` in assembly, never a Rust
  atomic, because ring 3's plain stores make a Rust atomic over them unsound (D-282).
- **D-526** Handles are 64 bits with an 18-bit slot and a 44-bit generation, and a slot whose
  generation would reach the limit retires (amends D-78). Each process's table is an object in its
  world's arena, and its mixer is derived once from the world key and the process ordinal and
  stored, so a value is a function of the key, the process, the slot and the generation and
  reveals no other table's allocation order. A retired slot stays charged to the quota.
- **D-527** Ring version 2 is one page of 64-byte submissions and 32-byte completions, and buffer 0
  is a separate data page beside it: arena frames are not contiguous, a submission names a byte
  range by buffer and offset, and a completion carries a second value. Each opcode reads only its
  own fields, and a nonzero field it does not use or any flag is refused with `DENIED`. `Random`
  takes its range from the buffer fields like `OpenAt` and `Read`.
- **D-528** A handle entry names its object by a copyable `ObjectRef`, and only `acquire` and
  `release` change what keeps an object alive: a handle holds one strong reference, `Duplicate`
  takes another before it installs, and `Close` or the table's teardown releases it. The table's
  entries sit behind one object lock, so a strong-count change from the object's pool leaf can run
  under it.
- **D-529** File work names its queue by world and `Local`, and a queue cannot be destroyed while
  an operation holds its pending entry, so the pending entry is the running item's durable count.
  A process's table and ring are destroyed by the collector after the queue is quiet; a hold that
  still borrows either refuses that step without changing anything, and the next collection
  retries it.
- **D-530** An `ArenaFrame` can back a shared mapping, so the ring page and buffer 0 are frames of
  the process's world. The mapping keeps the frame alive through the same keepalive a shared run
  uses until mapping records replace it.

## Processes

- **D-07** Native process creation is spawn-only with explicit grants and no fork, because fork is
  an implicit inheritance path beside the grants the kernel audits.
- **D-100** Until the user-space loader of `docs/roadmap.md` replaces it, the initial program is a
  Limine module and the loader refuses any segment that is writable and executable, runs past the
  file, overlaps another or reaches the kernel half; pages are filled before they are mapped
  read-only or executable.
- **D-117** A retired program becomes a zombie that the one service worker collects, never the dying
  thread (D-203). It keeps its ring and everything its in-flight operations can touch until
  `Queue::abandon` has ended each with a `CANCELLED` completion (D-153, D-167); reclaim then
  releases the handle table, the ring and the address space.
- **D-531** A process is an object in its world's arena with a state machine from `libs/world-tree`,
  and its teardown is a fixed sequence of steps the service worker's reaper runs in FIFO order; a
  step that must wait sends the process to the tail (amends D-117). Its status is written once to
  an observation in the spawner's world, which the process names only weakly.
- **D-532** An exiting thread never frees its own slot: a process's thread leaves it `Retired`
  until its teardown has seen it off the processor, and a kernel thread leaves it `Exited`, reusable
  only by another running thread. The reaper's queue is bounded by the thread slots until thread
  records live in arenas.
- **D-533** A memory object is a page array of arena data frames in its world, and its state word
  moves by compare and swap from `libs/memory-object`. A seal changes the state first, then seals
  every frame under the object's lock and waits for the flush, then renews the handle's entry in
  place under the next generation with the sealed rights, so the old value stops resolving and the
  slot is unchanged. The handle's generation is checked renewable before anything changes; a close
  racing the seal is ordered after it and leaves a sealed object with no handle. The frames sit
  behind an object-rank lock rather than a leaf, because sealing a frame takes the frames leaf.
  `MemoryCopy` moves bytes through a kernel stack buffer, so no two object locks are held together.
- **D-534** The kernel places each process's stack, ring, buffer 0 and time page from a seed
  derived once per address space from the world key with its own ordinal, sampling eight words
  per placement from the seed and a stored counter, and the entry block (version 3) names the
  three shared pages. Placements stay above 4 GiB, where position-dependent images link, and the
  stack keeps an unmapped guard page. A seed per process ordinal shared with the handle mixer was
  the alternative; a space ordinal keeps the seed with the space that C9's `SpaceCreate` makes
  without a process.
- **D-535** A user leaf names an arena data frame by reference: `AddressSpace::map_frame` counts
  the leaf in the frame's own token, readable and writable apart, and `clear_frame` clears only a
  leaf that names that frame and returns its `Flush`. A token with a writable leaf cannot seal, one
  with any leaf cannot be released or surrendered, and a token that was ever mapped waits for every
  other processor to pass the newest shootdown before it seals or frees, so no safe caller can free
  or seal a frame ring 3 still reaches. The counts fit the token's padding because tokens fill
  object pages; a per-frame pending flush did not fit. Read-only leaves stay counted over a seal.
- **D-536** A space's mapping records are the placement records: a kernel placement records its
  stack, ring, buffer 0 and time page (a guard page is a record of its own), `Map` records a range
  of an object and `Unmap` removes a whole record. An object record holds a strong reference, and a
  writer while it maps `WRITE`, released only after the flush that cleared its leaves has
  completed. The records live in one arena page of the space's world, so the space is small to
  move. A `Map` or `Unmap` takes the space out of its process for the operation, so no lock is held
  while it maps or waits; the caller is the process's one thread. Page tables stay kernel frames,
  and the ring, buffer 0 and time page stay shared mappings kept by the address space until
  reclaim.
- **D-537** The in-kernel loader validates the image first, so the spawn's one earmark covers each
  segment's object, then writes each segment into a sealable object, seals it read-only or
  executable against the world's code quota (65536 pages for the root), and maps it by record. A
  refused spawn and a process teardown both clear by record and then reclaim, so there is no
  region ledger and no rollback. A memory opcode holds a strong reference for its length, so a
  close racing it leaves the last release to the opcode.
- **D-481** The ring 3 witness is a cargo-built static `ET_EXEC` linked at `0x400000`, and the
  loader maps each segment at its `p_vaddr` with no per-program shift, because a position-dependent
  image holds absolute addresses that a shift breaks and the in-kernel loader takes no `ET_DYN`;
  each address space is already separate. Position independence and randomized placement return
  with the user-space loader (amends D-100).
- **D-191** `ProcessConstruction` owns the new address space and every mapped range until the thread
  is published, so any refusal in spawn rolls back through one owner, and `load_image` checks that
  the entry lies in an executable segment before taking a frame.

## Storage

- **D-08** WizFS is copy-on-write, checksummed and snapshotting, designed once behind the VFS with
  snapshot ids in the key, checksums beside pointers, one ordered journal and designed-in scrub and
  repair, its format in a host-tested `no_std` crate. It is built on the object store, with crash
  injection and a repair tool before the format freezes (D-73).
- **D-169** Storage is one NVMe controller with one I/O queue pair, polled and read-only, because
  NVMe is the only freely specified controller QEMU and VMware both present; queue size, stride and
  timeouts come from `CAP`, and every poll is bounded. A namespace must use 512-byte blocks with no
  metadata (D-196).
- **D-170** The first filesystem is read-only FAT32 in `libs/fat32`, generic over its `Sectors`
  trait, allocation-free and 512-byte only. A chain is bounded by the volume's cluster count and a
  directory walk by the clusters that can hold 65,536 entries with a cycle check, so ring 3's offset
  cannot choose the kernel's work (D-238, D-352).
- **D-171** The VFS abstracts directories and dispatches by enum, and `libs/path-walk` resolves `..`
  lexically and cannot ascend; absolute paths, escapes and over-long names are refused. tmpfs and
  devfs are the other backends, and a devfs `Device` is a `Node` variant that `vfs::read` cannot
  take (D-172).
- **D-188** An NVMe command with an uncertain outcome is resolved by clearing `CC.EN` and observing
  `CSTS.RDY = 0`, or else every queue and buffer frame is withheld for ever. DMA ownership is one
  linear `Idle`, `Reserved`, `InFlight` transaction, and ringing the doorbell consumes the borrowed
  `DmaPublication` (D-339).
- **D-199** Storage selection holds a function, its windows and its mapping as a lease, committed
  after a smoke read or retired by exact rollback; the next controller is tried only after complete
  retirement, and a stop that cannot be verified ends fallback.
- **D-200** `RingEnter` only validates and admits Read and OpenAt; one kernel worker fed by a fixed
  eight-slot queue does the VFS and device work, and a full queue answers `STATUS_NO_ROOM`. OpenAt
  reserves its handle and copies its path at admission and the worker installs through that
  reservation, because a FAT mount may take a `ThreadLock` that trap context refuses (D-381).

## Devices

- **D-39** A port claim reserves the whole width its accesses drive, claims are permanent and taken
  all at once through `PortSpace::reserve_all` (D-138), and `wost` exposes one typed claim per fixed
  device with the port numbers only there (D-351).
- **D-48** The console scrolls from a RAM shadow of the framebuffer once one is attached, so the
  device is not read, and maps the framebuffer write-combining through PAT entry 1 with exactly one
  alias (D-50).
- **D-49** A device window is mapped by `map_device`, uncached and never executable, into a
  `DeviceMapping` that is the only way to an `MmioRegion`. ACPI is hostile input parsed by
  `libs/acpi-tables`.
- **D-168** A driver claims a PCI function through CF8/CFC configuration access, and the claim mints
  one non-duplicable authority bound to registry, address and generation that every decode,
  bus-master, command and BAR transition borrows (D-265). BAR sizing is one restored transaction in
  `libs/pci-config` (D-187); memory decode follows a `MAXPHYADDR`-checked, page-rounded admitted
  range and a fresh BAR readback, and bus mastering consumes a fresh disabled-controller observation
  (D-278).
- **D-179** `libs/ps2-keys` decodes scancode sets 1 and 2, and the set in use is read off the
  arriving bytes by `SetProbe` (`F0` means set 2, a non-reply byte above `0x83` means set 1),
  because both the configuration byte and the identity reply lie on real machines; bytes are held
  until a decisive one arrives (D-239).

## DMA isolation and PCI

- **D-357** The storage controller runs behind a kernel-owned Intel VT-d domain that identity-maps
  only frames a live `DmaSegment` owns, and `admit_bus_master` requires that function's `DmaDomain`
  proof, which the admission spends (D-360), and first enables bus mastering on each bridge above
  the function (D-462). Without a usable unit storage does not start; the unit
  invalidates through its registers, flushes table lines when it lacks `ECAP.C` and draws its tables
  from one owned arena (D-358). No firmware RMRR is supported, so firmware bytes never become
  mapping authority (D-388).
- **D-367** VT-d mapping takes a `DmaSegment`, and once a translation may exist any failed map,
  unmap or invalidation retains the segment. Revocation completes a drained global invalidation
  before frames return (D-383), live entries publish by one qword store and NVMe mappings carry
  direction (D-387), and published tables and mapped segments keep their frames when dropped
  (D-433).
- **D-369** A rejected storage candidate clears and reads back Bus Master Enable before trusting the
  controller's stop, invalidates every queue segment before a frame drops, and never restores
  inherited BME: a transition that clears it never writes the original word back (D-462). A
  unit-wide write first fences every requester its DRHD declares, from a bounded
  inventory, and drains reads and writes (D-370).
- **D-371** DMAR authority comes from one sealed `PciTopology` probed over every bus, device and
  function before enumeration: bridges form a single-parent tree with nested intervals, a hierarchy
  scope covers only proved descendants, and a malformed scope invalidates the whole DMAR.
- **D-372** Interrupt remapping is mandatory: one DRHD must cover every present function, an empty
  EIME table denies every device interrupt, and a non-duplicable isolation proof gates bus-master
  admission and the first `sti`, because a hostile MSI could otherwise inject a privileged vector.
- **D-389** The sole segment-zero unit is taken over once per boot, before the first topology read,
  and only when whole-segment coverage is proved statically and no RMRR applies. It starts with an
  empty root and a deny-all interrupt table, translation is never disabled afterwards, and firmware
  TE, QIE, IRE or CFI left on refuse the takeover.
- **D-390** DMA safety is a boot-handoff contract: a checksummed `WZDH` ACPI table, accepted only on
  the CPU-identified QEMU TCG or VMware platform, attests CF8/CFC reach, static requesters and
  quiescence, and bare metal refuses. The kernel's first act clears and reads back Bus Master Enable
  on every function before memory initialization; VMware's 440BX host bridge is kept as a
  non-requester and its PIIX4 ISA bridge is fenced by masking both 8237 controllers (D-392, D-426).

## Build, gate and release

- **D-16** The build is self-contained: `xtask` writes the MBR, the FAT32 ESP and the VMware
  descriptors in Rust with no external imaging tool, uses the GNU host toolchain on Windows and runs
  whole on Linux (D-450), and a new system-level install needs the owner's approval.
- **D-17** Rust is pinned to an exact version in `rust-toolchain.toml`, because a floating channel
  lets a new lint turn the gate red without a commit, and nothing host-specific goes there (D-24).
  `limine` stays at 0.5.0 because 0.6 needs nightly, `fatfs` builds without `chrono`, whose Windows
  bindings need `dlltool.exe`, and the kernel links with `code-model=kernel` and static relocation.
- **D-18** QEMU (EDK2, with the machine type named exactly as `pc-q35-9.2`) is the development loop
  and a VMware Workstation boot is part of every phase's definition of done, so two firmware and
  device paths are exercised (D-218). A phase that adds a device adds it to the QEMU machine and the
  VMware template together (D-176).
- **D-25** Every vendored binary has a SHA-256 in a `CHECKSUMS` file that `cargo xtask
  verify-vendor` and the gate check, and an unlisted file is refused. Image assembly reads
  `BOOTX64.EFI` once, checks that buffer against its entry and writes that same buffer (D-379,
  D-380).
- **D-26** `cargo xtask ci` on the developer's machine is the gate. The hosted workflow,
  `.github/workflows/source-preview.yml`, runs a subset of its steps and boots nothing.
- **D-40** `xtask/src/elf.rs` reads the linked kernel after every build and requires every loadable
  segment in the higher half, page aligned and disjoint, the entry inside an executable segment, no
  writable executable segment, a `.bss` that costs no file bytes, and `.text` and `.requests`
  loaded, because a linker script that is incomplete still links.
- **D-482** `xtask` builds the witness in its own target directory, always in release, with
  `-C code-model=small` appended after the kernel target's `code-model=kernel`, where the last
  value wins, and without LTO, because fat LTO refuses to merge small-model code with the
  sysroot's kernel-model `core`. `xtask/src/image/witness.rs` checks the image against what the
  loader maps before packing it as the one Limine module.
- **D-220** Every locked crate has a reviewed license row in `supply-chain/inventory.txt`, and
  sources are crates.io only.
- **D-346** Controlled boots drive QEMU through QMP with bounded exchanges.
- **D-386** Every kernel build passes one exact rustflags list with `--remap-path-prefix` for the
  source root, the sysroot and the Cargo home, so shipped debug information names no host path;
  `cargo xtask debug` maps the source root back for GDB.
- **D-464** `xtask` trusts the checkout and the account that runs it: it reads and writes with the
  standard library and keeps no evidence bundle, committed SBOM or pinned debugger, because those
  defences guarded a developer's files against that developer. The gate's result on a commit is the
  record of a build.

## Tests

- **D-28** Checks that need a machine run in the kernel under the `selftest` feature, report through
  QEMU's `isa-debug-exit` port and are driven by `cargo xtask test-boot`, because a `no_std`,
  `no_main` binary cannot host `cargo test`; the feature never ships in an image.
- **D-65** The gate boots the self-test kernel in release and in debug, the debug boot being where
  an arithmetic overflow fails, and boots the shipped image, which must settle with no panic and log
  `user: ring 3 call observed`, the record the idle thread emits once after a call from CPL 3
  (D-427).
- **D-84** The ring protocol has TLA+ models under `models/`, including index publication with
  terminal ownership (D-229), run by `cargo xtask model-check` with a vendored TLC when a model
  changes, and `libs/ring-proto` uses proptest for its index arithmetic (D-112).
- **D-260** A kernel panic is written as one opening record with backslash, CR and LF escaped,
  followed by a completion record, the UART reports any byte it cannot send, and the host tool fails
  a boot at the opening record.
- **D-454** The suite's tiers are a boot smoke that must settle with its required records and no
  panic, syscall and ABI behaviour, isolation and security, a few edge tests per hostile-input
  parser, fixed-bug regressions, and a small tooling smoke set. Source rules are lints, attributes
  or types, never tests that read source text. A release is `cargo xtask ci` green, a VMware boot
  of both images read by `cargo xtask qualify-vmware`, and a person reading the release diff;
  `cargo mutants` and fuzzing are on-demand laptop work that never blocks a commit.
- **D-473** Proofs are Verus over the shipped `libs/` source, so the checked code is the built code.
  Exec functions carry `#[verus_spec]` attributes, a function with a loop sits whole in `verus! {}`
  because stable Rust refuses attributes on loops, and specifications and lemmas sit in one
  `verus! {}` module per crate. `vstd` is an exact-version, always-on dependency of proven crates
  only, since a feature gate would let the checked and the built code drift unseen; a plain build
  erases every ghost item. `cargo xtask verify` runs `cargo verus verify` outside the gate with
  `--no-cheating`, which refuses `assume`, `admit` and `external_body`, and
  `--no-external-by-default`, so no function is skipped for lacking an attribute; a crate's only
  exclusion is its `fmt` module. The Verus build is the one `tools/verus/VERSION` pins, made by
  `tools/verus/build.sh` from source at the pinned commits, because the published binaries need a
  newer glibc than the test host has. That host's g++ 12 has no `<format>` header, so Z3 is
  compiled with `tools/verus/format`, a stand-in that renders the `{}` fields of Z3's error
  messages; the Z3 tree itself is the pinned commit, unmodified.
- **D-510** A host test that a Verus proof states for every input is deleted, and a test stays
  only for what no proof states: `tlb-generation`'s enumerated catch-up test and `frame-space`'s
  free-count test went, while the search that finds a run straddling the hint keeps its test,
  because the proof shows a found run is free but not that every free run is found.
- **D-511** A proven type whose invariant ties several fields together changes them through one
  `no_unwind` function that borrows each field, with a lemma over the new values proven before the
  call, because Verus checks a type invariant after every field write and allows no temporary
  break. The same holds for a `FrameSpace` bitmap and its counters and a `Reservation`'s reserved
  and used counts.

## Conventions

- **D-19** The design lives in `docs/architecture.md`, written before or with the code. The wizard
  theme stays in product-facing strings, never in identifiers, and code comments never cite a
  decision.
- **D-463** Code carries no doc comments. `// SAFETY:`, `# Safety` and one line for a constraint
  the code cannot show are the only comments, because prose beside code restated it and drifted;
  the design lives in `docs/architecture.md`.
- **D-22** One `[workspace.lints]` table governs every crate, `xtask` included: `unsafe_code` is
  `deny`, lifted once at the root of `wost` and `forbid` in `libs/` and `xtask` (D-67). A
  suppression is `#[expect(lint, reason = "...")]`, with `#[allow]` only where `expect` cannot
  reach, and a lint is enabled in the commit that brings its count to zero (D-451).
- **D-27** The Rust API Guidelines are the house style, with the Rust for Linux coding guidelines
  taking precedence where they are more specific about kernel work.
