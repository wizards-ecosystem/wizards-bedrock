# Architecture

This document describes the kernel as the code builds it today: one x86_64 processor booted by
Limine under UEFI on QEMU or VMware Workstation. `docs/roadmap.md` holds the plan for what changes
next, and `docs/decisions.md` the reasons behind individual choices.

## 1. Crates and the unsafe rule

The repository is one Cargo workspace with five kinds of crate.

| Path | Builds for | `unsafe` | Holds |
|---|---|---|---|
| `wost/` | `x86_64-unknown-none` | allowed | boot glue, ports, MMIO, paging, VT-d, interrupt entry, context switch, locks |
| `kernel/` | `x86_64-unknown-none` | `#![deny(unsafe_code)]` | the kernel binary: policy, drivers, scheduler, ABI, filesystems |
| `libs/*` | host and kernel | `#![forbid(unsafe_code)]` | hardware-free `no_std` encodings and arithmetic, tested on the host |
| `xtask/` | host | `#![forbid(unsafe_code)]` | build, image, QEMU and VMware runners, the gate |
| `witness/` | `x86_64-unknown-none`, ring 3 | allowed in `witness/rt` only | the ring 3 witness and its runtime |

`wost` is the only kernel crate that writes `unsafe`. Its exports are safe functions and types, and each
one must leave no way for any safe caller, including a wrong one, to reach undefined behaviour.
Where a safety argument needs arithmetic, the arithmetic lives in a `libs/` crate and `wost` keeps
only the lock and the instruction: `libs/port-space` decides which ports a claim covers, and
`wost/src/arch/x86_64/port.rs` holds the registry and executes `in` and `out`.

The kernel uses `deny` rather than `forbid` because `wost::entry!` expands
`#[allow(unsafe_code)]` boot glue into the binary crate. That glue, `kernel/linker.ld` and
`kernel/build.rs` stay in the binary crate because a linker may drop `#[used]` statics in an rlib.
Architecture-specific code lives under `arch/x86_64/` and is reached through `arch::current`.
`wost/src/boot/limine_glue.rs` and the request statics that `wost::entry!` expands are the only
code that names the boot protocol; the kernel sees a `wost::boot::BootInfo`. `README.md` lists
the library crates.

## 2. Boot and handoff

The kernel is an ELF64 executable linked at `0xffffffff80000000` with `code-model=kernel` and
`relocation-model=static` (`.cargo/config.toml`). It is not relocatable, so Limine loads it at its
link address and there is no KASLR. `kernel/linker.ld` declares six `PT_LOAD` segments, one per
protection class: text, read-only data and writable data, each for the entry enclave and for the
rest of the kernel. The image is therefore W^X from its first instruction. Limine's requests sit in
a kept `.requests` section, and `.bss` is `NOLOAD` at the end of the writable segment.
The linker refuses orphan sections, and `xtask/src/elf.rs` reads every linked kernel back: it
requires each loadable segment in the higher half, page aligned and disjoint, the entry inside
an executable one, no writable and executable segment, `.bss` costing no file bytes, and `.text`
and `.requests` loaded.

`cargo xtask image` writes `target/bedrock.img`, a 64 MiB raw disk with an MBR and one FAT32 EFI
system partition from LBA 2048. It holds `/EFI/BOOT/BOOTX64.EFI` (the vendored Limine in
`boot/limine/`), `/boot/limine/limine.conf`, `/boot/wizard-kernel.elf` and `/boot/witness.elf`,
which Limine hands the kernel as its one module.

The kernel requests Limine base revision 4 and refuses to continue without it. `BootInfo`
carries the boot memory, the framebuffer, the image's physical address, the RSDP and the module,
each as a `Result`, so a missing answer is a typed refusal. The direct map offset must be a
page-aligned base in the canonical higher half.
The boot memory is one `wost::boot::BootMemory` holding the memory map and the direct map offset.
`wost::boot` builds it once from the bootloader's answer, and it is neither `Copy` nor `Clone`.
Only it opens `EarlyFirmwareMemory`, and `mm::init` consumes it, so a `MemoryMap` or `DirectMap`
built anywhere else starts no allocator and reads no firmware memory.
`libs/memory-map` accepts 1 to 128 regions and refuses a larger map whole. An unknown region kind
is treated like a reserved one: never allocated and never direct-mapped.

`kernel_main` and `boot_continue` in `kernel/src/main.rs` run this order:

1. `iommu::secure_handoff` clears Bus Master Enable on every PCI function and checks the platform
   DMA contract (section 8). The serial log and the framebuffer console come up, and only then
   does a refused handoff panic, so the reason is visible.
2. `segment::init` and `trap::init` install the boot processor's static GDT and TSS and the IDT; `cpu::establish_control_state`
   sets the control registers and flags the kernel owns; `cpu::enable_protections` turns on the
   supervisor protections; `xstate::init` enables extended state.
3. `mem::init` starts the frame allocator. `mem::init_paging` reads the ACPI interrupt tables,
   builds the kernel's tables, maps the thread stack arena, rewrites PAT entry 1, activates the
   tables, moves the boot processor onto its entry area (section 3) and starts the thread table.
4. `iommu::discover` validates the DMAR, and the boot thread moves to a guarded stack through
   `task::continue_on_stack`.
5. `boot_continue` attaches the console's RAM shadow, masks every I/O APIC
   (`kernel/src/routing.rs`), surveys PCI and selects storage (`storage::survey_the_bus`) and lists
   tmpfs and devfs (`storage::survey_the_filesystems`).
6. `interrupts::start` silences the 8259 PICs, applies the speculation controls and chooses user
   isolation, starts the local APIC, calibrates the clock, publishes the time page, checks that
   every interrupt source is masked or remapped, and only then sets the interrupt flag.
7. `random::seed` seeds the kernel's random generator (section 5). The keyboard comes up polled,
   the I/O APIC mappings are released with every line still masked, its poller and the service
   worker start, the idle thread starts, and `user::run` spawns `witness.elf`. The boot thread
   then blocks reading keys and draws each one.

Storage comes up before the clock is calibrated, so its bring-up waits use bounded spin counts.
A failed stage logs a typed reason and the boot continues with less: no paging keeps the
bootloader's tables, no guarded stack, clock or seeded generator keeps ring 3 closed, and no local APIC keeps
interrupts off.

## 3. CPU, traps and the syscall gate

