# 000 — Vision

The Wizard's OS is a from-scratch operating system, built as a multi-year
design-first project. We stand on a small set of lean open-source components
(bootloader, TCP/IP stack, libc-for-porting) and design everything else
ourselves from primary references.

## Destination

A full OS with userspace that can, eventually:

- run **Python** — RustPython early, a real CPython port later
- browse the web — ported `links` (text) first, then **NetSurf** (graphical)
- run its own **compositor + window system** (Wayland-inspired protocol,
  possibly more custom) with a native widget toolkit
- store files on **WizFS**, our own filesystem, behind a VFS layer

## Core decisions

| Area | Decision |
|---|---|
| Kernel language | Rust (stable channel while possible) |
| Architecture | x86_64 first; `kernel/src/arch/` isolates arch code for future ports |
| Boot | UEFI + Limine (vendored); own bootloader is a future side-quest |
| Kernel shape | Monolithic, modular |
| ABI | **Capability handles + async submission/completion queues** (native); POSIX only as an mlibc translation layer for ports — see `002-architecture.md` |
| Process model | `spawn()` only — no native `fork()` (compat-fork gated for the POSIX layer) |
| Filesystem | VFS first; WizFS = **CoW B-tree, checksummed, snapshots** (see `002-architecture.md`) |
| App formats | ELF64 native + **WASI Component Model** first-class (runtime mid-project) |
| Security | W^X, NX, SMEP/SMAP, KASLR, guard pages as baseline, not retrofit |
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
   security baseline on (NX, W^X, SMEP/SMAP/UMIP)
3. Memory — physical frame allocator, paging, kernel heap, memory objects
4. Multitasking — kernel threads, EEVDF-style scheduler, tickless idle
5. Userspace — ring 3, **handle table + rights**, spawn, ELF loading,
   channels, **async queue ABI**, initrd
6. VFS + WizFS — VFS over handles, tmpfs/devfs; **WizFS CoW B-tree**
   (checksums, snapshots) + virtio-blk/AHCI
7. Userland — shell, coreutils, TTY; **RustPython milestone**;
   **WASI runtime** (interpreter-class) — first sandboxed component apps
8. Network — NIC drivers, smoltcp, async socket handles, TLS (rustls)
9. POSIX gateway — mlibc over the translation layer, compat-fork,
   cross-toolchain; **links + CPython milestones**
10. Pixels — GPU drivers, compositor (channels + shared memory objects),
    GPU-first toolkit, terminal app
11. Graphical web — **NetSurf milestone**
12. Side-quests — own bootloader, SMP, USB, sound, CET shadow stacks,
    signed/measured boot, real hardware boot
