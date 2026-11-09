# D-03 Boot: UEFI-only via vendored Limine

Status: Accepted, 2026-07-18. Amended by D-219.

**Context.** Choice between writing a bootloader (BIOS or UEFI) and
reusing a lean open-source one. "Standing on the shoulders of clean open
source projects" is an explicit project value.

**Options.** (a) Limine now, own later; (b) Limine forever; (c) own UEFI
loader first; (d) classic BIOS boot-sector journey.

**Decision.** Limine (v10 binary, vendored in `boot/limine/` with license
and provenance note), UEFI-only. Writing our own bootloader is a
deliberately deferred side-quest (roadmap Phase 12).

**Amended by D-219.** Limine remains the boot boundary. The own-bootloader
side quest is retired rather than deferred; only a concrete handoff defect
which the maintained dependency cannot satisfy can reopen that choice.

**Rationale.** Limine delivers exactly what a kernel wants: entry in
64-bit long mode, higher-half load, memory map, framebuffer, modules —
identical across QEMU/EDK2, VMware EFI, and real UEFI hardware. BIOS real
mode is historical archaeology on a 2026 machine. A half-working homemade
loader would poison early kernel debugging ("is the bug in the loader or
the kernel?"). Deferring preserves the learning goal at the point where
we'll know precisely what a loader must provide.

**Consequences.** UEFI-only disk layout (no BIOS fallback); dependency on
Limine's boot protocol (base revision 4 since D-42); `limine.conf` +
`BOOTX64.EFI` are repo artifacts.