Each processor has an entry area in the entry window, at `0xfffffe8000000000` plus 128 KiB per
processor (`wost/src/arch/x86_64/percpu.rs`). Page 0 is the entry page: its own address, the
processor's block, its table page and its paired-root route (state, full root, stripped root, task
stack top and transition stack top), all the entry assembly reads before it switches roots. Page 1
holds the GDT and TSS. Above them, each over an unmapped guard page, sit the 16 KiB transition stack
and the double-fault, NMI and machine-check stacks, which the TSS names as IST1 to IST3. The block
holds the trap depth, the live frame, the current task, the owner of the live interrupt exclusion,
the held-rank word and the last user root.

The boot processor runs on a static entry page, static tables and one shared fault stack from the
first instruction of `boot::enter` until paging is active. `percpu::bring_up_boot_processor` then
maps its area from fresh frames and loads it: tables, `ltr` and `IA32_GS_BASE`. A second processor
takes the same two steps with its own index, the mapping on the boot processor and the load on
itself. Areas are never unmapped, only bring-up writes the window, and ring 3 never runs on a
processor without one. A stripped root copies the areas that exist when it is built, so building
the first one closes the set.

The GDT has seven entries: null, kernel code, kernel data, the two-entry TSS descriptor, user data
and user code (`wost/src/arch/x86_64/segment.rs`). Every code and data descriptor is written with
its accessed bit set, so the processor never writes a table that the stripped root maps read-only.

In ring 0 `IA32_GS_BASE` names this processor's entry page, and in ring 3 it is zero: every GDT
base is zero and `CR4.FSGSBASE` stays clear. An entry from ring 3 executes `swapgs`, and both paths
then execute `lfence`, so nothing runs ahead on the wrong base. The return to ring 3 executes
`swapgs` immediately before `iretq`. The NMI, double-fault and machine-check stubs do not trust the
pushed `CS`: they read `IA32_GS_BASE`, swap when it is not a kernel address and stop when it still
is not. Those three never resume, so the half-swapped `GS` they may interrupt is never returned to.
A switch loads the null `GS` selector between two `swapgs`, so only ring 3's zero base can clear.

Every IDT gate is an interrupt gate, so a handler starts with interrupts off. Generated assembly
stubs push a zero error code where the CPU pushes none, so every frame has one shape, and join
one common entry in `wost/src/arch/x86_64/interrupt.rs`. It executes `cld`, executes `clac` when
SMAP is on, loads `KERNEL_DATA` into `SS`, saves every general register and calls `dispatch` in
Rust. `dispatch` splits by the pushed `CS`.

A frame from ring 0 is trap context. `dispatch` raises the trap depth, records the live frame and
calls `on_trap` in `kernel/src/arch/x86_64/trap.rs`, which matches on a `TrapCause` decoded by
`libs/trap-cause`:

- A breakpoint resumes.
- A page fault inside the user-copy routine resumes at its fixup (section 7).
- Any other fault in ring 0 is a kernel bug and panics with the vector, error code, `RIP`, `CS`,
  `RFLAGS` and `RSP`. A double fault, machine check or NMI is terminal from either ring.
- The timer interrupt is acknowledged through an `EoiToken` minted from the live in-service bit. A
  spurious interrupt is not acknowledged, and a local APIC error panics. An unexpected vector
  panics, so a leaked legacy line cannot pass for something else.

A trap handler never logs, because the interrupted code may hold the console lock, and never
blocks. Only `task::switch_from_trap` switches from a trap, and it refuses a ring 3 frame.

A frame from ring 3 is handed to the thread it interrupted. An interrupt first runs `on_trap` as
above for its acknowledgement and clock work. Then the trap depth is zero again, and `dispatch`
mints a `ThreadContext`, wraps the frame in `UserRegisters`, sets the interrupt flag and calls
`on_user_entry` with `UserEntry::Call` (the `0x80` gate), `UserEntry::Fault(cause)` or
`UserEntry::Interrupted`. The handler runs as that thread on its own kernel stack, with the
frame at the stack top: it may allocate, block and be preempted. `UserRegisters` reads `rax`,
`rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9`, the vector, `RIP`, `RFLAGS` and `RSP`, and
`set_return` writes `rax`; nothing else in `wost` writes a ring 3 frame. A call is served, a
fault retires the program, and an interrupt lets the scheduler preempt the thread
(`thread::preempt`).

When the handler returns, `dispatch` clears the interrupt flag and validates the return: exact
user selectors, a `RIP` below `USER_END` that is user-executable in the live tables, an `RSP`
below `USER_END`, and `RFLAGS` with `IF` set and only user flags (no `NT`, `VM`, `IOPL` or
reserved bits). The flag stays clear until `iretq`, so ring 3 resumes exactly the image checked.
A refused return calls `on_invalid_return`, which retires the program.

Ring 3 enters the kernel only through vector `0x80`, the one DPL-3 gate.
`cpu::enable_protections` clears `EFER.SCE` and `IA32_SYSENTER_CS`, so `syscall` and `sysenter`
fault, and `task::enter_user` refuses while either is set. The calls are in section 6.

`cpu::establish_control_state` and `cpu::enable_protections` run after the IDT is installed, so a
bad bootloader mapping becomes a reported fault. The first sets `CR0.NE`, `CR4.MCE` and `CR4.TSD`,
clears `CR0.EM` and `CR4.DE`, `PCE` and `FSGSBASE`, writes zero to `IA32_TSC_AUX` where `RDTSCP` or
`RDPID` exists, and resets `RFLAGS`; bits the kernel does not own stay as the bootloader set them.
With `CR4.TSD` set, `RDTSC`, `RDTSCP`, `TPAUSE` and `UMWAIT` raise #GP in ring 3 before they retire,
and the fault path retires the program; the kernel never emulates them. `RDPID` cannot be trapped
and reads the zeroed `IA32_TSC_AUX`. The second sets `CR0.WP` and `EFER.NXE` and enables SMEP, SMAP
and UMIP where CPUID reports them, arming the entry path's `clac` with SMAP. Ring 3 requires NX;
missing SMEP, SMAP or UMIP is logged and tolerated.

Extended state is saved eagerly on every switch into a per-slot area sized from `CPUID.0DH`, with
`XSAVEC` where present and `XSAVE` otherwise (`wost/src/arch/x86_64/xstate.rs`). `XCR0` enables
the x87, SSE and AVX components the processor has, and a fresh slot starts from the architectural
zero image. Each switch also reloads the data selectors and the `FS` base.
Without extended state ring 3 stays closed. The kernel itself is built `+soft-float`.

