# D-32 The CPU protection baseline goes on at boot

Status: Accepted (2026-08-13)

**Context.** D-12 requires SMEP, SMAP, UMIP and NX from day one rather than as a
retrofit. Phase 2 is the first phase with a CPU to configure.

**Decision.** After the interrupt table is installed, the kernel enables every
one of the four that `CPUID` reports, then reads the control registers back and
logs what is actually on. Enabling is a safe call: none of these bits can
produce undefined behaviour, only a trap, and by then a trap is reportable.

**Rationale.** Ordering is the whole design. Enabling SMEP and SMAP against page
tables the bootloader built is a real risk — if Limine had mapped the kernel or
the framebuffer user-accessible, the next instruction fetch or the next pixel
would fault. Installing the IDT first turns that from a silent reset into a page
fault with an address. (It does not: verified 2026-08-13, all four enabled and
the framebuffer console still draws.)

**Consequences.**

- **QEMU is now `-cpu max`, not `-cpu qemu64`.** The default model exposes none
  of SMEP, SMAP or UMIP, so the kernel would have switched its baseline on
  against a CPU that cannot enforce it and the check that it did would have
  passed vacuously — which is exactly what the first run showed
  (`protections nx`). The VMware target and the real machine have all four.
- W^X, the other half of D-12, is a property of mappings and waits for Phase 3.
  Enabling NX now is what makes it enforceable then.
- `AC`, the flag that lifts SMAP for a deliberate user access, gets no accessor.
  It arrives with the checked user copy it exists for, not before.
