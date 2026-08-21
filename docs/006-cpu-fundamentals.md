# 006 — CPU Fundamentals (Phase 2)

The phase that makes the machine controllable: our own descriptor tables, a
fault path that reports instead of rebooting, the CPU's user/supervisor
protections switched on, and — once those hold — an interrupt controller, a
timer, and a keyboard.

Phase 1 left a kernel that can print and halt. It cannot survive a bug: a
divide by zero, a bad pointer, or a stray `int` triple-faults the machine and
VMware silently resets. Everything after this phase (paging, threads,
userspace) generates faults as a matter of course, so the fault path is the
first thing that has to exist.

References: Intel SDM Vol. 3A ch. 3 (protected-mode memory management), ch. 6
(interrupt and exception handling), ch. 7 (task management, for the 64-bit
TSS), ch. 11 (the advanced programmable interrupt controller); Intel 8259A
datasheet; OSDev wiki GDT/IDT/APIC; Osdev-Notes' interrupt chapter.

## Order of work

Each slice boots, is checked on the machine, and leaves the tree green. The
order is forced by dependency: nothing can take an interrupt until a fault can
be reported, and nothing should enable interrupts until the legacy controller
is silenced.

| Slice | Contents | Status |
|---|---|---|
| 1 | GDT + TSS, IDT, exception entry and reporting, SMEP/SMAP/UMIP/NX | **landed** |
| 2 | Legacy PIC silenced, local APIC, IRQ-safe locking, interrupts on | **landed** |
| 3 | APIC timer: calibration, periodic tick, uptime | **landed** |
| 4 | PS/2 keyboard: scancode set 2 decoding, key events | **landed** |

Each slice's design is recorded below as it is built, so that this file ends
the phase as the description of what exists.

## Slice 1 — the CPU takes exceptions

### What owns what

`wost` owns every instruction and every table the CPU reads directly:
`arch/x86_64/segment.rs` (GDT, TSS, selector reloads), `arch/x86_64/interrupt.rs`
(IDT, entry stubs, dispatch), and the CPUID/CR4/EFER reads and writes in
`arch/x86_64/cpu.rs`. The kernel sees a safe surface: `segment::init()`,
`interrupt::init(handler)`, `cpu::enable_protections()`, and a `&mut TrapFrame`
handed to its handler.

The *encoding* — which bit of a descriptor means present, where a 64-bit gate
splits its offset, what a TSS looks like in memory — is arithmetic over bits
with no hardware behind it, so by the rule in `005-code-conventions.md` it
lives in a `libs/` crate and is unit-tested on the host. That crate is
`libs/x86-tables` (`gdt`, `idt`, `tss`, `exception`). `wost` builds descriptors
from it and does nothing to them but load them.

### GDT

Limine leaves us in long mode on its own GDT, which it is entitled to reuse or
free. We install ours before anything else touches the CPU.

| Index | Selector | Contents |
|---|---|---|
| 0 | `0x00` | null |
| 1 | `0x08` | kernel code — present, DPL 0, executable, readable, `L=1` |
| 2 | `0x10` | kernel data — present, DPL 0, writable |
| 3–4 | `0x18` | the TSS (a system descriptor, 16 bytes, so it takes two slots) |

Base and limit are ignored for 64-bit code and data segments; they are written
as zero rather than as the traditional `0xFFFFF` flat span, because a value the
CPU ignores should not look like a value that matters.

There are no ring-3 descriptors yet. They arrive in Phase 5 with the first user
thread, and `SYSCALL`/`SYSRET` will constrain their order (the STAR MSR
requires user data immediately after user code); appending them then costs one
line, and inventing them now would mean writing a layout for an ABI that is not
yet designed. `005`'s "no speculative API" applies to tables as much as to
functions.

Loading the GDT does not change `CS`: the selector cached in the CPU stays
whatever Limine left. We reload it with a far return — push the new selector,
push a label address, `retfq` — and then load `SS`, `DS`, `ES` with the data
selector. `SS` matters even in 64-bit mode: `iretq` pops it on every return
from an exception.

### TSS

64-bit mode uses the TSS for exactly two things: the privilege-level stacks
(`RSP0`, needed when ring 3 exists) and the interrupt stack table. We need
`IST1` now, for the double fault.

A double fault is the CPU telling us that handling one fault raised another —
overwhelmingly, that the stack is unusable. Handling it on the same stack
guarantees a triple fault, which on hardware is a reset with no message. `IST1`
points at a dedicated 16 KiB stack in `.bss`; gate 8 selects it, and every
other gate uses the interrupted stack.

