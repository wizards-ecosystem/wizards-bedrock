# D-155 Two checks whose inputs made the property invisible

Status: Accepted, 2026-08-25.

**Context.** The last of the Phase 0–5 audit's Tier 3 findings that reach real
behaviour. Both are checks that exist, pass, and cannot fail — not because the
assertion is wrong, but because **every input they were given makes the right
answer and the wrong answer identical**.

**A framebuffer's span was only ever tested unpadded.** `FramebufferLayout`
takes a `width`, a `bytes_per_pixel` and a `pitch`, and `pitch` exists
precisely because rows can be padded. Every test used geometry where `pitch ==
width * bytes_per_pixel`, so the two are interchangeable in all of them, and
`row_bytes.checked_mul(height)` survives the suite. On a padded framebuffer —
which is most real hardware, and the only reason the field exists — that maps
short by `(pitch - row_bytes) * height` bytes and the bottom of the screen is
off the end of the mapping.

**A clock's rounding was only ever tested where rounding did not matter.**
`Timebase::new` computes a fixed-point multiplier with `+ denominator / 2`,
which is round-to-nearest and halves the worst-case error of every conversion
made through it. Change it to `+ denominator - 1` — round *up* — and every
test passes, including a docstring claiming that "losing a bit of the
multiplier breaks it".

**Decision.** Both are given inputs that distinguish the answers.

The framebuffer test uses `1024×768` at a pitch of 4352 against a row of 4096,
and asserts the span is the pitch's *and* is not the visible width's — the
second conjunct being what makes the first mean something.

The clock test checks the **claim** rather than the trick: for six real
frequencies it computes both integer neighbours of the exact ratio and their
distances, and requires the multiplier to be the closer one. That is what
"nearest" means, stated without reference to `+ denominator / 2`, so it catches
round-up, round-down and anything else.

**Rationale.** These are the same defect as D-142's `TYPICAL = 0xFEE0_0900`,
which sets both flag bits and so could not tell them apart. The pattern is
worth stating as a rule now that it has appeared three times:

**A test input that is realistic can be worse than one that is contrived.** A
square framebuffer with no padding is a real configuration and a useless test
value. A frequency whose ratio lands nowhere near a halfway point is a real
clock and proves nothing about rounding. The values a check needs are the ones
where the possible answers *differ*, and those often have to be constructed.

D-136's "pairwise distinct" instruction is the same idea from the other end,
and `005`'s testing section should carry both.

**Consequences.**

- Two host tests, both broken once with the exact mutation the audit named.
- No production code changed. Both computations were correct; neither could be
  shown to be.
- The audit's Tier 3 list is closed apart from `ps2-keys`, which anchors 21 of
  about 89 base scancodes and needs a transcribed table rather than a better
  input.

**Left owed.** `libs/ps2-keys`' scancode table. A swap of two unanchored keys
survives, and the fix is to transcribe set 2 from a source rather than to think
harder about the inputs — which is the D-142 treatment and a session's work, not
an hour's.
