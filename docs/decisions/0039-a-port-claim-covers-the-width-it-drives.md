# D-39 A port claim covers the width it drives

Status: Accepted, 2026-08-22.

**Decision.** `size_of::<T>()` is the number of ports an access drives. The
sealed `PortValue` trait guarantees that, since its only implementors are
the primitives of those widths. `claim` reserves that many, and
`PortBlock::read`/`write` check `offset + size_of::<T>() <= N`. The bitmap
moved to a new `libs/port-space` crate, on D-30's precedent: it is
arithmetic over bits with no hardware behind it, so it belongs where
`cargo test` reaches it. `Port::new` and `PortBlock::new` are private;
`claim` and `claim_block` are the only doors.

**Rationale.** A one-bit-per-claim registry let a `Port<u32>` drive four
ports against a one-port claim, and a `PortBlock` access run past its block.
`005-code-conventions.md` requires exclusivity "verified rather than
promised", and a caller should not have to remember the width; it is already
in the type.

**Consequences.** Phase 3's device mappings inherit this discipline:
`007-memory.md` asks for "a claim discipline like `port::claim`'s", which
means covering the span an access reaches.
