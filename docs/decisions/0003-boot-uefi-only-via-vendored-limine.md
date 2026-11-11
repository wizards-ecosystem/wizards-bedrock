# D-03 Boot: UEFI-only via vendored Limine

Status: Accepted, 2026-07-18. Amended by D-219.

**Decision.** Limine (v10 binary, vendored in `boot/limine/` with license
and provenance note), UEFI-only. Writing our own bootloader is a deferred
side quest (roadmap Phase 12).

**Amended by D-219.** Limine remains the boot boundary. The own-bootloader
side quest is retired, not deferred. Only a concrete handoff defect that the
maintained dependency cannot satisfy can reopen that choice.

**Rationale.** Limine enters in 64-bit long mode with a higher-half load,
memory map, framebuffer and modules, identically across QEMU/EDK2, VMware
EFI and real UEFI hardware. A half-working homemade loader would make every
early kernel bug ambiguous between loader and kernel.

**Consequences.** UEFI-only disk layout (no BIOS fallback); dependency on
Limine's boot protocol (base revision 4 since D-42); `limine.conf` and
`BOOTX64.EFI` are repo artifacts.