`iomap_base` is set to the size of the TSS, which is how x86 spells "this task
has no I/O permission bitmap". Any ring-3 `in`/`out` will fault, which is what
we want; the port registry in `wost::arch::current::port` is the kernel's own
exclusivity mechanism and is unrelated.

### IDT and the entry stubs

All 256 gates are filled. The 32 architectural vectors get the real handlers,
and vectors 32–255 get a stub that reports "unexpected interrupt" — cheap
insurance, because the alternative to a filled gate is a #GP that becomes a
double fault that becomes a reset. All gates are *interrupt* gates (type `0xE`),
so the CPU clears `IF` on entry: no handler can be interrupted before it has
decided whether that is safe.

Rust cannot write an interrupt handler directly. The `x86-interrupt` calling
convention is a nightly feature (`abi_x86_interrupt`) and D-01 keeps us on
stable, so entry is assembly: a per-vector stub that normalises the frame, and
one common tail that saves registers and calls Rust.

The two things a stub must fix before Rust can see the frame:

1. **The error code is not always there.** Vectors 8, 10–14, 17, 21, 29 and 30
   push one; the rest do not. Stubs in the second group push a zero, so the
   frame has one shape.
2. **The vector number is not on the stack at all.** Each stub pushes its own.

The stubs are generated by `.rept` in a `global_asm!` block and padded to a
uniform 16 bytes, so stub *n* is at `table + 16n` and the IDT is filled by a
loop rather than by 256 hand-written names. That the stride assumption holds is
not left to inspection: the table is bracketed by two symbols and `init`
asserts the span is exactly `256 × 16` bytes.

The common tail pushes the fifteen general-purpose registers, clears `DF` (the
System V ABI requires it and an interrupt does not), points `RDI` at the saved
frame, and calls the dispatcher. On return it pops, drops the vector and error
code, and `iretq`s. The stack is 16-byte aligned at the `call` by construction:
the CPU aligns to 16 before pushing its five words, and error code + vector +
fifteen registers restore that alignment exactly.

`TrapFrame` is `#[repr(C)]` in the order the pushes leave in memory, which is
the reverse of the order they are written. It is the only place in the kernel
where a struct definition and an assembly listing have to agree, so the
selftest checks the agreement on the machine rather than by eye.

**Not saved:** floating-point and SSE state. `x86_64-unknown-none` builds with
`+soft-float` and no SSE, so the kernel cannot disturb it. That changes when
ring 3 exists and the scheduler switches user threads (Phase 4/5), which is
where `XSAVE` belongs.

**Not done:** `swapgs`. There is no user mode to come from, so the `GS` base is
uninteresting; the check "did we come from ring 3" that guards it belongs with
the code that first makes ring 3 reachable.

### What a fault does

The kernel's handler is a safe `fn(&mut TrapFrame)` registered at init. It:

- **Vector 3 (breakpoint)** — records the frame and returns. `int3` is a
  deliberate trap: `RIP` already points past it, so returning resumes.
- **Everything else** — panics, with the vector name, the error code, and
  `RIP`/`CS`/`RFLAGS`/`RSP` in the message. The panic handler force-unlocks the
  consoles and halts, so a fault taken while the log was locked still prints.

Nothing is recovered and nothing is retried. That is D-14 applied literally: a
fault in ring 0 is a kernel bug, and the containment mechanism is supervised
restart, which needs a supervisor we do not have. When user threads exist, the
same handler grows a "was this ring 3?" branch that kills the thread instead —
one branch, in the one place faults arrive.

The handler must not log. Taking the console lock in trap context deadlocks
against interrupted code that already holds it — the hazard `001-boot.md`
records against `SpinLock`. The panic path is exempt because it force-unlocks
first; the breakpoint path records into atomics and lets its caller do the
printing.

### Security baseline

D-12 requires the CPU's protections on from the start, not retrofitted. What
can be enabled before paging is ours:

| Bit | Where | Effect |
|---|---|---|
| NX | `EFER.NXE` | the no-execute page bit is honoured rather than reserved |
| SMEP | `CR4.SMEP` | ring 0 cannot *execute* from a user page |
| SMAP | `CR4.SMAP` | ring 0 cannot *read or write* a user page |
| UMIP | `CR4.UMIP` | ring 3 cannot run `SGDT`/`SIDT`/`SLDT`/`SMSW`/`STR` |

