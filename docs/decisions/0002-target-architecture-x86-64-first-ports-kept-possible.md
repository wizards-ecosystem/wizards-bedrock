# D-02 Target architecture: x86_64 first, ports kept possible

Status: Accepted

**Context.** Host machine is an i7-13700KF (x86_64, UEFI) and testing
happens in VMware/QEMU on it.

**Options.** x86_64; ARM64/Raspberry Pi; RISC-V; multi-arch from day one.

**Decision.** x86_64 exclusively for now, with all arch-specific code
isolated under `kernel .../arch/x86_64/` so ARM64/RISC-V ports remain
feasible without a rewrite.

**Rationale.** Best documentation and tooling; boots on the actual
machine; QEMU + VMware both first-class. RISC-V/ARM are cleaner ISAs but
thinner step-by-step resources; the arch-isolation layer preserves the
option.

**Consequences.** We carry x86 legacy (GDT, APIC quirks); the `arch/`
boundary is a standing discipline every phase must respect.
