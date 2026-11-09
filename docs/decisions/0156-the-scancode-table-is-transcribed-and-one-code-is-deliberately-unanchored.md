# D-156 The scancode table is transcribed, and one code is deliberately unanchored

Status: Accepted, 2026-08-25.

**Context.** The last of the Phase 0–5 audit's Tier 3 findings. `libs/ps2-keys`'
set 2 table maps about eighty-nine make codes to keys, and twenty-one were
anchored by a test. A swap of two unanchored codes — `0x33`/`0x34` would
exchange **H and G on every keyboard** — survived the whole suite.

This is D-142's shape and needed D-142's treatment: not a better test input, a
**transcription from the document that fixes the values.**

**Decision.** Sixty-eight single-byte make codes are written out as literals
from Andries Brouwer's *Keyboard scancodes* §10, the set 2 table Linux's own
documentation points at, and asserted in both directions — every code names its
key, and no two codes name the same key, since a duplicate is a transcription
error the first loop cannot see.

**The sixty-ninth is left out, and that is the part worth recording.**

The source read as `\` = `0x5C`. This table has `0x5D`. Mainstream references
agree with the table; a second primary source could not be reached to settle
it, and one summarising fetch is not enough to overturn a value the kernel has
been running on.

Three options and only one is honest:

1. **Assert `0x5D`** — the table's own value. That is the definition restated,
   the exact defect this test exists to remove (D-124).
2. **Assert `0x5C`** — the one source. That is asserting a guess against
   working code, and would have turned the gate red on a value that is probably
   right.
3. **Leave it unanchored and say why.**

Option 3. **Sixty-eight anchored with the gap named beats sixty-nine with one
invented.** The comment states the disagreement, so the next person to reach a
second source knows exactly what to check and what it would settle.

**Rationale.** The transcription found the discrepancy, which is the argument
for doing it at all: a table checked against itself agrees with itself, and the
only way to learn that two documents disagree is to compare them. That the
disagreement is unresolved does not make the exercise a failure — it converted
an invisible uncertainty into a written one.

It also sets the rule for the next transcription, and there will be more:
**where the source and the code disagree and neither can be confirmed, anchor
neither and record the conflict.** A test is a claim, and this project's
standing position is that a claim nobody can check is worth less than no claim.

**Consequences.**

- Sixty-eight codes are pinned in both directions. Swapping `0x33`/`0x34`
  turns it red; it did not before.
- One code is documented as unverified, in the test that would otherwise be
  assumed to cover it.
- The audit's Tier 3 list is closed.

**Left owed.** `\`'s make code, against a second primary source — the IBM PS/2
Technical Reference or a keyboard controller datasheet. Ten minutes for anyone
with either to hand, and worth doing before a keyboard layout depends on it.

Extended codes — those behind an `E0` prefix — are not transcribed here. They
are the arrow keys, the navigation cluster and the keypad, and the same
treatment applies to them.
