# D-155 Two checks whose inputs made the property invisible

Status: Accepted, 2026-08-25.

**Decision.** A check is given inputs where the possible answers differ. Two
Tier 3 findings of the Phase 0 to 5 audit were checks that passed and could not
fail, because every input made the right and wrong answers identical:

- `FramebufferLayout` was only tested with `pitch == width * bytes_per_pixel`,
  so computing the span from `row_bytes` survived. On a padded framebuffer,
  most real hardware, that maps short by `(pitch - row_bytes) * height` bytes.
  The test now uses `1024×768` at a pitch of 4352 against a row of 4096 and
  asserts the span is the pitch's *and* is not the visible width's.
- `Timebase::new` rounds its fixed-point multiplier to nearest with
  `+ denominator / 2`, and `+ denominator - 1` passed every test. The test now
  checks the claim rather than the trick: for six real frequencies it computes
  both integer neighbours of the exact ratio and requires the multiplier to be
  the closer one, which catches round-up, round-down and anything else.

**Rationale.** **A test input that is realistic can be worse than one that is
contrived.** An unpadded framebuffer is a real configuration and a useless test
value; a frequency whose ratio lands nowhere near a halfway point proves
nothing about rounding. This is the same defect as D-142's
`TYPICAL = 0xFEE0_0900`, which set both flag bits and so could not tell them
apart. D-136's "pairwise distinct" instruction is the same idea from the other
end, and `005`'s testing section should carry both.

**Consequences.**

- Both host tests were broken once with the exact mutation the audit named. No
  production code changed: both computations were correct and neither could be
  shown to be.
- The audit's Tier 3 list is closed apart from `libs/ps2-keys`, which anchors
  21 of about 89 base scancodes. A swap of two unanchored keys survives, and the
  fix is to transcribe set 2 from a source rather than choose better inputs:
  the D-142 treatment.
