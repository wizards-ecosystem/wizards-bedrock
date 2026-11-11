# D-04 Kernel shape: monolithic-modular

Status: Verified, 2026-07-18. As refined by D-13.

**Decision.** Monolithic-modular: drivers, filesystems and the network stack
live in the kernel as clean internal modules, with no userspace-server IPC
tax on the core path.

**Rationale.** It is the fastest route to a working full OS for one person.
The capability discipline (D-06) applies at the ABI boundary, not through
process isolation of subsystems, and the framekernel containment result
(`010` §1) supports containing unsafe code inside a monolithic design.

**Consequences.** In-kernel bugs can take down the system (mitigated by
D-13/D-14); no seL4-style formal isolation story.