Each is enabled only if `CPUID` reports it (UMIP is Cannon Lake and later;
VMware and QEMU without `+umip` will not have it), and the result is read back
from the register rather than assumed. The kernel logs what is actually on.

Enabling SMEP and SMAP against page tables we did not build is the one real
risk here: if Limine had mapped the kernel or the framebuffer user-accessible,
the first instruction fetch or the first pixel would fault. It does not — and
the ordering makes the experiment safe. The IDT is installed first, so if the
assumption were wrong the machine would print a page fault with a `RIP` instead
of resetting. That is what a fault path is for.

W^X — the other half of D-12 — is a property of mappings, so it belongs to
Phase 3, where the kernel builds its own page tables. Enabling NX now is what
makes it enforceable then.

`AC` (the SMAP override flag) gets no accessor. Nothing in the kernel may touch
user memory yet, and the primitive to do it — `stac`/`clac` around a checked
user copy — is designed with the copy, not before it.

### Interrupts stay disabled

`IF` remains clear through this slice. The legacy PIC is still wired to whatever
the firmware left, the local APIC is not configured, and `SpinLock` still does
not save the interrupt flag. Enabling interrupts before those three are fixed
would be taking an IRQ into a kernel that cannot lock safely.

So nothing enables interrupts in this slice, for the same reason the function
to do it was deleted in Phase 1: its safety contract has to say what the caller
must have already arranged, and until the PIC is silenced and `SpinLock` is
IRQ-safe, that contract would be a guess. It arrives in slice 2 as
`interrupt::enable`, with its first caller — and by then both preconditions are
values it can assert rather than prose.

### How it is checked

Host tests (`libs/x86-tables`): descriptor and gate bit layouts against values
derived from the SDM tables, the TSS's size and field offsets, selector
arithmetic, and the error-code and name tables.

On the machine (`kernel/src/selftest.rs`, run by `cargo xtask test-boot`):

- the code selector and task register hold what we installed — proof the GDT
  and TSS were loaded and `CS` was really reloaded
- the IDT the CPU holds is ours, with the limit we wrote
- a breakpoint is delivered and execution resumes past it
- the frame that handler saw is the frame the CPU pushed: vector 3, error code
  0, `CS` = kernel code, `RFLAGS` bit 1 set, `RSP` within a page of the
  interrupted stack pointer — the check that catches a `TrapFrame` whose fields
  have drifted out of step with the assembly
- every protection `CPUID` reports is actually enabled
- a one-shot latch admits exactly one caller

The double-fault IST cannot be checked without deliberately destroying the
stack, and the vectors that end in a panic cannot be triggered by a test that
has to return a verdict. Both are structural: verified by reading back the
gates, and by the VMware boot that ends every phase (D-18).

## Slice 2 — the CPU takes interrupts

Slice 1 left a kernel that can be interrupted by itself. This slice makes it
interruptible by hardware, which needs three things that are not obviously
related: nothing else may still be wired to the CPU, a controller has to be
configured to deliver, and every lock in the kernel has to survive being
interrupted. All three are preconditions of the same one-instruction step, and
the order is forced.

### What owns what

`wost` gains two drivers and one primitive: `arch/x86_64/pic.rs` (the 8259
pair), `arch/x86_64/apic.rs` (the local APIC), and the interrupt flag, which
moved from `cpu.rs` into `interrupt.rs` — next to the table whose existence is
the reason setting it is safe. `arch/x86_64/msr.rs` is the model-specific
register accessor both `cpu` and `apic` now share; it was private to `cpu`.

The APIC's encoding — which register is where, which bit enables what, how a
local vector table entry packs a vector and a mask — is arithmetic with no
hardware behind it, so by the same rule that produced `x86-tables` it is
`libs/x86-apic` (`base`, `command`, `local_vector`, `register`, `spurious`,
`version`), unit-tested on the host. `wost` encodes with it and does nothing
but write the result.

The kernel gains `arch/x86_64/vector.rs`: which vector carries what. That is
policy, not mechanism, so `apic::init` takes the two vectors it needs as an
argument rather than choosing them.

### x2APIC, and why the choice was not free

The local APIC has two encodings of the same registers. The original — xAPIC —
is a page of memory-mapped words at a physical address the processor reports in
`IA32_APIC_BASE`, conventionally `0xFEE00000`. x2APIC puts the same registers
in model-specific registers, reached with `rdmsr`/`wrmsr`.

