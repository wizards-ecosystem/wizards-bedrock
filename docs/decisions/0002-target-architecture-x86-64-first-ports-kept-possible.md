# D-02 Target architecture: x86_64 first, ports kept possible

Status: Accepted, 2026-07-18.

**Decision.** x86_64 only for now, with all arch-specific code isolated
under `kernel .../arch/x86_64/` so ARM64/RISC-V ports stay feasible without
a rewrite.

**Rationale.** x86_64 has the best documentation and tooling and boots on
the host machine and both VMs.

**Consequences.** The `arch/` boundary is a standing discipline every phase
must respect.
