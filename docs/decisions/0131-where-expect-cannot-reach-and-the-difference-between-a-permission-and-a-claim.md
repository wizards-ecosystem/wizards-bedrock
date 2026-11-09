# D-131 Where `expect` cannot reach, and the difference between a permission and a claim

Status: Accepted, 2026-08-25.

**Context.** D-130 replaced `#[allow(dead_code)]` with
`#[cfg_attr(not(feature = "selftest"), expect(dead_code))]` across `kernel/`
and found seven attributes that had stopped being true. Its "Left owed" named
`wost`, where the form would be a plain `#[expect(dead_code)]` because `wost`'s
dead code is dead in every configuration.

`wost` has no `#[allow(dead_code)]`. So this took the wider question instead:
**every** lint suppression in `wost` and `libs/` is the same kind of assertion,
and none of them was checked either.

There were fourteen. Converting all of them found one stale, and — more
usefully — found the two places where the technique does not apply.

**What was stale.** `#![allow(unreachable_pub)]` on `port::sealed`, the module
holding the sealed-trait marker. The lint no longer fires there: `PortValue:
sealed::Sealed` makes `Sealed` reachable through a public bound, so the trait
is not unreachable and has not been for some time. Deleted. Putting it back
fails `cargo xtask clippy`, which is the break-test.

Two more were live and are now falsifiable:
`missing_debug_implementations` on the Limine glue, and `missing_docs` on
`ps2-keys`' key names, where the comment beside it — "the names are the
documentation" — is the kind of justification that ages well and is worth
keeping honest.

**The first limit: `expect` does not fire inside a macro expansion.**

`boot::entry!` carries ten `#[allow(unsafe_code)]`. They exist so the calling
crate can hold `#![deny(unsafe_code)]` while `wost`-authored code expands
there; each sits on a `#[used] static` with an `#[unsafe(link_section = …)]`.
Converting them to `expect` compiled clean, which looked like a result.

It was not one, and it was measured rather than reasoned:

1. Deleting the `#[unsafe(link_section)]` from one of those statics — the only
   thing on it that triggers `unsafe_code` — and leaving the `expect` produced
   **no unfulfilled expectation**.
2. Adding `#[expect(non_snake_case)]` to a static named
   `__WOST_REQUESTS_START`, which nothing could possibly fulfil, was reported
   by **nothing**.

`unfulfilled_lint_expectations` does not reach into a macro expansion. So an
`expect` there reads as checked and is exactly as unfalsifiable as the `allow`
it replaced — which is **worse than the `allow`**, because the next reader
spends their scepticism elsewhere. All ten are reverted, and the macro's doc
comment carries the measurement rather than this entry alone.

**The second limit: a permission is not a claim.**

`wost/src/lib.rs` carries `#![allow(unsafe_code)]`, the one place the
workspace's deny is lifted (D-67). It converts cleanly and stays `allow`.

`expect` asserts that the lint *does* fire. Here that means "`wost` contains
`unsafe`". The rule this attribute implements is that `wost` is the only crate
that **may** — D-13's framekernel discipline, not a quota. A refactor that
emptied `wost` of `unsafe` would be the best day this design ever had, and an
`expect` would fail the build for it.

The general form, which is what makes this worth an entry rather than a
comment: **`expect` is right where the suppression covers a specific construct,
and wrong where it grants a standing exemption.** D-130's `dead_code`
attributes are the first kind — each names one item that has no caller.
A crate-level `unsafe_code` lift is the second.

**Decision.** Convert the suppressions that cover specific constructs and are
not macro-expanded; delete the one that had gone stale; leave the ten macro
attributes and the crate-level lift as `allow`, each with the reason in the
source rather than only here.

**Rationale.** D-130's argument is right and this does not weaken it: an
`allow` that stopped being needed is silent for ever, and the gate should hold
us to the claim. What this adds is where that stops working, found the only way
it can be — by deliberately breaking an expectation and watching nothing
happen. A technique with an unstated boundary is how a later session gets a
false green, and "convert `allow` to `expect`" reads as universally applicable.

The macro case is the one to remember, because it is invisible: the attribute
is written in `wost`, the lint is evaluated in `kernel/`, and the reason it
cannot be checked is a property of neither file.

**Consequences.**

- `wost` and `libs/` hold two `expect`s where they held `allow`s, one attribute
  fewer than before, and eleven `allow`s that are documented as unconvertible
  with why.
- No check-count change and no image change: this is entirely about what fails
  the build, not what the build produces.
- D-130's boundary is stated. Anyone converting a macro's attributes now finds
  this before spending a session on it.

**Left owed.** The eleven remaining `allow`s in `wost` are unfalsifiable and
stay that way. The macro ten could be made checkable by moving the statics out
of the macro and into `wost` proper — which rust-lang/rust#47384 forbids, since
`#[used]` statics inside an rlib can be dropped wholesale by the linker. That
is the same constraint D-67 recorded, so the ten are as good as they get until
that changes.