The first implementation used xAPIC, reaching the page through Limine's direct
map of physical memory. It page-faulted on the first read, at
`0xFFFF8000FEE00030` — and that fault, printed with an address instead of
resetting the machine, is precisely what slice 1 was for. Under Limine base
revision 3 the direct map covers the regions the memory map lists; a device
hole is not one of them. Mapping the page is therefore work the kernel cannot
do until it owns page tables, which is Phase 3.

So: **x2APIC only.** Its registers need no mapping at all, which is what lets a
phase about the CPU bring up the interrupt controller without first becoming a
phase about memory. It is also less machinery, not more — no direct map, no
volatile memory access, no argument about aliasing a device page between a
configuration write and an end-of-interrupt from a handler. Every x86_64
processor since 2008 has it, and SMP will want it regardless; a processor
without one is reported (`ApicError::NoExtendedMode`), not worked around.

Registers are still *named* by their xAPIC offset, because that is how SDM
table 11-1 names them, and `Register::msr` converts: `0x800 + offset / 16`.
Both columns of that table are transcribed into the host tests, so the
derivation has something independent to be wrong against.

The direct map went with it. `BootInfo` briefly carried one; with no caller
left it is deleted rather than kept for Phase 3, under `005`'s no-speculative-
API rule — the more so because the bound it enforced turned out to be false.

### Silencing the 8259s

Whatever the firmware left the legacy pair programmed with is still connected
to this CPU. `pic::silence` walks both through the initialisation sequence and
masks every line.

Remapping a controller that is about to be masked anyway looks redundant, and
is not. The firmware leaves the primary delivering IRQ 0 as vector 8 — the
double fault — so anything that did leak through would arrive as the one
exception whose report is guaranteed to be a lie. The vectors move to
`0x20`–`0x2F` first, and then everything is masked.

Two details worth recording. The first word of the initialisation sequence
clears the interrupt mask, so for the length of `remap` every line the firmware
left connected is unmasked; that is harmless only because the whole sequence
runs with `IF` clear, which is a property of *when* this is called and not of
the function. And the classic `io_wait` between accesses is deliberately
absent: it covers the bus recovery time of a discrete 8259A, and on every
machine this kernel boots the pair is emulated by the chipset or the
hypervisor.

The other half of the same job is in the APIC: firmware conventionally wires
LINT0 to the legacy controller's output and LINT1 to the non-maskable
interrupt, so both pins are masked during `apic::init`. Masking the 8259s
without masking LINT0 would leave the path open at the other end.

### Bringing the APIC up

In order, because the order is the design:

1. `cpuid` for a local APIC, then for x2APIC. Absent either, report and stop.
2. Enable in `IA32_APIC_BASE`, then switch to x2APIC — **two writes**. The
   architecture's state machine has no edge from disabled straight to extended
   mode; a single write setting both bits raises a general protection fault.
   Each is read back rather than assumed, because a disabled APIC stays
   disabled until reset on some processors and the write is simply ignored.
3. Read the version register. A version below `0x10` means nothing is
   answering as an APIC, which is the one failure the previous steps cannot
   detect.
4. Task priority to zero — accept every priority class. Firmware may not have
   left it there.
5. Mask LINT0, LINT1 and the timer.
6. Give the error line a real vector, and clear whatever the firmware
   accumulated in the error-status register so the next reading is ours.
7. **Last**, write the spurious-interrupt register, whose bit 8 software-enables
   the whole controller. Nothing it is capable of delivering can arrive before
   this, so everything above happens against a controller that is off.

The interrupt flag is not set here. That is a separate step with a separate
precondition, which is the next section.

### Locks that survive being interrupted

`SpinLock` did not touch the interrupt flag, and `001-boot.md` recorded the
hazard: a lock taken in ordinary kernel context and also taken from a handler
deadlocks, and on one CPU that is not a race but a certainty. The fix is in the
lock, not at the call sites — acquiring one clears `IF`, and the guard restores
it on drop.

Restoring, not enabling, is the whole of the correctness argument. An inner
lock taken while an outer one is held saw the flag already clear and puts back
a clear flag, so nesting cannot hand interrupts back early. The release order
matters for the same reason: the guard unlocks and *then* restores, because the
other order opens a window in which an interrupt could arrive at code still
holding the lock.

Two hazards survive, and both are entries the flag does not mask. A fault can
be raised while a lock is held, and a non-maskable interrupt can arrive at any
time at all. So the rule from slice 1 stands — **a trap handler still must not
log** — but it now stands for a narrower reason: not because interrupts are
unsafe around locks, but because exceptions and NMIs are.

