# D-13 Framekernel discipline: all `unsafe` in one privileged crate

Status: Provisional, 2026-07-18. Provisional since 2026-08-23: the containment property is verified, the published numbers are unreplicated; see D-71 and `010` §1.

**Context.** Research pass, strongest verified result (Asterinas,
USENIX ATC'25).

**Decision.** The kernel is one address space with monolithic
performance, but ALL `unsafe` Rust lives in a small privileged framework
crate — **`wost`** (Wizard OS Standard, our OSTD analogue): boot,
paging, MMIO, port I/O, DMA, context switch, interrupt entry. Every
other kernel crate (scheduler, VFS, WizFS, network, drivers) is
`#![forbid(unsafe_code)]` safe Rust written against `wost`'s safe
abstractions.

**Rationale, amended by D-71 and `010` §1.** The paper reported a ~14% TCB ratio
and broad Linux-parity single-core results. Those numbers are retained here as
historical context, not as evidence: absolute TCB size is the relevant audit
cost, and only single-core syscall dispatch is a defensible performance
summary. No independent replication exists, which is why this ADR is
Provisional. The containment *property* is verified on this codebase, and the
authors' lesson that safe MMIO/port/DMA abstractions must exist from day one
still stands.

**Consequences.** The existing Phase-0 kernel gets restructured into
`wost` + service crates before Phase 2 lands significant code. Safe DMA
is genuinely hard (ownership of in-flight buffers) — `wost`'s API
design is a first-class engineering problem, not plumbing.
