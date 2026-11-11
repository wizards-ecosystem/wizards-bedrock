# D-30 Descriptor encoding is a host-tested library

Status: Accepted, 2026-08-13.

**Decision.** `libs/x86-tables` holds the GDT, IDT and TSS bit layouts, the
selector arithmetic, and the exception name and error-code tables. `wost`
builds descriptors from it and only loads them.

**Rationale.** A wrong descriptor bit gives no diagnosable failure: it gives
a triple fault, or a machine running with the wrong privilege. Only a host
test against the manual (SDM table 6-1 for the error-code mask) can check
it.

**Consequences.** The crate is x86-specific, a first for `libs/`. A port
adds a sibling crate instead of growing this one. `ERROR_CODE_VECTORS` feeds
both the host tests and, as a `const` operand, the stub-generating
assembler, so the two cannot drift.
