# D-32 The CPU protection baseline goes on at boot

Status: Accepted, 2026-08-13.

**Decision.** After the interrupt table is installed, the kernel enables
each of SMEP, SMAP, UMIP and NX that `CPUID` reports (D-12), reads the
control registers back and logs what is on. Enabling is a safe call: none of
these bits can cause undefined behaviour, only a trap, and by then a trap is
reportable.

**Rationale.** Ordering is the design. Enabling SMEP and SMAP against the
bootloader's page tables is a risk: had Limine mapped the kernel or the
framebuffer user-accessible, the next fetch or pixel would fault. Installing
the IDT first turns a silent reset into a page fault with an address.

**Consequences.**

- **QEMU is `-cpu max`, not `-cpu qemu64`.** The default model exposes none
  of SMEP, SMAP or UMIP, so the kernel would enable its baseline on a CPU
  that cannot enforce it and the check would pass vacuously. VMware and the
  real machine have all four.
- W^X, the other half of D-12, is a property of mappings and waits for Phase
  3. Enabling NX now makes it enforceable then.
- `AC`, the flag that lifts SMAP for an intended user access, gets no
  accessor until the checked user copy it exists for.
