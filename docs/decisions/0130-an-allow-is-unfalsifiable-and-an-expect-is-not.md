# D-130 An `allow` is unfalsifiable and an `expect` is not

Status: Accepted (2026-08-25)

**Context.** `kernel/` carried twenty-six `#[allow(dead_code)]`. Each one is an
assertion — *this item has no caller in the shipped build, and that is
deliberate* — and each was written when it was true. Nothing checked that any
of them still was.

Seven had stopped being true, and every one of them stopped for the same
reason: the shipped kernel grew a caller. D-116 made it run a program, so
`user::spawn` became live, and with it `HandleTable`'s working half, `Entry`'s
fields, `HandleError`'s variants, `ObjectType`, `Bootstrap`, `spawn_in_space`
and `spawn_weighted`. The attributes stayed. So did the prose beside them:
`spawn_in_space` said "no shipped caller: the shipped kernel reads the keyboard
and runs no user program", and `HandleTable`'s working half said the shipped
kernel reached none of it. Both were describing the kernel of two phases ago,
in a doc comment a reader would have no reason to doubt.

This is D-124's shape — a comment that states a property is not the property —
with an attribute standing in for the comment. It is worse than a comment in
one respect: a comment is at least visibly prose, and `#[allow]` looks like
enforcement.

Three of them had a second problem. `spawn_weighted`, `spawn_in_space` and
`yield_now` carried the attribute **between the halves of their doc comment** —
legal, since a doc comment is itself an attribute, and it renders as two
paragraphs with nothing to say they were ever one. Nobody reading the built
documentation would see anything wrong.

**Options.**

1. **Audit them by hand, now.** Fixes today's seven and nothing about
   tomorrow's.
2. **Delete the attributes and `cfg`-gate the items instead.** Correct, and it
   means the shipped image stops carrying the code the checks exercise — so a
   check would be testing a different binary from the one that ships, which
   D-22 already refuses on the linting side.
3. **`#[cfg_attr(not(feature = "selftest"), expect(dead_code))]`.** State it as
   an expectation rather than a permission, and gate it on the configuration
   the claim is actually about.

**Decision.** Option 3, for the nineteen that are still true; the seven that
are not are deleted along with the prose that repeated them.

`expect` fails the build when the lint it names does *not* fire. So the
attribute is now falsifiable from both sides, and the gate already builds both
kernel configurations with `-D warnings` (D-22):

- if a shipped caller appears, `unfulfilled_lint_expectations` fires in the
  build without the checks;
- if the item goes dead in the checked build too, `dead_code` fires there,
  where there is no attribute at all.

**Rationale.** This is D-28's argument moved off checks and onto attributes. A
check that cannot fail is worth less than no check, because it is also a claim;
an `allow` that has stopped being needed is exactly that, and it is silent for
ever. Twenty-six unfalsifiable assertions is not a tidiness problem, it is
twenty-six things the gate was believed to be holding and was not.

Which seven were stale was found **empirically rather than by reading call
sites**: the attributes were removed wholesale, both configurations compiled,
and the compiler named them. That is the same move as breaking a check to watch
it go red — the answer comes from the machine, and reasoning about call graphs
across a `#[cfg]` boundary is exactly the reasoning that produced the seven.

One property had to be confirmed rather than assumed, because the whole
approach rests on it: rustc's dead-code pass seeds its worklist from items
whose lint level is `Allow`, and `Expect` is a different level. If `expect` did
not seed the same roots, converting would have reported every item an allowed
one transitively used. It does seed them — the conversion produced seven
unfulfilled expectations and not one new `dead_code` — so the two forms differ
only in what happens when the claim stops being true, which is the whole point.

**Consequences.**

- Nineteen `#[cfg_attr(not(feature = "selftest"), expect(dead_code))]` in
  `kernel/`, each of which the gate now holds us to; seven attributes and three
  stale sentences gone.
- Broken and watched go red (D-28): putting the attribute back on `ObjectType`,
  which has had a shipped caller since D-116, fails `cargo xtask clippy` with
  `this lint expectation is unfulfilled`. That is the failure mode the whole
  change exists to buy, so it is the one that had to be seen.
- The three attributes wedged inside a doc comment are out of it. `yield_now`
  keeps its expectation and now carries it immediately above the item, where an
  attribute belongs.
- No check count changes and no behaviour changes. The image is byte-identical;
  what changed is what the gate refuses.

**Left owed.** `wost/` has its own `#[allow]`s and has not been swept. The same
treatment applies and the same empirical method finds them, but `wost` has no
feature to gate on — its dead code is dead in every configuration — so the form
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
