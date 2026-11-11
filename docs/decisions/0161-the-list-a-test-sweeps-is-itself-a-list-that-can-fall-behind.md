# D-161 The list a test sweeps is itself a list that can fall behind

Status: Accepted, 2026-08-25.

**Decision.** A hand-written list a test sweeps is anchored to what it claims to
enumerate, by the means its subject admits.

- `libs/x86-apic`'s register `ALL`, which three sweeps iterate and whose
  comment claimed *"Every register named here, so the sweeps below cannot miss
  one"*, is compared against the crate's own source through
  `include_str!("register.rs")`, **by offset rather than by count**: a list
  that dropped one register and repeated another has the right length, and
  only the offsets say so. The MSR anchoring list is tied to `ALL`'s length, so
  a register defined without a manual value behind it fails.
- `libs/x86-apic::timer`'s `ALL` over the eight `Divide` variants is anchored by
  an **exhaustive match**, so a ninth variant is `error[E0004]: non-exhaustive
  patterns` and the crate stops building. A length assertion after it covers a
  match updated without the list.

**Rationale.** Which mechanism applies is a property of what is enumerated.
Where the compiler can enumerate (enum variants), that is strictly better than a
test: it fails at build time, in every configuration, without being run. A set
of `pub const` items cannot be, so the source is read, the technique D-157 used
on `Ring.tla` and on `cli.rs`'s own match arms. **A check is only as good as the
enumeration it sweeps, and an enumeration written by hand beside its subject is
a copy.** D-157 caught copies of a config and a parser; this catches copies of a
crate's own item list.

**Consequences.** `libs/timebase`'s `PLAUSIBLE_HZ` is left: it is a list of real
clock frequencies to test against, not an enumeration of anything the crate
defines, so there is nothing for it to fall behind.
