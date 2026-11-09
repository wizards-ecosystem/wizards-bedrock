# D-30 Descriptor encoding is a host-tested library

Status: Accepted (2026-08-13)

**Context.** `005-code-conventions.md` names "descriptor encoding" as a future
`libs/` crate. Phase 2 is where it arrives.

**Decision.** `libs/x86-tables` holds the GDT, IDT and TSS bit layouts, the
selector arithmetic, and the exception name and error-code tables. `wost` builds
descriptors from it and does nothing to them but load them.

**Rationale.** A wrong bit in a descriptor does not produce a diagnosable
failure; it produces a triple fault, or worse, a machine that runs with the
wrong privilege. The only place that can be checked is a host test against the
manual — `size_of::<Gate>() == 16`, the TSS's field offsets, the code descriptor
equalling `0x00209A0000000000`, and the error-code mask agreeing with a
transcribed copy of SDM table 6-1. Twenty-six such tests exist, and none of them
could run in the kernel.

**Consequences.** The crate is x86-specific, which is a first for `libs/`; a
port adds a sibling rather than growing this one. `ERROR_CODE_VECTORS` is
consumed both by the host tests and, as a `const` operand, by the assembler that
generates the stubs — so the two cannot drift.