`libs/x86-cpuid` decodes `IA32_ARCH_CAPABILITIES` and decides the speculation posture; an absent
capability word never counts as immunity. Boot enables enhanced IBRS where it is offered. The
return to ring 3 executes `VERW` unless the capability word reports immunity to every flaw it
clears: MDS, TAA (or no TSX), the MMIO stale-data classes and RFDS.

Where CPUID enumerates IBPB (leaf 7 `EDX[26]` or leaf `0x80000008` `EBX[12]`), `task::prepare`
issues it whenever the arriving thread's user root differs from the last user root this CPU ran.
A kernel-only thread leaves that record alone, and releasing an address space clears it, so a
recycled root frame still counts as a new space.

`apply_speculation_controls` in `kernel/src/interrupts.rs` chooses one user isolation mode per boot:

- Shared: a user address space copies the kernel's top-level entries. This needs an affirmative
  `RDCL_NO` or the exact QEMU TCG hypervisor tuple.
- Paired: each user space is a pair of roots. The full root holds the user mappings and the
  kernel half. The stripped root shares the user subtrees and maps only the entry enclave: the
  `.kpti.*` linker sections holding the entry text, the IDT and the entry flags, and every
  processor's entry area, with its entry and table pages read-only.
- Closed: neither is available, and ring 3 never opens.

Under paired roots the entry assembly loads the full root before any Rust runs, and the exit path
loads the stripped root after its last secret access and a scrub of the transition stack. The fault
stacks are not scrubbed: nothing that pushes onto them resumes. PCID is
off, so each `CR3` load flushes. An exception or NMI inside that transition window writes
`wost: kpti stop` to COM1 and halts the machine. The self-test kernel forces paired roots so the
path runs on QEMU too.

## 4. Memory

`wost::mm` owns `Frame`, `Segment` (a contiguous run) and the allocator lock; `libs/frame-space`
owns the bitmap search, two bits per frame. Only memory the bootloader marks usable is managed, up
to a 16 GiB ceiling (`wost::mm::CEILING`); RAM above it is reported and ignored. Releasing a
foreign or free frame is a visible refusal.

