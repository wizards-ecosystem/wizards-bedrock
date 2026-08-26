# 000 — Vision

The Wizard's OS is a from-scratch operating system, built as a multi-year
design-first project. We stand on a small set of lean open-source components
(bootloader, TCP/IP stack, libc-for-porting) and design everything else
ourselves from primary references.

## Destination

A full OS with userspace that can, eventually:

- run **Python** — RustPython early, a real CPython port later
- render the web — ported `links` (text) first, then **NetSurf** (graphical).
  NetSurf renders documents, not applications: HTML 4.01 and CSS 2.1, with
  JavaScript disabled by default and 1,539 bindings unimplemented upstream.
  The milestone proves the libc, the window system, the network stack and TLS
  work together against a real third-party codebase — which is worth having,
  and is not the same claim as browsing the 2026 web (D-75)
- run its own **compositor + window system** (Wayland-inspired protocol,
  possibly more custom) with a native widget toolkit
- store files on **WizFS**, our own filesystem, behind a VFS layer — reached
  late and deliberately; the phases before it run on a read-only third-party
  format (D-73)

## Core decisions

| Area | Decision |
|---|---|
| Kernel language | Rust (stable channel while possible) |
| Architecture | x86_64 first; `arch/` modules in both `wost` and `kernel`, reached through `arch::current`, isolate arch code for future ports |
| Boot | UEFI + Limine (vendored); own bootloader is a future side-quest |
| Kernel shape | Monolithic, modular |
| ABI | **Capability handles + async submission/completion queues** (native); POSIX only as an mlibc translation layer for ports — see `002-architecture.md` |
| Process model | `spawn()` only — no native `fork()` (compat-fork gated for the POSIX layer) |
| Filesystem | VFS first, then a read-only third-party format; WizFS = **CoW B-tree, checksummed, snapshots** in a phase of its own, late (D-73) |
| App formats | ELF64 native + **core Wasm/`wasip1`** first-class; the Component Model is a watch item (D-72) |
| Security | W^X, NX, SMEP/SMAP/UMIP live; **every stack guarded** — thread (D-52), double-fault (D-89) and boot (D-92); KASLR deferred to Phase 12 with its cost named (D-46) |
| Scheduler | EEVDF-inspired virtual deadlines, tickless |
| Theme | Wizard/magic at product level (app names, shell personality); conventional naming in code |
| Dev loop | QEMU (UEFI, serial stdio) minute-to-minute; VMware Workstation as milestone target |
| Build | Fully self-contained on Windows: rustup (GNU host) + QEMU only; disk imaging in pure Rust (`xtask`) |

## Method

Design-first: every subsystem gets a doc in `docs/` before implementation,
citing references (OSDev wiki, Osdev-Notes, Intel SDM, os.phil-opp.com as
reference — not followed linearly). Docs are numbered in creation order.

## Roadmap (phases)

0. **Forge** — toolchain, workspace, boot to first light ✅ (2026-07-18)
1. First light polish — logging, panic handling, framebuffer console;
   framekernel split into `wost` + `kernel` ✅ (2026-08-12)
2. CPU fundamentals — GDT, IDT, exceptions, APIC timer, keyboard;
   security baseline on (NX, W^X, SMEP/SMAP/UMIP) ✅ (2026-08-20) — *descriptor
   tables, the fault path and NX/SMEP/SMAP/UMIP 2026-08-13; the x2APIC,
   IRQ-safe locking and interrupts on 2026-08-14; the calibrated 100 Hz tick,
   uptime, and the polled set 2 keyboard 2026-08-20; W^X follows the page
   tables into Phase 3 (D-32); slice table in `006-cpu-fundamentals.md`*
3. **Memory** — physical frame allocator, paging, kernel heap
   ✅ implementation (2026-08-22; the dated Phase-3 VMware result was not
   recorded) — *the grown boot handoff and the frame allocator; the
   kernel's own W^X-correct tables and a `CR3` switch that refuses to go
   blind; the heap, in the direct map because mapping was then one-way
   (D-86 later lifted that, and the heap stayed where it was on its own
   merits); the
   console back buffer; the I/O APIC, which retires the polled keyboard
   (D-37) and takes down D-33's wall; write-combining. KASLR is deferred to
   Phase 12 with its cost named (D-46). Guarding the boot and double-fault
   stacks and moving to growable stacks were left to Phase 5 and done there
   (D-89, D-92); memory objects moved there too, with D-44's untyped
   decision.
   Slice table in `007-memory.md`*