The cost is interrupt latency. A section holding a lock cannot be preempted, so
a long one — scrolling the framebuffer console — delays every interrupt behind
it. That is a scheduler's problem and waits for one.

### Enabling interrupts

`interrupt::enable` is a safe function, and that is the point. The version
deleted in Phase 1 was deleted because its safety contract could only have been
prose; now both of its preconditions are values, and it asserts them:
`table_is_loaded()`, and `pic::is_silenced()`. The third — that locks are
IRQ-safe — is not a runtime state but a property of the code, discharged above.

This is `005`'s rule that an abstraction must make the unsafe operation
underneath it *checkable*, applied to an instruction rather than to a pointer.
The private `set_flag` that both `enable` and `InterruptState::restore` call is
where the difference lives: setting the flag the first time needs an argument,
putting back a flag that was already set does not.

### Which vector carries what

| Vector | Name | Handler |
|---|---|---|
| 0–31 | architectural | breakpoint resumes; everything else panics |
| `0x40` | `PING` | recorded, dismissed, resumes |
| `0xFE` | `ERROR` | panics with the APIC's error-status register |
| `0xFF` | `SPURIOUS` | counted, resumes, **not** dismissed |

The APIC's two vectors are the top two, so device vectors can be handed out
upward from 32 without ever meeting them. `0xFF`'s low four bits are all set
because the earliest APICs hardwire them — `Spurious::enabled` refuses any
other value rather than letting the hardware silently substitute one.

A spurious interrupt is deliberately not dismissed. It is the APIC reporting
that it committed to delivering something and then found it had nothing to say,
so it never set an in-service bit; an end-of-interrupt there would retire a
real interrupt underneath it.

`PING` is how the kernel proves its own interrupt path. It is not a test
fixture — the same self-directed interrupt is how one CPU will ask another to
do work — but its first caller is the self-test, and it is the only way to
observe delivery end to end before a timer exists.

### How it is checked

Host tests (`libs/x86-apic`): the MSR derivation against a transcribed copy of
SDM table 11-1's own MSR column, the mode-transition encoding, the local vector
table's masked value against the architectural reset value, the spurious
register's hardwired low bits, and that no register offset can escape the
`0x800`–`0x8FF` block the architecture reserves.

On the machine, eight new checks:

- both legacy controllers read back fully masked
- the local APIC is software-enabled, and reports an integrated version
- this CPU is accepting interrupts
- a lock clears the flag, a nested lock does not restore it early, and the
  outer release gives it back
- a self-interrupt is delivered
- **a second one is delivered** — which is the end-of-interrupt check, because
  without it the first interrupt is still in service and nothing of the same
  priority class can follow
- the frame the handler saw has the interrupt flag set in the interrupted
  code's `RFLAGS`, which is the only direct evidence it arrived through the
  flag rather than as a software `int`
- no spurious interrupt has been delivered, and the APIC has recorded no error

Both new mechanisms were broken once to confirm the harness goes red:
removing the handler's end-of-interrupt fails the second-delivery check alone,
and removing the lock's `cli` fails the lock check alone.

## Slice 3 — the CPU keeps time

Slice 2 left a kernel that takes interrupts nobody sends. This slice gives it
the first device that sends them on its own: the local APIC's timer, ticking
100 times a second, and an uptime in milliseconds derived from it. Both halves
of that sentence hide a measurement problem, because the clock the timer
counts has no documented rate.

### What owns what

`wost` gains five functions on the APIC module — set the divide, arm the
countdown, start it periodic, read where it stands, read its entry back — and
gains **no new `unsafe`**: every one rides the existing gated MSR helpers,
which is the framekernel split paying rent. The kernel gains
`arch/x86_64/pit.rs`, a driver written entirely outside `wost` because claimed
ports made port I/O safe in Phase 1, and `time.rs`, where calibration policy
and the uptime arithmetic live. The encodings go where encodings go: the
timer's registers, its mode bits and the divide table join `libs/x86-apic`
(`timer`), and the 8254's control word plus the calibration arithmetic are the
new `libs/x86-pit` — all host-tested against the manuals.

`vector.rs` grows `TIMER = 0x41`, beside `PING` rather than down at 32:
vectors `0x20`–`0x2F` are where slice 2 remapped the legacy controllers, and
staying clear of them keeps a leaked legacy interrupt panicking as
"unexpected" instead of masquerading as a tick.

### The only clock we can reach