Every managed frame has one 8-byte record (`libs/frame-space` `Record`, `wost/src/mm/table.rs`),
carved at `mm::init` from the first contiguous run large enough. It names the owner (free, kernel,
page table, device, or one arena with the frame's kind: object, data or table) and whether the
frame is writable or sealed. Every allocation sets the record and every release checks and clears
it, so a frame freed by the wrong path stops the kernel.

A frame's cache colour is its frame number modulo the colour count, read once at `mm::init` from
`CPUID` leaf 4 (leaf `0x8000_001D` on AMD) by `libs/x86-cpuid`. The allocator keeps free counts per
colour and searches the bitmap under a colour mask. One leaf lock covers the bitmap, the pool and
the arena nodes, so the counts always agree.

Arenas (`wost/src/mm/pool.rs`) hold per-colour reservations, not frames. `pool::reserve_root`
reserves all but a margin of each colour once, `pool::donate` moves reservation from a live parent
to a new child whose mask lies within the parent's, and a refusal changes nothing. For each colour
the unused reservations never exceed the free frames, so the kernel's own allocations take only
unreserved frames and a reserved frame can always be taken. An operation calls `pool::earmark`
first and then takes `ArenaFrame`s that cannot fail; each is zeroed when taken and when released.
`pool::dissolve` walks the records in bounded steps, zeroes and frees the arena's frames, and
returns its reservation to the heir. There are 256 nodes; a node's generation is in every
`ArenaId`, and a node whose generation reaches the maximum is retired. Each node counts its
outstanding `ArenaFrame` and `SealedFrame` tokens, and dissolve refuses with no change while that
count or an earmark is not zero, so no safe handle reaches a frame it frees. A holder ends a token
either by releasing it (zeroed and credited at once) or by `surrender`, which costs constant time
and leaves the frame to dissolve's scan.

Kernel objects made for a world live in object pages (`wost/src/mm/slab.rs`). An `ObjectPage`
owns one arena object frame and holds `libs/object-pool`'s slot state,
one borrow count per slot and the objects. Slot state and objects change only through `&mut`; a
`Hold` is a counted shared borrow of one object, and a borrowed object can be neither removed nor
reached mutably, and its page cannot close or surrender. A closing page forgets its live objects
and zeroes the frame; a surrendered page forgets them and leaves the frame to dissolve. A kernel `Pool` (`kernel/src/arena/pool.rs`) keeps one object type of one arena behind a
leaf lock: its pages sit in a `Directory` (`kernel/src/arena/directory.rs`), a radix of 64-way
nodes that are themselves object pages and whose depth grows with the highest ordinal, up to four
levels. A `Local` names an object by page ordinal (22 bits), slot (10 bits) and generation. A pool
earmarks one page and a full directory path for each object it may create, takes those frames
before its lock and releases what it did not use. It keeps one empty page; any other page that
empties closes, and its ordinal keeps the least generation above every one the page issued, so a
reused ordinal never repeats a generation and an exhausted one retires.

The world registry (`kernel/src/world/registry.rs`) has one entry per arena node: the generation,
a word with the world's state, and the record pool that holds the world's record in its own
arena. A `NodeRef` is the node index and its arena generation. After the generator is seeded,
`world::start_root` draws the root world's key from it, reserves the root arena with a margin
of 1/32 of each colour and publishes the root world.

The heap (`wost/src/mm/heap.rs`) addresses frames through the direct map. Requests up to 2048
bytes go to eight slab classes from 16 bytes (`libs/slab-layout`); larger ones take whole frames.
Slab frames are never returned. Nothing may allocate before `mem::init`, and the heap refuses to
run while the trap depth is non-zero, so nothing allocates in a handler that may have interrupted
the allocator.

Memory that ring 3 or a device can write has its own type, `mm::USegment`, which admits only
plain-data copies, and `map_user` takes only a `USegment`. `USegment::zeroed` is the only way to
make one: it zeroes the `Segment` it consumes and refuses a run it cannot clear, so ring 3 sees
only zeros and the bytes the kernel writes afterwards. A `USegment` is zeroed again before its
frames return to the pool, and a run whose clear fails is withheld.

`wost/src/arch/x86_64/paging.rs` owns every table write, over the encoding in `libs/x86-paging`.
No mapping call takes a bare physical address: `map_owned` consumes allocator frames, `map_boot`
takes a `BootRegion` the boot map covers, `map_device` takes an admitted device range and
`map_user` takes a `USegment`, which it refuses to map executable. Executable user bytes come
only as a `SealedSegment` through `map_user_sealed`, which refuses `WRITABLE`. Every path ends in
`map_range`, which refuses a writable and executable flag set and preflights the whole range,
refusing it before any write unless every target is absent. `map_user` and the shared doors also refuse a range outside the user half in
every space. RAM mappings keep the direct map's
write-back type.

`unmap` returns a token that owns the frames until the TLB is flushed. Ownership of a mapped frame
is page-table software bit 9 (`Flags::OWNED`), and large pages are never split. A clear executes
`invlpg` only when a walk of the current `CR3` reaches that slot. `activate` walks the candidate
tables for its own code, stack, direct map and every framebuffer and firmware byte handed out at
boot before writing `CR3`, and refuses while NX is off or a higher-half top-level entry is
user-accessible. The kernel space is private to
`kernel/src/mem.rs`, which exports operations on it and never the `AddressSpace`. Activation
reserves the top-level entries of the device window and the entry window, and no higher-half top-level table is made
afterwards, so every user root shares one kernel topology.

The image is mapped by linker-symbol class, the entry enclave included. The direct map, at the
bootloader's offset, covers usable memory, bootloader and module memory, and the framebuffer and
ACPI memory in whole pages, and never reserved ranges or device holes. Usable memory is mapped
with one 4 KiB leaf per frame and the rest with 2 MiB pages wherever alignment allows. Every
direct-map leaf is non-executable, the image's frames included, and `activate` refuses a usable
frame without its own 4 KiB writable leaf.

W^X holds per frame. Sealing a frame (`USegment::seal`, `ArenaFrame::seal`) consumes its only
handle, marks its record sealed and rewrites its direct-map leaf read-only, the one present entry
that ever changes in place (`paging/direct.rs`). A sealed frame becomes writable again only when it
is released: the record goes free first, then the leaf turns writable, then the frame is zeroed.
Each change returns a `paging::Flush`; on one processor the change is invalidated locally before
the token is made, so every token starts complete.

| Address | What |
|---|---|
| `0xffffffff80000000` | kernel image |
| bootloader's offset | direct map of physical memory |
| `0xffffff0000000000` to `0xffffff0100000000` | device window: MMIO, uncached and never executable |
| `0xfffffe8000000000` | entry window: one 128 KiB entry area per processor (section 3) |
| `0xfffffe0000000000` | kernel thread stack arena |
| below `0x0000800000000000` (`USER_END`) | user half; mappings end one page lower (`USER_MAPPING_END`) |

Kernel stacks are 32 KiB above an unmapped 4 KiB guard, laid out by `libs/stack-layout` in an
arena of sixteen slots (`kernel/src/thread.rs`); the boot thread uses the same geometry. A stack
inside the bootloader's 2 MiB direct map cannot be guarded, which is why the boot thread leaves
the bootloader's stack. Each `TSS` or switch write of a stack top re-walks that stack's leaves in
the loaded `CR3`. Every map and unmap call refuses the entry window, so the guard pages under the
entry and fault stacks stay absent.

## 5. Threads, scheduling, locks and time

`wost/src/arch/x86_64/task.rs` owns thread slots and the context switch, a private naked function
that saves callee-saved registers and swaps stacks. The kernel names threads by slot and never
holds a context value. Every switch passes through `task::prepare`, which installs `RSP0`, the
address-space root, extended state and `FS_BASE` for the next thread.

`kernel/src/thread.rs` is the policy: names, states and EEVDF entities over the same slots, with
the arithmetic in `libs/eevdf` and a 3 ms slice. If the policy ever returns nobody while every
thread is a candidate, the kernel takes the first candidate. The idle thread sits outside the
policy, runs only when nothing else can, halts, and emits the reports calls left behind. The
other kernel threads are the keyboard poller and the service worker. A ring 0 frame is preempted
at trap exit, after end-of-interrupt, and only when it had interrupts enabled. Every spin lock
clears that flag, so critical sections need no preempt count. A ring 3 thread is preempted in its
handoff, as a thread.

A `WaitQueue` (`kernel/src/wait.rs`) is a list of thread slots linked through
`Threads::next_waiter` under the scheduler lock, so a wake from a handler allocates nothing. A
waiter checks its condition and links itself under that lock with interrupts off, so no wake can
fall between the check and the block. A wait may also end at a deadline.

`wost/src/sync.rs` has two lock kinds, and every lock names its rank as its first type
parameter: a `libs/lock-order` level, declared once per crate, whose number orders it. `wost`
declares only `Leaf`, the highest rank; `kernel/src/rank.rs` declares the storage, IOMMU, console
and serial thread locks, then the kernel address space and the scheduler, as one chain whose ranks
a constant assertion keeps ascending. `SpinLock` clears the interrupt flag while held and restores
the flag it found; setting the flag or switching threads while one is held panics.
`SpinLock::lock` borrows a `Held<M>` token mutably for the guard's life and compiles only when the
lock's level is declared after `M`. The thread's own token is `ThreadContext`, which is
`Held<Unlocked>`, so no blocking call compiles under the guard. `lock_leaf` takes no token and is
for take, mutate and return with no call-out. Each processor's per-CPU block holds a word of the
ranks it holds: a spin lock is taken only when its rank is above every one held, and a leaf guard
also holds `Leaf`, so nothing at all is taken under it. A violation panics before the lock is
waited on, and a user copy under a leaf guard panics too. `ThreadLock` is for long work such as
console output: it leaves interrupts on and refuses trap context and any held `SpinLock`. Its
holder can be preempted, so thread-lock ranks are kept in a word per thread slot, and they rank
below every spin lock. `ThreadContext` is a zero-sized, non-`Send`, non-`Sync` token, and every call
that may block takes one. `wost` mints it in two places only: `boot::enter`, which hands it to the
kernel's entry function, and `task::thread_start`, which begins every thread on its own stack and
passes `&mut ThreadContext` and one `usize` argument to the thread's entry. The boot thread's move
to its guarded stack consumes its token and restarts through `thread_start`, so trap code can
reach no token. `thread::spawn` takes the entry and its argument.

There is no unwinding: every profile uses `panic = "abort"`. `wost::panic::terminate` disables
interrupts for good, refuses re-entry, force-releases every `wost` lock through each module's
`force_unlock`, clears the held-rank words and calls the kernel reporter. The reporter (`kernel/src/panic.rs`) uses
`try_lock` only, writes a framed record (`libs/panic-record`) to serial and a panic screen to the
console, and halts the machine.

The TSC is the clock and the local APIC timer is a one-shot alarm (`kernel/src/time.rs`). The alarm
is armed only for the next slice end, sleeper deadline or world clock refresh, so an idle kernel
with nothing due takes no timer interrupts. Time is nanoseconds through one multiply and shift
(`libs/timebase`). Where the TSC is invariant and CPUID leaf 0x15 reports a complete rate,
`time::init` takes that as the TSC rate and measures only the APIC timer against it. Otherwise it
measures both against the 8254 PIT over 20 ms windows and accepts a rate when two windows agree to
one part in 64; leaf 0x16 is never used as a rate. When the PIT was the source it also checks live
APIC windows as a watchdog. Two consecutive conclusive disagreements revoke public time: `Uptime`
stays at the last trusted value and scheduling continues on a private epoch. An NVMe or VT-d wait is
bounded by the clock, through `timebase::PollLimit` for NVMe, with a spin count only when no clock
exists.

The kernel's random generator (`kernel/src/random.rs`) is a `libs/entropy` `Base` and one `Local`
behind a leaf lock: a BLAKE2s pool and ChaCha20 keys with fast key erasure. `random::seed` credits
64-bit `RDSEED` words, or `RDRAND` words on a processor without `RDSEED`, through `cpu::SeedReader`,
which exists only where CPUID enumerates its instruction and retries a clear carry a bounded number
of times. A word of zero, all ones or a repeat of the last is discarded uncredited. The generator is
ready after 256 credited bits, and the boot log names the source that seeded it. `random::fill`
refuses with `NotReady` before then, and every draw replaces the key that produced it before it
returns. The ring's `Random` draws from it in 256-byte rounds.

## 6. The native ABI

A program's external authority is its handle table. The data path is a submission and completion
ring, with four synchronous calls beside it. The constants in this section are the ABI, and
`libs/native-abi` defines each of them once for the kernel and for programs.

### Handles and rights

A handle is a 64-bit value (`libs/handle-space`; the rights are `native_abi::Rights`). Bits 0 and
1 are always set, so zero is never a handle. Bits 2 to 19 are the slot index and bits 20 to 63 a
44-bit generation. A slot whose generation would reach 2^44 retires instead of repeating one. The
value is XORed with the table's mixer: the first 8 bytes of `entropy::derive(world key, "handle",
process ordinal)` with bits 0 and 1 cleared, derived once when the table is made and stored in it.
A handle resolves only in the table that issued it, because each table has its own entries.

The universal rights are `DUPLICATE` (bit 0), `TRANSFER` (1), `WAIT` (2) and `INSPECT` (3). The
per-type rights are `READ` (8), `WRITE` (9), `EXECUTE` (10), `MAP` (11), `TRAVERSE` (12),
`ENUMERATE` (13) and `MODIFY` (14). An undefined bit is refused, never masked. The object types are
an opaque `Channel` identity, a `File` and a `Directory` (`kernel/src/object.rs`).

Each process's table is an object in its world's table pool (`kernel/src/handle.rs`). Its capacity
is the world's handles limit (64 for the root world), and its entry pages, 64 entries each, are
arena frames taken when it is made, so a ring operation never allocates for a handle. The entries
sit behind one object lock (`rank::Object`, taken with a thread context). Held and promised counts
share one atomic word; a retired slot stays counted, so the quota never promises a slot that no
longer exists. An entry names its object by `ObjectRef`: a `Local` channel in the same world, or a
`File` or `Directory` value. A channel is counted: a handle holds one strong reference, a
`Duplicate` takes another, and `Close` or the table's teardown releases it, so the channel goes
with its last handle.

A new program's table holds exactly one entry at birth: an opaque `Channel` with the universal
rights, which has no peer and carries no messages. Every later handle arrives through a ring
operation that a held handle authorizes, and `Close` ends one.

### The ring

Each program has one ring, version 2: one page mapped at `0x0000200000000000`, and buffer 0, the
data page, mapped on its own at `0x0000200000001000` (`kernel/src/queue.rs`, layout in
`native_abi::ring`). The queue is an object in its world's queue pool, and both pages are arena
data frames of that world, mapped without ownership and kept alive by the mapping. The kernel
reads ring memory once into a snapshot and acts only on copies.

| Offset | Contents |
|---|---|
| 0 | header: version (`2`), submission count (4), completion count (8), submission head (16, kernel writes), submission tail (20, user writes), completion head (24, user writes), completion tail (28, kernel writes); bytes 32 to 63 zero |
| 64 | 8 submissions of 64 bytes |
| 576 | 16 completions of 32 bytes |

A submission is opcode (`u32` at 0), token (`u32` at 4), handle (`u64` at 8), second (`u64` at
16), offset (`u64` at 24), argument (`u64` at 32), length (`u32` at 40), buffer (`u32` at 44), at
(`u64` at 48) and flags (`u64` at 56). A completion is token (`u32` at 0), status (`u32` at 4),
value (`u64` at 8), extra (`u64` at 16) and a reserved zero `u64`. A byte range names a registered
buffer by index and a byte offset in it; only buffer 0 exists until programs register more. An
opcode reads only its own fields: a nonzero field it does not use, or any flag, is refused with
`DENIED`. Each direction has one producer and one consumer: the
producer writes the entry and then release-stores the index, and the consumer acquire-loads the
index and copies the entry once. `wost` accesses each publication word with one aligned 32-bit
`mov`, because ring 3's plain stores make a Rust atomic over them unsound.

Every operation posts exactly one terminal completion. Completion space is reserved when a
submission is accepted, so the completion ring cannot overflow, and an operation that produces a
handle reserves its table slot then too. Every submission field is a value ring 3 chose, so the
kernel narrows it with `try_from` and refuses or saturates.

| Opcode | Name | Effect |
|---:|---|---|
| 1 | `Echo` | completes with the argument |
| 2 | `Uptime` | completes with world time in nanoseconds, the value the time page shows |
| 3 | `Duplicate` | needs `DUPLICATE`; completes with a new handle whose rights may only narrow |
| 4 | `Timer` | completes once world time reaches `argument` nanoseconds past now, rounded up to the precision; the value is that deadline |
| 5 | `Cancel` | ends the oldest operation whose token is `argument`; the target still completes once, with `CANCELLED` |
| 6 | `OpenAt` | resolves the path in `buffer`, `at`, `length` from a directory handle; completes with a new handle. A path naming the directory itself is `BAD_PATH` |
| 7 | `Read` | reads file `handle` from file offset `offset` into `buffer` at `at`, up to `length` bytes |
| 8 | `Random` | fills `length` bytes of `buffer` from `at` with generator output; completes with the byte count, or `UNAVAILABLE` before the generator is seeded |
| 9 | `Close` | ends `handle`; its object goes once no handle or kernel reference holds it |

The statuses are `OK` (0), `UNKNOWN_OPCODE` (1), `NO_HANDLE_SLOT` (2), `BAD_HANDLE` (3), `DENIED`
(4), `CANCELLED` (5), `NOT_FOUND` (6, a cancel that named nothing in flight), `NO_ROOM` (7),
`BAD_BUFFER` (8), `NO_ENTRY` (9), `BAD_PATH` (10), `UNAVAILABLE` (11) and `IO` (12, a device or
volume failure). A pending operation stays
in its slot until its completion is posted, and ring pages are reclaimed only once the queue is
quiet. Cancellation is advisory.

`OpenAt`, `Read` and `Random` move data through buffer 0, so the kernel validates no user pointer
for them. A file opened from a directory gets `READ` plus the directory's universal rights. No program
can hold a directory handle today, because spawn installs only the bootstrap channel.

### Calls

The `0x80` gate dispatches on `rax` (`kernel/src/user/call.rs`, numbers in `native_abi::Call`).

| `rax` | Call | Effect |
|---:|---|---|
| 0 | `Exit` | ends the thread with the status in `rdi`; the only call that does not return |
| 1 | `RingEnter` | reaps up to the completion head, expires due timers, serves submissions; returns how many completions it posted |
| 2 | `ThreadBase` | sets `FS_BASE` to `rdi`; `-EINVAL` outside user space |
| 3 | `RingWait` | returns at once when nothing is in flight, else sleeps until an operation is due and serves it |

An unknown number answers `-ENOSYS` (`-38`) and is counted for the idle thread to report. Calls
run in the calling thread's context with interrupts on. `RingWait` blocks on the ring's wait queue
until a completion is posted or the earliest deadline passes, and file work goes to the service
worker (section 9).

### Entry block and time page

A program starts at its ELF entry with `RSP` 16-byte aligned on a 32-byte entry block: version
(`u64` at 0, value `2`), the launcher's argument (`u64` at 8), the bootstrap handle (`u64` at 16)
and the launcher's operand (`u64` at 24). The argument is also in `rdi`; every other general
register is zero.

The time page is mapped read-only at `0x0000100000000000` in every program
(`kernel/src/timepage.rs`, layout in `native_abi::timepage`). Version 2 holds constants written
before any mapping exists: version (`u32` at 0, value `2`), page size (`u32` at 4), mode (`u32` at
8: `0` none, `1` normal), precision in nanoseconds (`u64` at 16) and generation (`u64` at 24, never
0). One aligned `u64` at 64 holds world time in nanoseconds. Ring 3 cannot execute `rdtsc`, so the
page carries a value, not a conversion, and needs no sequence lock.

World time belongs to a world's clock, a `timepage::WorldClock`: host time minus the host time at
which the world started, so it starts at 0 and never reveals host uptime. A served value is world
time rounded down to the precision (1 ms) and raised by an atomic maximum, so it never goes back;
the page word is raised by a compare-and-swap loop in `wost`. `Uptime` serves the same value, and a
`Timer` deadline is a multiple of the precision. The page is refreshed before every return to ring
3, and while a thread of the world runs the scheduler arms its alarm at least every max(precision,
1 ms), so a program spinning in ring 3 sees time advance. The cost is one timer interrupt per
millisecond on a processor running such a thread, and none while no world thread runs. Spawn takes
the clock as a value; today every process gets the one root clock, `timepage::root`.

## 7. Processes and the loader

`user::spawn` (`kernel/src/user/construction.rs`) refuses unless every ring 3 precondition holds, in
this order: interrupts admitted, the boot thread on a guarded stack, NX on, `CR4.TSD` set, extended
state isolated, the clock calibrated, the random generator seeded, the service worker running, and a
shared or paired isolation mode. It then builds the address space, loads the image, maps a one-page
user stack at `0x0000300000000000`, maps the time page, the ring and buffer 0, creates the handle
table with its birth handle, binds the space and publishes a thread. Before it builds anything it
earmarks, in the root world's arena, every frame its queue, table and birth channel can need, so a
full arena refuses the spawn with nothing changed. `ProcessConstruction` owns the space and
every mapped range until the thread is published, so any refusal rolls back through one owner.

A process today is a thread slot. Its record (`Bootstrap` in `kernel/src/user.rs`) is one thread,
one address space, and the `TableRef` and `QueueRef` that name its table and ring in the root
world, in global arrays indexed by slot. There is no
parent and no fork; the kernel spawns one program at boot. How a program ended is kept by slot
when it retires: `Exit` stores its status, and a fault stores the vector, error code, `CR2` (for a
page fault, read before interrupts reopen; zero otherwise) and `RIP`. The service worker writes it
as an `exit` or `fault` boot record once the program's memory is reclaimed. Nothing yet reads it
back into ring 3.

The loader (`kernel/src/user/load.rs`, over `libs/elf64`) maps the Limine module. It refuses a
segment that is writable and executable, runs past the file, overlaps another, is misaligned or
reaches the kernel half, and checks that the entry lies in an executable segment. Pages are filled
before they are mapped read-only or executable. This in-kernel ELF loader is temporary until the
Phase 7 user-space loader replaces it.

`witness.elf` is the ring 3 witness, the Rust program `witness/ring3` over the runtime
`witness/rt`, which alone outside `wost` may write `unsafe`. Its entry block argument picks a
behaviour: an `Echo` or a `Duplicate` through the ring, an unserved call, a fault, a jump into the
kernel half, a direct counter read, an invalid return stack or flags, a read of the kernel-half
address in the entry block's operand, `RingWait` idle and after a `Timer`, `ThreadBase` with a user
base and with refused ones, or `syscall`, which faults with `#UD` because `EFER.SCE` is clear.
Self-tests in `kernel/src/selftest/processes.rs` attribute each ending to its program. It checks
the entry block
and ring versions, and it reports by how it ends: exit status 0 is agreement, a nonzero status
names the first wrong answer, and a behaviour built to fault ends in that fault. It is a boundary
check, not a userland. The runtime sets `AC` only across `int 0x80`, so no program access runs
under alignment checking.

