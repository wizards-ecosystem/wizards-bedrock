# D-130 An `allow` is unfalsifiable and an `expect` is not

Status: Accepted, 2026-08-25.

**Decision.** In `kernel/`, an item with no shipped caller carries
`#[cfg_attr(not(feature = "selftest"), expect(dead_code))]`, not
`#[allow(dead_code)]`; the attributes that had stopped being true are deleted
with the prose that repeated them. `expect` fails the build when the lint it
names does *not* fire, and the gate builds both kernel configurations with
`-D warnings` (D-22), so the attribute is falsifiable from both sides:

- if a shipped caller appears, `unfulfilled_lint_expectations` fires in the
  build without the checks;
- if the item goes dead in the checked build too, `dead_code` fires there,
  where there is no attribute at all.

An attribute sits immediately above its item, never between the halves of a
doc comment, where it renders as two paragraphs with nothing to say they were
ever one.

**Rationale.** This is D-28's argument moved off checks and onto attributes: an
`allow` that has stopped being needed is a claim that is silent for ever, and
`#[allow]` looks like enforcement. Gating the items out with `cfg` instead
would make the checks test a different binary from the one that ships, which
D-22 refuses. Stale attributes are found **empirically rather than by reading
call sites**: remove them, compile both configurations, and the compiler names
them. `Expect` seeds rustc's dead-code worklist the same as `Allow`, so the two
forms differ only in what happens when the claim stops being true.

**Consequences.** Putting the attribute back on `ObjectType`, which has had a
shipped caller since D-116, fails `cargo xtask clippy` with `this lint
expectation is unfulfilled`. The image is byte-identical; what changed is what
the gate refuses.

**Left owed.** `wost/` has its own `#[allow]`s and has not been swept. The same
treatment applies and the same empirical method finds them, but `wost` has no
feature to gate on (its dead code is dead in every configuration), so the form
there is a plain `#[expect(dead_code)]` rather than a `cfg_attr`, and the
sweep should be its own pass rather than folded into this one.

`#[allow(unused)]` and the other blanket allows are not covered here. Every one
of them is the same kind of claim and none of them is checked.

**Amended by D-131 and D-132.** The conclusion above — that converting `allow`
to `expect` makes an unfalsifiable claim falsifiable — has two limits, both
measured after this was written and neither foreseen in it. An `expect` inside a
**macro body** is reported by nothing, which makes it *worse* than the `allow`
it replaces rather than better: it reads as checked. And an `expect` asserts
that a lint does fire, so it is wrong wherever the suppression is a standing
exemption rather than a specific construct — `wost`'s crate-level
`#![allow(unsafe_code)]` says `wost` is the only crate that **may** hold unsafe,
not that it does. D-131 has both, with the measurements. Nothing above is
retracted; the sweep it describes was over `kernel/`, where neither limit
applies.
