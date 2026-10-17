# 000 - Vision

**Ownership after D-366 (2026-10-07).** Bedrock owns the kernel mechanisms
and ABI described here. Product userland, graphics policy, compatibility ports
and application delivery belong to [Realm](https://github.com/wizards-ecosystem/wizards-realm).
The original phase numbers and dated observations remain historical; they do
not create a second current product backlog in Bedrock.

The Wizard's Bedrock is a from-scratch Rust kernel, built as a multi-year
design-first project. The original combined OS vision below now spans Bedrock
and Realm. Kernel mechanisms stay here; Realm owns the operating environment,
compatibility ports and application experience.

## Destination

The inherited system destination, now owned jointly with Realm, is an OS that can:

- run **Python**: RustPython as an early compatibility proof, then a real
  CPython port; neither is the native first-party userland (D-183)
- render the web: ported `links` (text) first, then **NetSurf** (graphical).
  NetSurf renders documents, not applications: HTML 4.01 and CSS 2.1, with
  JavaScript disabled by default and 1,539 bindings unimplemented upstream.
  The milestone proves the libc, the window system, the network stack and TLS
  work together against a real third-party codebase, which is worth having,
  and is not the same claim as browsing the 2026 web (D-75)
- run its own **compositor + window system** (Wayland-inspired protocol,
  possibly more custom) with a native widget toolkit
- store files on **WizFS**, our own filesystem, behind a VFS layer, reached
  late and deliberately; the phases before it run on a read-only third-party
  format (D-73)

## Core decisions

| Area | Decision |
|---|---|
| Kernel language | Rust (stable channel while possible); it does not move to Wizzy (D-183) |
| First-party target userland | Every feasible non-kernel component is Wizzy once the compiler-readiness gate opens; product work waits there. Narrow runtime shims, host tooling, and third-party libraries, ports and compatibility are exceptions (D-183) |
| Architecture | x86_64 first; `arch/` modules in both `wost` and `kernel`, reached through `arch::current`, isolate arch code for future ports |
| Boot | UEFI + Limine (vendored); an own bootloader is not planned without a concrete unmet handoff requirement (D-219) |
| Kernel shape | Monolithic, modular |
| ABI | **Capability handles + async submission/completion queues** (native); POSIX only as an mlibc translation layer for ports; see `002-architecture.md` |
| Process model | `spawn()` only, with no native `fork()` (compat-fork gated for the POSIX layer) |
| Filesystem | VFS first, then a read-only third-party format; WizFS = **CoW B-tree, checksummed, snapshots** in a phase of its own, late (D-73) |
| App formats | ELF64 native + **core Wasm/`wasip1`** first-class; the Component Model is a watch item. An artifact format does not choose the first-party source language (D-72, D-183) |
| Security | W^X, NX, SMEP/SMAP/UMIP live; **every stack guarded**: thread (D-52), double-fault (D-89) and boot (D-92); KASLR deferred to Phase 12 with its cost named (D-46) |
| Scheduler | EEVDF-inspired virtual deadlines, tickless |
| Theme | Wizard/magic at product level (app names, shell personality); conventional naming in code |
| Dev loop | QEMU (UEFI, serial stdio) minute-to-minute; VMware Workstation as milestone target |
| Build | Fully self-contained on Windows: rustup (GNU host) + QEMU only; disk imaging in pure Rust (`xtask`) |

## Method

Design-first: every subsystem gets a doc in `docs/` before implementation,
citing references (OSDev wiki, Osdev-Notes, Intel SDM, os.phil-opp.com as
reference, not followed linearly). Docs are numbered in creation order.

## Roadmap (phases)

0. **Forge**: toolchain, workspace, boot to first light [done] (2026-07-18)
1. First light polish: logging, panic handling, framebuffer console;
   framekernel split into `wost` + `kernel` [done] (2026-08-12)
2. CPU fundamentals: GDT, IDT, exceptions, APIC timer, keyboard;
   security baseline on (NX, W^X, SMEP/SMAP/UMIP) [done] (2026-08-20). *Descriptor
   tables, the fault path and NX/SMEP/SMAP/UMIP 2026-08-13; the x2APIC,
   IRQ-safe locking and interrupts on 2026-08-14; the calibrated 100 Hz tick,
   uptime, and the polled set 2 keyboard 2026-08-20; W^X follows the page
   tables into Phase 3 (D-32); slice table in `006-cpu-fundamentals.md`*
3. **Memory**: physical frame allocator, paging, kernel heap
   [done] implementation (2026-08-22; the dated Phase-3 VMware result was not
   recorded). *The grown boot handoff and the frame allocator; the
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
4. Multitasking: kernel threads, EEVDF-style scheduler, tickless idle
   [done] (2026-08-23). *Threads and the naked switch, the EEVDF seam over
   `libs/eevdf` 2026-08-22; tickless (the counter is the clock, the timer an
   alarm), sleeping, blocking on a wait queue, the idle thread, and D-34's
   latency debt paid 2026-08-23; closed by a settled VMware boot and the
   D-76 soundness audit. Slice table in `009-multitasking.md`*
5. **Userspace**: ring 3, **handle table + rights**, spawn, ELF loading,
   channels, **async queue ABI**, initrd
   [done] (2026-08-24). *The lock discipline moved into the borrow checker
   (D-85); an address space that grows after activation and carries evidence
   for every mapping (D-86 to D-88); trap entry rebuilt FRED-shaped over
   `libs/trap-cause`; ring 3 through one interrupt gate at DPL 3, with the
   SMAP copy guard and `VERW` on the way out; the handle table over
   `libs/handle-space` with rights in one word; **a table of its own for every
   program**, installed by the same code that moves `RSP0` and the extended
   state (D-106, D-108); D-44's untyped split with its first real caller
   (D-107); channels carrying bytes and handles, attenuating at both ends
   (D-110); a program born holding exactly one thing: a way to be spoken to
   (D-111); and the queue protocol **model-checked before the kernel
   implemented it** (D-112, D-113). Closed by a dated VMware boot and the
   D-115 audit. 124 machine checks at the close, and 343 lines of `unsafe` in
   `wost` against a budget of 380; both have moved since, and
   `xtask/src/unsafety.rs` holds the current one because a number in prose
   cannot be checked. The mlibc sysdep is written and analysed rather than
   compiled: the toolchain wall is measured in D-114. D-181 found that its
   three-bin summary conflated C++ body state with remaining platform work and
   replaced it with an exact two-axis row for each inspected function. The
   register is in `fixtures/abi-pressure/README.md`; the gate discovers active definitions,
   checks BODY evidence in comment-free code and selected disputed REMAINDER
   labels against kernel witnesses. Slice table in
   `011-userspace.md`*
6. VFS + storage: **planned slices landed 2026-08-25.** D-184's authoritative
   finite foundation and current phase-close status is maintained in
   `016-pre-wizzy-foundation.md`. *The ABI half first (D-149): a
   completion carries a handle, the clock became a page rather than a call, and
   a dead program's work is finished rather than forgotten. Then PCI with a
   device that is claimed rather than a port range (D-77's other half), a
   polled NVMe driver, FAT32 read-only checked against `fatfs` as an oracle, a
   VFS over directories whose resolver has no way up, tmpfs and devfs, and a
   `read` that is a submission. The current check and mutation counts live in
   `014-storage.md`'s checked closing table rather than being copied here. The
   write path, a page cache, symlinks and namespace
   composition are carried deliberately; WizFS is still not here (D-73). Slice
   table in `014-storage.md`*
7. Userland: **Wizzy-first** (D-183): measure and build only the boundary,
   runtime and platform preparation needed to open the compiler-readiness gate
   in `015-wizzy-userland.md`; product `init`, TTY, shell and tmpfs-backed tools
   wait at that gate and are then written in Wizzy. Core Wasm + `wasip1`
   remains a first-class app format (D-72). AOT is not assumed. The userland
   itself is designed in `019-userland.md` and `020-style-guide.md` (D-314)
8. Network: Rust kernel-side NIC and async-socket work, third-party `smoltcp`
   and `rustls`, and a Wizzy-first capability-native client/service (D-183)
9. POSIX gateway and compatibility runtimes: mlibc grown from Phase 5's
   pressure test only for selected ports, compat-fork and cross-toolchain;
   **RustPython, links and CPython milestones** remain compatibility proofs,
   retain their upstream languages rather than becoming native userland, and
   keep *CPython's test suite passing* as the definition of done (D-183)
10. Pixels: GPU drivers; the first-party target-side compositor (channels +
    shared memory objects), GPU-first toolkit and terminal app are Wizzy-first
    (D-183)
11. Document web: **NetSurf milestone** (D-75): a framebuffer frontend, and
    the integration proof that libc, windowing, networking and TLS hold
    together under a real third-party codebase
12. Side-quests: SMP, USB, sound, CET shadow stacks,
    signed/measured boot, real hardware boot
13. **WizFS**: the CoW B-tree, in a phase of its own, with crash injection
    and a repair tool in its definition of done (D-73). User-facing repair and
    administration tools are Wizzy-first (D-183). Last because the schedule
    evidence says so, not because it matters least: bcachefs took ten years and
    ten months, and every phase above it can run on Phase 6's read-only format
    meanwhile

The established phase identities remain intact: network is still Phase 8,
POSIX Phase 9, pixels Phase 10, document web Phase 11, side-quests Phase 12,
and WizFS Phase 13. Earlier ADRs that defer by number still name the work they
were written to name. D-183 changes their first-party language and readiness
gate, not their phase numbers.