`xtask` builds it always in release, in `target/witness`, with `-C code-model=small` appended to
the kernel target's flags and no LTO. It links a static `ET_EXEC` at `0x400000` through
`witness/ring3/link.ld`. Its text, read-only and writable loads are page aligned, and program
header 0 is the text load. `xtask/src/image/witness.rs` refuses an image with a dynamic section or
interpreter, an empty, misaligned or writable and executable load, more than 8 loads, or a load
past 2 GiB. The loader maps each segment at its `p_vaddr`; only stacks move per program. The
idle thread logs `user: ring 3 call observed` once after the first call from CPL 3.

`copy_from_user` and `copy_to_user` (`wost/src/arch/x86_64/usercopy.rs`) are the only code that
opens the SMAP window, for one copy. Each first walks every page and requires effective `USER`
permission, checks the range against the user limit by comparison, and recovers a fault at a
linker fixup that only the trap adapter writes.

A program that exits or faults becomes a zombie, which the service worker collects. A zombie keeps
its ring and everything its in-flight operations can reach until `Queue::abandon` has ended each
with a `CANCELLED` completion; then its ring and handle table go back to the world's pools, the
table releasing every object its entries held, and its address space is released. A hold that
still borrows the ring or table refuses that step, and the next collection retries it.

## 8. DMA isolation and PCI

