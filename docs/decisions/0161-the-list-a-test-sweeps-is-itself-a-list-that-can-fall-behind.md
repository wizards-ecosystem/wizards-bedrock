# D-161 The list a test sweeps is itself a list that can fall behind

Status: Accepted (2026-08-25)

**Context.** D-157 said a check whose name quantifies must read its domain
rather than restate it, and claimed a sweep of the other fifty-odd
universally-named checks found no third instance. That sweep looked at test
*names* against their bodies. It missed the case where the body does iterate a
list — and the list is the thing that has fallen behind.

`libs/x86-apic` keeps `const ALL: &[(&str, Register)]` in its test module,
carrying a comment that reads *"Every register named here, so the sweeps below
cannot miss one."* Three tests iterate it: the alignment check, the offset
collision check, and — separately — a second hand-written list anchors each
register to the MSR the SDM gives it. Nothing tied any of it to the fourteen
`pub const NAME: Register` the crate actually defines. A fifteenth register
added and left out of `ALL` would shrink all three sweeps at once, and all
three would go on reporting success.

`libs/x86-apic::timer` has the same shape over the eight `Divide` variants.

**Decision.** Both lists are anchored to what they claim to enumerate, by the
means each one admits.

`ALL` for registers is compared against the crate's own source, read through
`include_str!("register.rs")`, **by offset rather than by count** — a list that
dropped one register and repeated another has the right length, and only the
offsets say so. The MSR anchoring list is then tied to `ALL`'s length, so a
register defined without a manual value behind it fails rather than passes
unmentioned.

`ALL` for `Divide` is anchored by an **exhaustive match**, which makes the
compiler the checker: a ninth variant is `error[E0004]: non-exhaustive
patterns` and the crate stops building. The length assertion sits after it for
the case the match is updated and the list is not.

**Rationale.** These are the two mechanisms available, and which one applies is
a property of what is being enumerated, not a preference. Enum variants can be
enumerated by the compiler, and where they can, that is strictly better than a
test: it fails at build time, in every configuration, without being run. A set
of `pub const` items cannot be, so the source is read instead — the same
technique D-157 used on `Ring.tla` and on `cli.rs`'s own match arms.

The general statement, which now covers three findings in one day: **a check is
only as good as the enumeration it sweeps, and an enumeration written by hand
beside its subject is a copy.** D-157 caught copies of a config and a parser;
this catches copies of a crate's own item list. The tell is identical in all
three — the check contains a copy of its subject.

**Consequences.**

- Two new tests in `libs/x86-apic`, four breaks, each red: a register defined
  and left out of `ALL`; a register dropped from `ALL`; a register with no MSR
  anchored to the manual; a `Divide` variant missing from `ALL`. The
  compile-time anchor was demonstrated separately — a ninth variant fails the
  build with `non-exhaustive patterns`, in the crate's production code as well
  as the test.
- The comment claiming `ALL` was complete is now a check that it is.
- `libs/timebase`'s `PLAUSIBLE_HZ` was examined and left. It is a list of real
  clock frequencies to test *against*, not an enumeration of anything the crate
  defines, so there is nothing for it to fall behind.
