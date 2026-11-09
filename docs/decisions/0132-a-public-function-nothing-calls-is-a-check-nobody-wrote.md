# D-132 A public function nothing calls is a check nobody wrote

Status: Accepted (2026-08-25), amends D-130

**Context.** D-129 swept `wost` for `pub fn`s nothing calls and found two. The
same sweep over `libs/` is a different question with a different answer, and
worth stating why: `kernel/` is a binary crate, so `dead_code` already reports
everything unreachable in it — that is what D-130's twenty-six attributes were
suppressing. A `libs/` crate is a library, so its public surface is reachable
*by definition* and the compiler will never say a word about it. Nothing in the
gate looks at it at all.

Two were dead. One is dead weight; the other is the interesting one.

**`Mixer::is_identity` had no caller anywhere, including in tests.** It exists
to ask exactly one question, and D-95 is why: the kernel derived its own mixer,
got `Mixer(0)` for the first table, and every handle in that table came out
unmixed — the first one was the literal integer three. `for_table` was written
to fix that, `is_identity` was written to name the property, and then **no test
in the crate called `for_table` at all.** Every test uses `Mixer::new` with a
hand-picked non-zero seed. So D-95's fix has never been exercised, and the
helper standing next to it made it look as though it had been.

And the property does not hold. `for_table` reads "distinct and never the
identity", and the arithmetic says almost: the multiplier is odd, so the
multiply is a bijection mod 2³², so **exactly four `id`s land in
`0..=RESERVED`** — which is precisely the range `Mixer::new` clears to zero.
Four table numbers out of four billion get a mixer that mixes nothing:

    3016240323  3356813644  3697386965  4037960286

No kernel counting tables upward reaches one. That is also exactly what was
true of `Mixer(0)` until the day the kernel derived one.

**Options.**

1. **Correct the doc to "almost never".** Honest, and leaves a mixer that can
   be the identity in a codebase whose reason for having a mixer is D-95.
2. **Force a bit on after the mask**, so the result cannot be zero. Cheap, and
   it breaks the bijection — two `id`s differing only in that bit would collide,
   doubling the collision class for every table rather than for four.
3. **Divert the four.** Keep the bijection, and give the identity's preimages
   the multiplier itself.

**Decision.** Option 3, with the two constants named — `MULTIPLIER` and
`ADDEND` — and a test that derives the four `id`s rather than listing them.

The test computes the multiplier's inverse mod 2³² by Newton's iteration, five
steps, asserts it *is* an inverse, and uses it to name the four preimages. A
hardcoded list of four magic numbers would be a constant that only equals
itself, which is the thing D-124 was about; a derived one fails if anybody
changes the multiplier.

**Rationale.** The bug is four-in-four-billion and the finding is not. What
this is really about is that D-95's property had a *name* and no *check*, and
the name is what made it invisible: a reader who greps for the concern finds
`is_identity`, sees a function whose whole purpose is to answer it, and
concludes it is covered. An unused helper is worse than an absent one, because
it reads as evidence.

That generalises past this instance. `dead_code` is a real check and it stops
at a crate's public boundary, so every `libs/` crate has a surface the gate does
not look at — and this project puts its most careful arithmetic there precisely
because `cargo test` can reach it.

Why not option 2: the collision structure is already four-to-one from the mask,
and it does not matter, because a table's identity is compared separately —
`entry.owner != self.id` is what refuses a handle from another table, not the
mixer. The mixer's job is to remove the accidental case. Making it worse for
every table to fix four of them is the wrong trade, and it would have made the
new "two table numbers do not share a mixer" test fail, which is how the trade
became visible rather than theoretical.

**Consequences.**

- `for_table` never returns the identity, and its doc no longer says something
  the code does not do. Broken and watched go red (D-28): the old body fails
  `no_table_number_gets_a_mixer_that_mixes_nothing` on all four `id`s.
- A second test records the bound the first one does not: table numbers spread
  across distinct mixers, *four-to-one at the mask*, which is a property of the
  format rather than a fault.
- `Flags::intersects` in `libs/x86-paging` is deleted. Zero references, tests
  included; `contains` is what every caller wants and `is_write_execute` is
  where the one composite predicate lives (D-12).
- `xstate::Layout::size`'s doc said "never less than `HEADER_BYTES`" two lines
  above `is_consistent`, the function written to catch the case where it
  returns zero. It now says what it does and points at the invariant.

**Amendment to D-130.** D-130 concluded that converting `#[allow]` to
`#[expect]` makes an unfalsifiable claim falsifiable. Two limits were measured
afterwards and are recorded in D-131; both are real and neither was foreseen
here. In short: an `expect` inside a **macro body** is reported by nothing, so
it is *less* honest than the `allow` it replaces — it reads as checked. And an
`expect` asserts that a lint *does* fire, so it is wrong wherever the
suppression grants a standing exemption rather than covering a specific
construct — `wost`'s crate-level `#![allow(unsafe_code)]` says `wost` is the
only crate that **may** contain unsafe, not that it does, and an `expect` there
would fail the build on the day the crate was finally emptied.

**Left owed.** The `libs/` sweep was done by hand, with a shell loop counting
identifier occurrences, and **a green result from it is not evidence of
anything.** It found two out of roughly four hundred public functions — a good
enough ratio to be worth repeating and a bad enough tool to be worth replacing.
It cannot see a name used only in a doc comment, so it over-reports; it would
miss anything reachable only through a `pub use`, so it under-reports; and it
matches text rather than resolving paths, so two items sharing a name defeat it
entirely. A second pass over `pub const`, `pub static`, `pub type` and the
`pub struct`/`enum`/`trait` declarations came back clean, which says as much
about the tool as about the tree.

D-129's sweep of `wost` was narrower still — top-level `pub fn` only, where both
of these are inherent methods — so the two passes together are not a covering
one. `xtask` is where a real one belongs, beside `unsafety` and `decisions`,
and it is not written. Until it is, `libs/`'s public surface is the one place in
this repository where "nothing reported it" means nothing looked.