The kernel's first act, before memory initialization, is `iommu::secure_handoff`. It accepts only
the CPU-identified QEMU TCG or VMware platform, because a device can rewrite RAM but cannot forge
the hypervisor CPUID leaf. It requires a checksummed `WZDH` ACPI table, which `xtask` supplies to
both machines, attesting CF8/CFC reach, static requesters and quiescence until this point. It then
clears and reads back Bus Master Enable on every function of segment 0; the walk owns the
configuration ports for its length instead of holding their lock, so no claim starts during it. VMware's 440BX host
bridge, whose BME bit is read-only, is kept as a qualified non-requester, and its PIIX4 ISA bridge
is fenced by masking both 8237 controllers. Any other refusal panics, so a machine without
CF8/CFC configuration access does not boot.

`iommu::discover` validates the DMAR. Before the first topology read, `survey_the_bus` takes over
the sole segment-zero VT-d unit with an empty root and an empty interrupt-remapping table, so every
DMA request and every device interrupt faults. Takeover needs whole-segment coverage proved
statically (an `INCLUDE_PCI_ALL` unit, or a platform `WZDM` table naming the unit) and no reserved
memory region, and refuses firmware that left translation, queued invalidation or interrupt
remapping on. Translation is never turned off afterwards. `kernel/src/iommu.rs` holds the policy
and `wost/src/arch/x86_64/vtd.rs` the registers, over the encodings in `libs/vtd`.
`pci::snapshot_topology` then seals every bus, device and function into one `PciTopology`, whose
bridges form a single-parent tree with nested bus intervals; a malformed DMAR scope invalidates
the whole DMAR.

