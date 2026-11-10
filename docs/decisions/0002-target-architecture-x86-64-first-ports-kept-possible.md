# D-02 Target architecture: x86_64 first, ports kept possible

Status: Accepted, 2026-07-18.

**Context.** The host is an i7-13700KF (x86_64, UEFI); testing runs in
VMware/QEMU on it.

**Options.** x86_64; ARM64/Raspberry Pi; RISC-V; multi-arch from day one.

**Decision.** x86_64 only for now, with all arch-specific code isolated
under `kernel .../arch/x86_64/` so ARM64/RISC-V ports stay feasible without
a rewrite.

**Rationale.** Best documentation and tooling; boots on the actual machine;
QEMU and VMware both first-class. RISC-V/ARM are cleaner ISAs with thinner
step-by-step resources; the arch-isolation layer keeps the option.

**Consequences.** We carry x86 legacy (GDT, APIC quirks); the `arch/`
boundary is a standing discipline every phase must respect.