The APIC timer counts the bus clock divided by a configurable factor, and
nothing reports that clock's rate. CPUID leaf 15H would, on hardware — under
QEMU's software emulation it is absent or zero, and VMware's answers are not
trustworthy either. The modern references — the HPET, and any clock reached
through the I/O APIC's neighbourhood — are memory-mapped device pages, which
is the same wall D-33 hit: no page tables until Phase 3, no MMIO. What
remains is the 8254: port I/O, present in every PC and every hypervisor, its
input crystal fixed at 1.193182 MHz since the original machine derived it
from a colourburst oscillator.

So the timer is measured against the 8254 once, and the 8254 is then
**parked**: a control word halts the count, no initial count ever follows,
and the permanent port claim means nothing can program the chip again.
Counter 0 in mode 0 does the measuring — its gate is always high, its one
output edge lands on a controller slice 2 masked, and mode 0 decrements
monotonically with no reload to reason about inside the window. The classic
alternative, gating counter 2 through the speaker port, drags in port `0x61`
and a second device for zero benefit here.

### Measuring

The window is twenty milliseconds — `CRYSTAL_HZ / 50` crystal ticks. The
bound above is the counter itself: sixteen bits at the crystal rate wrap in
54.9 ms, beyond which a window is indistinguishable from a shorter one. The
bound below is jitter: each latch-and-read is a bus transaction (a VM exit,
under emulation), and the shorter the window the larger those transactions
loom. Twenty milliseconds puts the bracketing reads at parts in ten thousand
while leaving a 35 ms stall margin.

The four reads bracket symmetrically — crystal, APIC, *window*, APIC,
crystal — so the APIC's window nests inside the crystal's and the skew lands
on both ends. Calibration runs between `apic::init` and `interrupt::enable`,
which buys measurement atomicity by ordering alone: nothing can preempt a
kernel that has not enabled interrupts. It also means the timer this arms
delivers its first tick at the `sti`, not before.

The arithmetic is one division, rounded to nearest, from the raw pair — rate
and reload count both, because deriving the count from an already-rounded
rate would compound two floors. The divide configuration never enters the
correctness path at all: calibration measures the *divided* clock and derives
the count from the same clock, so the tick lands on 100 Hz even if the divide
write misfired; the factor exists so the log line can report the undivided
rate. A measurement can still lie — a stall long enough to wrap the crystal
counter mod 65536 corrupts the ratio — so the result must land between one
megahertz and fifty gigahertz, and three attempts are allowed before the
kernel concludes the hardware is lying and boots without a clock. A timer
failure vetoes nothing: `interrupt::enable` still runs, because a kernel
without a clock still wants its keyboard.

QEMU measures at ~997 MHz (it models a gigahertz); VMware has traditionally
advertised tens of megahertz. Both land comfortably inside the bounds, and
the boot log prints what was actually measured.

### A tick that does almost nothing

The handler increments one atomic and dismisses the interrupt. D-31 and D-34
still bind in trap context — no locks, no logging — and a tick has no more to
say than "one more". 100 Hz is deliberately boring: cheap enough to leave
running, decimal enough to read, and explicitly disposable when the tickless
scheduling of D-15 arrives; the vision doc counts wakeups as a feature, and
this is a hundred a second until a scheduler knows better.

One cost is accepted and recorded rather than fixed: a critical section
longer than a tick — a full framebuffer scroll holds the console lock across
a screenful — can swallow a tick, because the APIC holds only one pending.
Uptime then undercounts by ten milliseconds. That is D-34's latency cost
wearing a clock face, and Phase 4's tickless design is the fix.

### Uptime

Whole ticks come from the handler's counter; milliseconds come from
interpolating the hardware countdown inside the current tick — the fraction
`(initial − current) × 10 / initial`, which never exceeds a tick and reaches
one exactly at the countdown's zero, where the value meets what the counter
will read after the increment.

Two races, two defences. A tick can land between reading the counter and
reading the countdown, so the countdown read is sandwiched between two reads
of the tick count and retried when they disagree. The `compiler_fence`s
around that read are load-bearing: `rdmsr` is `options(nomem)` assembly, and
without the fences the compiler may float the atomic loads across it and
unmake the sandwich. The second race the sandwich cannot see: code holding
the interrupt flag clear — any lock — can watch the countdown reload while
the tick it announces stays pending, and a second read in the same section
would see time step back. Every reading therefore passes through a high-water
clamp, monotonic against everything already reported. Breaking the handler's
end-of-interrupt during testing produced exactly that state for seconds at a
stretch — countdown reloading, tick counter frozen — and the clamp held.

