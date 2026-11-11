# D-174 The Phase 6 audit: one defect in three places, and a claim nothing checked

Status: Accepted, 2026-08-25.

**Decision.** **Every field a submission carries is a value ring 3 chose.** `as`
is not a conversion at that boundary; it is a silent reinterpretation, and the
only casts that belong there are widening ones. The Phase 6 close audit (D-77's
pattern) found three narrowing casts, one defect, now all `try_from`:

- A **rights word**: `Rights::from_bits(submission.argument as u32)` read
  `0xFFFF_FFFF_0000_0003` as `DUPLICATE | TRANSFER`, defeating the reason
  `from_bits` returns an `Option`. It is **refused**.
- A **cancellation token**: `0x1_0000_DEAD` would end the operation tagged
  `0xDEAD`. It is **refused**.
- A **file offset**: four billion and one wrapped to one. It is clamped to
  `u32::MAX`, which is past every file this kernel can hold and so reads
  nothing, as an offset past the end reads everywhere else here.

`devfs::DEVICES` claimed it was "checked against [the enum] by a test that
counts (D-161)", and no such test existed; it now does, with the
exhaustive-match shape D-161 used for `Divide`.

**Rationale.** A truncation answers a different question correctly, with no
fault, no log line and no wrong-looking value, which is the worst shape a
boundary can fail in. A wrong claim in a doc comment is worse than a missing
test, because it is a reason not to write one.

**Consequences.** The one machine check covering all three submits a valid low
half with a garbage high half, so a cast produces a **plausible** answer rather
than an absurd one; an input where both readings look wrong would catch
nothing.