For a storage candidate the domain identity-maps only frames that a live `DmaSegment` owns. Once a
translation may exist, a failed map, unmap or invalidation retains the segment, and revocation
completes a drained global invalidation before frames return. Before a unit-wide write the kernel
claims and fences every other requester the unit governs, from an inventory of at most 64
functions. Bus mastering is admitted only by spending that function's `DmaDomain` proof, and
admission first enables it on each bridge above the function.

Interrupt remapping is mandatory and denies everything. The first `sti` and bus-master admission
both need a non-duplicable isolation proof, so a hostile MSI cannot inject a vector. Every I/O APIC
line stays masked, and NVMe and the keyboard are polled. Without a usable unit, storage does not
start and the log says `storage: no DMA isolation`.

Configuration access is CF8/CFC only, 256 bytes per function. Enumeration walks bridges with a
worklist, because q35 and VMware place functions behind them (`kernel/src/pci.rs`). A driver
claims a function, not a port range, and the claim mints one non-duplicable authority bound to the
registry, address and generation; every decode, bus-master, command and BAR transition borrows
it. BAR sizing is one restored transaction in `libs/pci-config`, and memory decode follows a
`MAXPHYADDR`-checked, page-rounded admitted range. Permission that depends on hardware state is a
non-`Copy` token minted from a fresh observation of that device, such as `ControllerStopped` after
`CC.EN = 0` and `CSTS.RDY = 0`. A device retirement that cannot be verified keeps its authority in
a bounded quarantine and halts storage selection.

## 9. Storage and the VFS

`survey_the_bus` tries each NVMe function in turn and keeps the first that passes. Selection holds
a function, its windows and its mapping as a lease: the candidate is isolated, disabled, admitted
to bus mastering, started and made to read block 0. A failed candidate is retired by exact
rollback before the next is tried. QEMU and VMware each present a namespace-less controller first,
so the retirement path runs on every boot.

The driver (`kernel/src/nvme.rs`, over `libs/nvme`) brings up one controller with one I/O queue
pair, polled and read-only. Queue size, doorbell stride, timeouts and page sizes come from `CAP`,
and every poll is bounded. Controller interrupts are masked through `INTMS`. A namespace must use
512-byte blocks with no metadata, and one command is outstanding at a time. A command with an
uncertain outcome is resolved by clearing `CC.EN` and
observing `CSTS.RDY = 0`; otherwise every queue and buffer frame is withheld for the boot. A
timeout or protocol anomaly stops the controller for the rest of the boot with one logged reason,
and later reads complete with `IO`.

`libs/fat32` is a read-only, allocation-free FAT32 reader over any `Sectors` source, the same crate
on the host and in the kernel. The volume starts at sector 2048 of the namespace
(`kernel/src/fs.rs`), no partition table is read, and a volume larger than the namespace is
refused. A cluster chain is bounded by the volume's
cluster count and a directory walk by a fixed entry budget with a cycle check. Names are 8.3, and
long-name entries and the volume label are skipped.

The VFS (`kernel/src/vfs.rs`) abstracts directories and dispatches by enum over three backends:
FAT32, tmpfs (a small fixed arena populated once at boot, `kernel/src/tmpfs.rs`) and devfs, which
names `storage` and `console` (`kernel/src/devfs.rs`). A device is its own `Node` variant, which
`vfs::read` cannot take. `libs/path-walk` resolves `..` lexically and cannot ascend; absolute
paths, escapes and over-long names are refused, and there is no root. Reads have no cursor: each
`Read` names its offset.

`RingEnter` only validates and admits `OpenAt` and `Read`. One kernel worker fed by a fixed
eight-slot queue (`kernel/src/queue/work.rs`) does the VFS and device work and posts the
completion; a full queue answers `NO_ROOM`. `OpenAt` reserves its handle slot and copies its path
at admission. The in-kernel FAT reader and file worker are temporary until Phase 10's object store
replaces them.

