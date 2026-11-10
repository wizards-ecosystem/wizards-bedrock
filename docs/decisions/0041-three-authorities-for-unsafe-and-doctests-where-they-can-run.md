# D-41 Three authorities for `unsafe`, and doctests where they can run

Status: Accepted, 2026-08-22.

**Context.** `005-code-conventions.md` admitted code to `wost` on a
judgement: "an operation that cannot be expressed safely". That phrasing has
held, but it is no test for a borderline case. Phase 3 is about to add more
`unsafe` to `wost` than any phase since the split: frame ownership,
page-table mutation, a heap.

The second research pass (`008-research-notes-2.md` §2) found two things
`005` lacked. Microsoft's Pragmatic Rust Guidelines publish a closed list of
valid reasons to write `unsafe`, which turns the judgement into a checklist.
The Rustonomicon states the standard a wrapper is held to: *safe code must
not be able to cause UB through this abstraction, however it is called*.
That is what `005`'s "checkable, not merely wrapped" was reaching for.

The same pass found `005` overshooting the other way. It rejects C-EXAMPLE
outright, reasoning that "these crates are `no_std` and cross-compiled, so
doctests cannot run". That is true of `wost` and `kernel` and false of
`libs/`, which carry `#![cfg_attr(not(test), no_std)]` to build for the host
under `cargo test`. Doctests compile and run there, inside the bare
`cargo test` that `default-members` scopes and `cargo xtask ci` gates.

**Options.** (a) Leave the admission test as a judgement; (b) adopt the
taxonomy as guidance in prose; (c) adopt it as the admission test, name the
Nomicon as the standard, and split C-EXAMPLE by where doctests execute.

**Decision.** (c). Three reasons admit `unsafe` to `wost`, and nothing else:
a novel abstraction, a measured performance need, or FFI/the platform. Every
export is held to the sound-wrapper principle. The Unsafe Code Guidelines
Reference is explicitly *not* an authority. It describes itself as advisory
and partly historical, and citing it as a rule has misled people before.

C-EXAMPLE is adopted in `libs/` only. `wost` and `kernel` keep the exemption
on the same grounds, now stated as a fact, not a blanket preference.

**Rationale.** The taxonomy's value is that reason 2 is falsifiable: it needs
a number, and nothing in this kernel has ever produced one, which is worth
knowing. The C-EXAMPLE split costs nothing: the harness for those doctests
has existed since D-26. It lands on the encoding crates D-30 keeps
producing, where an example is the clearest statement of a bit layout.

**Consequences.** No code change. `005` gains the taxonomy, the Nomicon
standard and the split, plus three matching rustdoc conventions: first line
under fifteen words, conventional section order, `#[doc(hidden)]` for
technically-public items. `libs/x86-paging` is the first crate written under
the new rule, with doctests on its entry encoders. No lint enforces example
presence, so review does, and the rule is written down for that reason.

*Amended 2026-08-30 by D-249's framebuffer repair.* The scroll path's raw
`rep movsd` had moved address and length arithmetic outside `MmioRegion`,
with no measurement admitting D-41's performance exception. It is removed.
`libs/pixel` now proves each visible-row span against the padded layout,
while `MmioRegion` admits the complete destination slice before issuing
volatile platform writes. Padding determines row starts and the mapping
bound; it is not copied or published.