4. Multitasking — kernel threads, EEVDF-style scheduler, tickless idle
   ✅ (2026-08-23) — *threads and the naked switch, the EEVDF seam over
   `libs/eevdf` 2026-08-22; tickless (the counter is the clock, the timer an
   alarm), sleeping, blocking on a wait queue, the idle thread, and D-34's
   latency debt paid 2026-08-23; closed by a settled VMware boot and the
   D-76 soundness audit. Slice table in `009-multitasking.md`*
5. **Userspace** — ring 3, **handle table + rights**, spawn, ELF loading,
   channels, **async queue ABI**, initrd
   ✅ (2026-08-24) — *the lock discipline moved into the borrow checker
   (D-85); an address space that grows after activation and carries evidence
   for every mapping (D-86 to D-88); trap entry rebuilt FRED-shaped over
   `libs/trap-cause`; ring 3 through one interrupt gate at DPL 3, with the
   SMAP copy guard and `VERW` on the way out; the handle table over
   `libs/handle-space` with rights in one word; **a table of its own for every
   program**, installed by the same code that moves `RSP0` and the extended
   state (D-106, D-108); D-44's untyped split with its first real caller
   (D-107); channels carrying bytes and handles, attenuating at both ends
   (D-110); a program born holding exactly one thing — a way to be spoken to
   (D-111); and the queue protocol **model-checked before the kernel
   implemented it** (D-112, D-113). Closed by a dated VMware boot and the
   D-115 audit. 124 machine checks at the close, and 343 lines of `unsafe` in
   `wost` against a budget of 380 — both have moved since, and
   `xtask/src/unsafety.rs` holds the current one because a number in prose
   cannot be checked. The mlibc sysdep is written and analysed rather than
   compiled: the toolchain wall is measured in D-114, and the finding — a few
   tags map, most need a ring-3 door to a mechanism the kernel already has,
   and one needs a namespace this design does not have — is what D-74 asked
   the slice for. The counts are in `userland/README.md`, where the gate
   checks them against the sysdeps' own tags. Slice table in
   `011-userspace.md`*
6. VFS + storage — **done 2026-08-25.** *The ABI half first (D-149): a
   completion carries a handle, the clock became a page rather than a call, and
   a dead program's work is finished rather than forgotten. Then PCI with a
   device that is claimed rather than a port range — D-77's other half — a
   polled NVMe driver, FAT32 read-only checked against `fatfs` as an oracle, a
   VFS over directories whose resolver has no way up, tmpfs and devfs, and a
   `read` that is a submission. 180 machine checks, 65 mutations, four new
   `libs/` crates. The write path, a page cache, symlinks and namespace
   composition are carried deliberately; WizFS is still not here (D-73). Slice
   table in `014-storage.md`*
7. Userland — shell, coreutils, TTY; **RustPython milestone**;
   **Wasm runtime** (interpreter-class, `wasip1`) — first sandboxed apps
8. Network — NIC drivers, smoltcp, async socket handles, TLS (rustls)
9. POSIX gateway — mlibc grown from Phase 5's stub to a real sysdep set,
   compat-fork, cross-toolchain; **links + CPython milestones**, with
   *CPython's test suite passing* as the definition of done
10. Pixels — GPU drivers, compositor (channels + shared memory objects),
    GPU-first toolkit, terminal app
11. Document web — **NetSurf milestone** (D-75): a framebuffer frontend, and
    the integration proof that libc, windowing, networking and TLS hold
    together under a real third-party codebase
12. Side-quests — own bootloader, SMP, USB, sound, CET shadow stacks,
    signed/measured boot, real hardware boot
13. **WizFS** — the CoW B-tree, in a phase of its own, with crash injection
    and a repair tool in its definition of done (D-73). Last because the
    schedule evidence says so, not because it matters least: bcachefs took ten
    years and ten months, and every phase above it can run on Phase 6's
    read-only format meanwhile

Phase numbers 0–12 are unchanged, so earlier ADRs that defer to "Phase 12"
(KASLR in D-46, SMP in D-70, CET) still mean the side-quests phase.