## 10. Console and input

The serial log is a polled 16550 at COM1, 115200 8N1, with a loopback probe at bring-up so a
missing controller is reported (`kernel/src/arch/x86_64/uart.rs`). Records carry a level, with a
compile-time ceiling of `Info`. QEMU sends serial to standard output; VMware writes it to a log
file in the machine's directory.

The framebuffer console (`kernel/src/console.rs`) draws Noto Sans Mono bitmap glyphs, packing
pixels from the bootloader's channel masks (`libs/pixel`). Once `boot_continue` attaches a RAM
shadow of the screen, it scrolls from the shadow, so the device is not read. The framebuffer is
mapped write-combining through PAT entry 1 with exactly one alias. The console carries the
banner, echoed keys and the panic screen; diagnostics stay on serial.

The PS/2 keyboard (`kernel/src/arch/x86_64/i8042.rs`, `kernel/src/input.rs`) is brought up after
interrupts are on. `libs/ps2-keys` decodes scancode sets 1 and 2 and reads the set in use from the
arriving bytes, because the configuration byte and the identity reply are both unreliable. Since
every I/O APIC line stays masked, a poller thread drains the controller every 10 ms and wakes the
boot thread. Every fixed device is reached through one permanent typed port claim in `wost`, and a
claim reserves every port its accesses drive.

## 11. The build, the image and the gate

`xtask` builds the kernel and writes the MBR, the FAT32 partition and the VMware descriptors in
Rust with no external imaging tool, on Linux and on Windows with the GNU toolchain. Rust is pinned
in `rust-toolchain.toml`. Every vendored binary has a SHA-256 in a `CHECKSUMS` file, and image
assembly reads Limine once and writes that same checked buffer. `supply-chain/inventory.txt` gives
every locked crate a reviewed license row, and the only source is crates.io. Kernel builds pass one
exact rustflags list with `--remap-path-prefix`, so shipped debug information names no host path.

The QEMU machine is fixed (`xtask/src/qemu.rs`): `pc-q35-9.2` under TCG with
`-cpu max,hypervisor=on,tcg-cpuid=on`, one CPU, 512 MiB, an `intel-iommu` with interrupt
remapping, a PCI bridge, two NVMe controllers, EDK2 firmware from the QEMU install and the `WZDH`
table. QMP drives controlled boots over bounded exchanges. `cargo xtask vmware` generates
`vm/self-test/` or `vm/shipped/` with the same devices in VMware form and two virtual CPUs.

Checks that need a machine are compiled into the kernel under the `selftest` feature
(`kernel/src/selftest.rs`) and report a verdict through QEMU's `isa-debug-exit` port. The feature
never ships in an image. `cargo xtask ci` is the gate: formatting, clippy, host tests, the supply
chain and vendored digests, the self-test kernel in release and debug, a shipped boot that must
settle with no panic, record the witness's exit with status 0 and consume an injected key, and
degraded boots
without x2APIC and without an IOMMU. `README.md` lists the steps and the tasks outside the gate.

The gate judges a boot only by boot records (`libs/boot-record`), the panic record and the exit
code. A boot record is one serial line, `[info ] record v3 <kind> <key>=<value>...`, that the
kernel writes beside the sentence a person reads. The kinds are `verdict` (`passed`, `failed`,
`expected`), `fact` (`name`, from a closed list), `protections` (`wp`, `smep`, `smap`, `umip`,
`nx`, each `0` or `1`), `memory` (`colours`, the cache colour count), `exit` (`status`) and
`fault` (`vector`, `error`, `address`, `rip`). The self-test boot
requires more than one colour: its `-cpu` line sets `x-vendor-cpuid-only=off`, so QEMU's
AMD-branded TCG processor answers `CPUID` leaf 4. Every field is required and in order, and
numbers are canonical decimal.
A line that opens as a record but has an unknown version, kind, field or fact fails the boot.
Each boot names the records it requires (`xtask/src/qemu/boot.rs`, `xtask/src/qemu/serial.rs`),
and `qualify-vmware` reads a VMware transcript the same way. The `FAILED` sentences of a
self-test name failures for the message only.

## 12. Models

`models/Ring.tla` checks ring admission: no dropped completion, reservations equal to unreported
operations, and never more completions than operations. `models/RingMemory.tla` checks index
publication with terminal ownership. `models/Wakeup.tla` checks the `libs/wake-state` word against a
wake and a timeout over two waits: a blocked thread stays reachable by a waker, and a wait completes
once, by a claim on that wait. `models/Shootdown.tla` checks `libs/tlb-generation` on three CPUs
with two concurrent unmaps: a frame is freed only when every CPU that holds its translation is off
the space with a slot seen below the flush, and every flush wait ends. `models/Pin.tla` checks the
counted pin on a world node: no pinned world's frames are reused, and no pin is counted after the
pin word closes. `models/Call.tla` checks the `libs/channel-state` call cell over two calls on one
cell, racing reply, close of the reply right, cancel, timeout and the death of either side: each
call has exactly one outcome, a reply right succeeds at most once and only on the call it names, a
received call is never peer closed, and each moved handle is in exactly one place. Each `*.cfg`
named after a mutation must violate its named invariant.
`cargo xtask model-check` runs them with the digest-pinned `tools/tla/tla2tools.jar`.

## 13. What the kernel trusts

- The bootloader and CPU honour their mapping and instruction contracts. Parsers bound every
  reported range but cannot make an unmapped physical address safe to read.
- Firmware describes the machine. ACPI tables are read only from ACPI memory through
  `EarlyFirmwareMemory` and `FirmwareMemory`, after their length, checksum and signature check
  out. The MADT's I/O APIC addresses and the DMAR's register base are trusted once admitted as
  device ranges.
- The `WZDH` table and the QEMU TCG CPUID tuple are trusted configuration on the two hypervisors.
  A hostile monitor can spoof CPUID, so they attest nothing elsewhere, and bare metal refuses to
  boot.
- A device can ignore writes, so state that gates memory or interrupt safety is read back or its
  uncertainty is retained. Where an x2APIC MSR write offers no readback, the architecture is
  trusted.
- A device that ignores Bus Master Enable is unconstrained until the entry fence, and a device sees
  the physical addresses of its buffers, because the domain is an identity map.
- A VMware boot is read by a person and checked by `qualify-vmware`. Its transcript carries no
  per-boot challenge, so a copied earlier transcript would pass.
