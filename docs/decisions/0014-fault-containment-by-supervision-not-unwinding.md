# D-14 Fault containment by supervision, not unwinding

Status: Verified, 2026-07-18.

**Context.** Theseus OSDI'20 fault-injection data (verified): 800K-fault
campaign, only 69% recovery, failures dominated by Rust/LLVM's missing
asynchronous unwinding (still open in 2026).

**Decision.** Services are designed restartable under a supervisor;
`catch_unwind` is diagnostics only, never the containment mechanism.
Hardware isolation (rings + address spaces) remains the boundary for all
user code; language safety is a kernel-internal tool.

**Consequences.** Subsystems keep restartable state boundaries
(Hubris/Zircon lineage); panic = log + supervised restart or halt,
never "unwind and hope."
