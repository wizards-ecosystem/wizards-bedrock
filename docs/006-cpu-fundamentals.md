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
TSS); OSDev wiki GDT/IDT/APIC; Osdev-Notes' interrupt chapter.

## Order of work

Each slice boots, is checked on the machine, and leaves the tree green. The
order is forced by dependency: nothing can take an interrupt until a fault can
be reported, and nothing should enable interrupts until the legacy controller
is silenced.

| Slice | Contents | Status |
|---|---|---|
| 1 | GDT + TSS, IDT, exception entry and reporting, SMEP/SMAP/UMIP/NX | **landed** |
| 2 | Legacy PIC silenced, local APIC, IRQ-safe locking, `enable_interrupts` | next |
| 3 | APIC timer: calibration, periodic tick, uptime | |
| 4 | PS/2 keyboard: scancode set 2 decoding, key events | |

Slice 1 is the subject of the rest of this document; slices 2–4 get their
design recorded here as they are built, so that this file ends the phase as
the description of what exists.

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

So `cpu::enable_interrupts` is still not written, for the same reason it was
deleted in Phase 1: its safety contract has to say what the caller must have
already arranged, and until the PIC is silenced and `SpinLock` is IRQ-safe,
that contract would be a guess. It arrives in slice 2, with its first caller.

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
