# D-04 Kernel shape: monolithic-modular

Status: Verified, 2026-07-18. As refined by D-13.

**Context.** Solo developer, multi-year horizon, full-OS destination.

**Options.** Monolithic-modular; microkernel; hybrid.

**Decision.** Monolithic-modular: drivers, filesystems, and the network
stack live in the kernel as clean internal modules; no userspace-server
IPC tax on the core path.

**Rationale.** Fastest route to a working full OS for one person; the
capability discipline (D-06) applies at the ABI boundary rather than via
process isolation of subsystems. The framekernel evidence supports containing
unsafe code inside a monolithic design; `010` §1 withdrew the percentage-based
TCB and broad performance claims while leaving that containment result intact.
Capsicum's measurements (unverified tier) found direct capability-checked
syscalls cost only a few percent — supporting "fast path over message passing."

**Consequences.** In-kernel bugs can take down the system (mitigated by
D-13/D-14); no seL4-style formal isolation story.
