# D-132 A public function nothing calls is a check nobody wrote

Status: Accepted, 2026-08-25. Amends D-130.

**Decision.** `Mixer::for_table` never returns the identity. Its multiplier is
odd, so the multiply is a bijection mod 2³² and **exactly four `id`s land in
`0..=RESERVED`**, the range `Mixer::new` clears to zero. Those four preimages
get the multiplier itself, which keeps the bijection. The constants are named
`MULTIPLIER` and `ADDEND`, and the test derives the four `id`s through the
multiplier's inverse mod 2³² (Newton's iteration, asserted to be an inverse)
rather than listing them, because a hardcoded list would be a constant that
only equals itself (D-124).

The unused `Mixer::is_identity` is replaced by
`no_table_number_gets_a_mixer_that_mixes_nothing`, which calls `for_table`.
`Flags::intersects` in `libs/x86-paging` is deleted: `contains` is what every
caller wants and `is_write_execute` is where the one composite predicate lives
(D-12). `xstate::Layout::size`'s doc says what it does and points at
`is_consistent`.

**Rationale.** D-95's property had a *name* and no *check*: no test called
`for_table`, and `is_identity` standing beside it made it read as covered. An
unused helper is worse than an absent one, because it reads as evidence.
`dead_code` stops at a crate's public boundary, so every `libs/` crate has a
surface the gate does not look at. Forcing a bit on after the mask would break
the bijection and double every table's collision class to fix four; it does not
matter for authority, because a table's identity is compared separately
(`entry.owner != self.id` refuses a handle from another table), and the mixer's
job is to remove the accidental case.

**Consequences.** A second test records that table numbers spread across
distinct mixers, *four-to-one at the mask*, which is a property of the format
rather than a fault.

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
anything.** It over-reports names used only in doc comments, under-reports
anything reachable only through a `pub use`, and matches text rather than
resolving paths. D-129's sweep of `wost` was narrower still, so the two passes
together are not a covering one. `xtask` is where a real one belongs, beside
`unsafety` and `decisions`, and it is not written. Until it is, `libs/`'s
public surface is the one place in this repository where "nothing reported it"
means nothing looked.
