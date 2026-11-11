# D-14 Fault containment by supervision, not unwinding

Status: Verified, 2026-07-18.

**Decision.** Services are designed restartable under a supervisor;
`catch_unwind` is diagnostics only, never the containment mechanism.
Hardware isolation (rings and address spaces) remains the boundary for all
user code; language safety is a kernel-internal tool.

**Rationale.** Theseus's fault-injection campaign (OSDI'20, verified)
recovered only 69% of faults, mostly because Rust/LLVM lacks asynchronous
unwinding.

**Consequences.** Subsystems keep restartable state boundaries
(Hubris/Zircon style); a panic logs, then restarts under supervision or
halts, never "unwind and hope."