### The idle loop

`kernel_main` used to end in `halt_forever`. It now ends in the idle loop:
halt, wake, do the work the interrupt brought, halt again. Today the work is
nothing; slice 4's keyboard drain is written into the loop body, and a kernel
whose interrupts declined halts once and stays halted, exactly as before.

### How it is checked

Host tests: the three timer registers against both columns of SDM
table 11-1, the periodic mode bit against the LVT figure, all eight divide
encodings against the SDM's scattered-bits table, the interpolation's
monotonicity and its boundary behaviour, the 8254 control word against the
datasheet (including `0xB6`, the byte forty years of speaker code agree on),
wrap-surviving tick deltas, and the calibration arithmetic's rounding, edges
and overflow refusals — with fixtures shaped like both QEMU's gigahertz and
VMware's sixty-six megahertz.

On the machine, seven new checks:

- the timer's entry reads back periodic, unmasked, and on our vector
- the calibrated rate lands inside bounds restated in the test
- the tick counter advances
- **a second tick arrives** — the end-of-interrupt proof, same argument as
  the ping pair
- uptime never runs backwards, swept continuously across three ticks with one
  read taken under a lock — the clamp's reason to exist
- at least one reading is not a whole tick, so interpolation is alive
- uptime and the tick counter agree, once twenty ticks have settled

Every one was broken once and watched go red. One break earned its keep
twice over: a clock deliberately run at eleven milliseconds per tick sailed
*past* the agreement check, because a fresh boot had not accumulated enough
error to escape the check's one-tick slack. The check now waits out twenty
ticks before judging — and the incident is why the rule says to break the
check, not just write it.

## Slice 4 — the keyboard

The phase's last slice is its first input device, and the moment the machine
stops being write-only: press a key at the console and the glyph appears.
Under that one visible behaviour sit three separate jobs — getting bytes out
of a 1980s controller, decoding a protocol with five kinds of sequence, and
proving a real keystroke crosses the whole path — and each landed where its
nature puts it.

### What owns what

`wost` gains **nothing**. The i8042 speaks entirely through two I/O ports,
ports have been safe since Phase 1's claim registry, and so the driver —
`kernel/arch/x86_64/i8042.rs` — is the first written wholly outside the
privileged crate: the framekernel split collecting what slice 0 paid for.
Zero new `unsafe` in this slice, as in the last.

The protocol is arithmetic: `libs/ps2-keys` holds the set 2 tables, the
byte-at-a-time decoder, shift tracking, and the US layout that turns a key
identity into a character — layout deliberately a sibling of the decoder,
not part of it, so a second layout never touches the state machine. The
kernel's policy lives in `input.rs`: a module-local singleton (not `dev.rs`,
whose charter is the output devices the panic path must reach — a panicking
kernel has no use for its keyboard), a batched drain, and the echo.

### Polled, because the only interrupt path is walled off

IRQ 1 has exactly two routes to this CPU. One is the legacy 8259 pair, and
slice 2's whole point was that they are silenced — `interrupt::enable`
asserts it, a self-test pins their masks, and reopening one line would
reintroduce the mixed-controller state that slice removed, for code the I/O
APIC would obsolete. The other is the I/O APIC, which is a memory-mapped
device page — the same wall D-33 hit, standing until Phase 3 owns page
tables.

So the controller is told to raise nothing at all — its config byte's two
interrupt enables are cleared — and slice 3's idle loop does the work: every
tick wakes the `hlt`, and the loop drains every pending byte before halting
again. Worst-case latency is one tick, ten milliseconds, a third of the way
to the threshold where a keypress starts feeling detached; the keyboard's
own buffering holds an order of magnitude more than a tick of typing, so
nothing is lost. When Phase 3 pages the I/O APIC in, the polled drain
retires and the driver and decoder under it carry over unchanged. See D-37.

### Bringing up the 8042

The standard sequence, every wait bounded in the 16550 driver's idiom, and
each failure a typed decline — a kernel without a keyboard is worth more
than one that hung bringing it up. Disable both ports, flush, then write the
configuration: interrupts off, **translation off** — the controller boots
translating the keyboard's set 2 into set 1 for the sake of a 1980s BIOS,
and QEMU really does boot with that shim on, so clearing bit 6 is what makes
the decoder's input raw. The controller self-test follows, and then the
configuration is **written again**: real controllers may reset it to
power-on defaults during the test, a quirk worth two lines now rather than
an afternoon later. Interface test, port enable, keyboard reset (the basic
assurance test is allowed hundreds of milliseconds, and emulators disagree
on whether an acknowledgement precedes it, so either shape is accepted),
set 2 selected — a refusal here is tolerated, because set 2 is the power-on
default and the self-test's readback is the arbiter — and scanning on.

