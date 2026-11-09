# D-41 Three authorities for `unsafe`, and doctests where they can run

Status: Accepted (2026-08-22)

**Context.** `005-code-conventions.md` admitted code to `wost` on a judgement:
"an operation that cannot be expressed safely". That phrasing has held, but it
is not a test a reviewer can apply to a borderline case, and Phase 3 is about
to add more `unsafe` to `wost` than any phase since the split — frame
ownership, page-table mutation, a heap.

The second research pass (`008-research-notes-2.md` §2) surfaced two things
`005` did not have. Microsoft's Pragmatic Rust Guidelines publish a **closed
list** of valid reasons to write `unsafe`, which turns the judgement into a
checklist. And the Rustonomicon states the standard a wrapper is actually held
to — *safe code must not be able to cause UB through this abstraction, however
it is called* — which is what `005`'s "checkable, not merely wrapped" was
reaching for.

The same pass found `005` overshooting in the other direction. It rejects
C-EXAMPLE outright, reasoning that "these crates are `no_std` and
cross-compiled, so doctests cannot run". True of `wost` and `kernel`. False of
`libs/`, which carry `#![cfg_attr(not(test), no_std)]` **precisely so** they
build for the host under `cargo test` — where doctests compile and run, inside
the bare `cargo test` that `default-members` already scopes and `cargo xtask
ci` already gates.

**Options.** (a) Leave the admission test as a judgement; (b) adopt the
taxonomy as guidance in prose; (c) adopt it as the admission test, name the
Nomicon as the standard, and split C-EXAMPLE by where doctests execute.

**Decision.** (c). Three reasons admit `unsafe` to `wost` — a novel
abstraction, a measured performance need, or FFI/the platform — and nothing
else. Every export is held to the sound-wrapper principle. The Unsafe Code
Guidelines Reference is explicitly *not* an authority, because it describes
itself as advisory and partly historical, and citing it as a rule has misled
people before.

C-EXAMPLE is adopted in `libs/` only. `wost` and `kernel` keep the exemption
on the same grounds as before, now stated as the fact it is rather than a
blanket preference.

**Rationale.** The taxonomy's value is that reason 2 is falsifiable: it needs
a number, and nothing in this kernel has ever produced one, which is itself
worth knowing. The C-EXAMPLE split costs nothing — the harness that runs those
doctests has existed since D-26 — and it lands exactly where it is most useful,
on the encoding crates D-30 keeps producing, where an example is the clearest
statement of a bit layout available.

**Consequences.** No code change; `005` gains the taxonomy, the Nomicon
standard, and the split, plus the three rustdoc conventions that go with it
(first line under fifteen words, conventional section order, `#[doc(hidden)]`
for technically-public items). `libs/x86-paging` is the first crate written
under the new rule and carries doctests on its entry encoders. The lint that
would enforce example presence does not exist, so this one is enforced by
review — which is why it is written down.

*Amended 2026-08-30 by D-249's framebuffer repair.* The scroll path's raw
`rep movsd` had moved address and length arithmetic outside `MmioRegion` and
had no measurement admitting D-41's performance exception. It is removed.
`libs/pixel` now proves each visible-row span against the padded layout, while
`MmioRegion` admits the complete destination slice before issuing volatile
platform writes. Padding determines row starts and the mapping bound; it is
not copied or published.
