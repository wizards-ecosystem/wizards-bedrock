# D-131 Where `expect` cannot reach, and the difference between a permission and a claim

Status: Accepted, 2026-08-25.

**Decision.** Every lint suppression in `wost` and `libs/` is the same kind of
assertion as D-130's. Convert the ones that cover specific constructs and are
not macro-expanded; delete the stale ones; leave macro attributes and the
crate-level lift as `allow`, each with the reason in the source.
`#![allow(unreachable_pub)]` on `port::sealed` was stale (`PortValue:
sealed::Sealed` makes `Sealed` reachable) and is deleted; putting it back fails
`cargo xtask clippy`.

**The first limit: `expect` does not fire inside a macro expansion.**
`boot::entry!`'s `#[allow(unsafe_code)]`s let the calling crate hold
`#![deny(unsafe_code)]` while `wost`-authored code expands there. Measured, not
reasoned: deleting the only trigger under an `expect` produced **no unfulfilled
expectation**, and `#[expect(non_snake_case)]` on `__WOST_REQUESTS_START`, which
nothing could fulfil, was reported by **nothing**. An `expect` there reads as
checked and is exactly as unfalsifiable as the `allow`, which is **worse than
the `allow`**, because the next reader spends their scepticism elsewhere. The
macro's doc comment carries the measurement.

**The second limit: a permission is not a claim.** `wost/src/lib.rs` carries
`#![allow(unsafe_code)]`, the one place the workspace's deny is lifted (D-67),
and it stays `allow`. `expect` would assert "`wost` contains `unsafe`", while
the rule is that `wost` is the only crate that **may**: D-13's framekernel
discipline, not a quota. **`expect` is right where the suppression covers a
specific construct, and wrong where it grants a standing exemption.**

**Rationale.** D-130's argument is right and this does not weaken it. A
technique with an unstated boundary is how a later session gets a false green,
and the macro case is invisible: the attribute is written in `wost`, the lint
is evaluated in `kernel/`, and the reason it cannot be checked is a property of
neither file.

**Left owed.** The remaining `allow`s in `wost` are unfalsifiable and stay that
way. The macro ones could be made checkable by moving the statics out of the
macro and into `wost` proper, which rust-lang/rust#47384 forbids, since
`#[used]` statics inside an rlib can be dropped wholesale by the linker. That
is the same constraint D-67 recorded, so they are as good as they get until
that changes.