### Set 2, byte by byte

The decoder is a five-state machine fed one byte at a time, and it is
*total*: every byte in every state has a defined transition, and garbage
costs at most the sequence it corrupted. The states exist because the
protocol has five shapes — bare makes, `F0`-prefixed breaks, `E0` extended
pairs, PrintScreen's sequences wrapped in *fake shifts* (`E0 12` and
`E0 59`, shifts no shift key sent, kept so 1980s BIOSes could track state;
discarded here), and Pause, eight bytes with no break code at all. Protocol
replies — acknowledgements, echoes, self-test results — can interleave with
any of them and are swallowed without disturbing what is in flight. One
table quirk earns a note: F7 sits at `0x83`, above `0x7F`, which is *why*
set 2 breaks need a prefix byte rather than a high bit.

A host test feeds all 256 bytes through every reachable prefix state and
proves nothing panics or wedges; the sequences are driven byte-at-a-time
with silence asserted at each intermediate step, which is the exact shape of
the driver's loop.

### From key event to glyph

The idle loop's drain batches under the keyboard lock — eight events per
hold, so a stream of input cannot pin the lock — and resolves each event's
character against the shift state *as it stood at that event*, then delivers
outside the lock: a `debug!` per event (compiled but discarded at the
default level; the self-test asserts arrival programmatically for exactly
that reason), and the echo through the console lock, which is mainline
context, where taking it is the sanctioned nesting. Enter echoes a newline.
Backspace deliberately echoes nothing: the console has no cursor-retreat
primitive, and building one for a single caller is Phase 7's shell work,
not this slice's.

### The harness can press a key

D-28's machinery can only observe what the machine does to itself; input
needs an outside hand. The hand is QMP, QEMU's JSON control socket, spoken
by a hand-rolled client in `xtask/src/qmp.rs` — three fixed exchanges, no
JSON dependency, on the same argument as the hand-rolled SHA-256. The
harness binds a loopback listener before the spawn and QEMU dials out to
it, so there is no port to guess and no connect to race.

The injection is gated on a marker, not sprayed from power-on: early
keys would land in firmware setup prompts or the bootloader's menu, and the
self-test's own controller commands must never interleave with injected
bytes. The kernel prints one line — `selftest: awaiting injected
keystroke`, held as a matching constant in `qemu.rs` the way the exit codes
already are — and the harness, whose serial now flows through a forwarding
thread that keeps every line visible, answers it with exactly one press and
one release of `A`, explicit down and up rather than the monitor's
`sendkey`, whose automatic release fires on QEMU's schedule.

The verdict never leaves the kernel. A QMP failure of any kind prints its
cause and retires the injector; the check then fails on its own doubly
bounded wait — a tick budget for the honest case, an iteration cap for a
dead timer — and the sixty-second kill stands behind everything. See D-38.

### How it is checked

Host tests: the two lookup tables against transcribed set 2 anchors and an
injectivity sweep; every sequence shape byte-at-a-time; recovery from
corruption in every prefix state; the 256-byte totality sweep; shift's
"released one, still holding the other" case; the US layout's rows; and the
QMP client's command strings and reply classification, including a real
`SHUTDOWN` event line — which will genuinely arrive on that socket when the
exit device fires, and must be skipped, not misread as a verdict.

On the machine, five new checks, reaching 34:

- the controller came up (its decline path names a reason and boots on)
- its configuration byte, read back **live**, has both interrupt enables
  and the translation shim off — the check that would catch the
  after-self-test rewrite going missing
- an echo command round-trips to the keyboard and back
- the keyboard reports itself speaking set 2 — under translation this reads
  back translated, so it cross-checks the config bit too
- **an injected keystroke arrives, and its release follows** — one real key
  crossing a process boundary, a socket, the emulated bus, the controller,
  and the decoder, in order

Every one was broken once: a wrong self-test expectation declines the whole
driver; leaving translation on fails the config check and corrupts the set
readback with it; a wrong echo byte fails alone; a keyboard deliberately
put in set 3 fails alone; injecting `b` instead of `a` fails the keystroke
check after its budget; and deleting the QMP channel entirely goes red
rather than hanging, which is the injector's failure contract holding.
