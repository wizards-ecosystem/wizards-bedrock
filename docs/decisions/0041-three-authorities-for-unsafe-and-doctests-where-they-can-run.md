# D-41 Three authorities for `unsafe`, and doctests where they can run

Status: Accepted, 2026-08-22.

**Decision.** Three reasons admit `unsafe` to `wost`, and nothing else: a
novel abstraction, a measured performance need, or FFI/the platform
(Microsoft's Pragmatic Rust Guidelines, `008-research-notes-2.md` §2). Every
export is held to the Rustonomicon's sound-wrapper principle: safe code must
not be able to cause UB through this abstraction, however it is called. The
Unsafe Code Guidelines Reference is explicitly *not* an authority: it
describes itself as advisory and partly historical.

C-EXAMPLE is adopted in `libs/` only. `wost` and `kernel` keep the exemption
on the same grounds, now stated as a fact, not a blanket preference.

**Rationale.** A closed list turns the `005` judgement into a checklist, and
reason 2 is falsifiable: it needs a number. `libs/` carry
`#![cfg_attr(not(test), no_std)]`, so their doctests run inside the bare
`cargo test` that `cargo xtask ci` gates; there an example is the clearest
statement of a bit layout.

**Consequences.** `005` gains the taxonomy, the Nomicon standard and the
split, plus three rustdoc conventions: first line under fifteen words,
conventional section order, `#[doc(hidden)]` for technically-public items.
No lint enforces example presence, so review does, and the rule is written
down for that reason.

*Amended 2026-08-30 by D-249's framebuffer repair.* The scroll path's raw
`rep movsd` had moved address and length arithmetic outside `MmioRegion`,
with no measurement admitting D-41's performance exception. It is removed.
`libs/pixel` now proves each visible-row span against the padded layout,
while `MmioRegion` admits the complete destination slice before issuing
volatile platform writes. Padding determines row starts and the mapping
bound; it is not copied or published.
