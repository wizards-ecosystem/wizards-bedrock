# D-156 The scancode table is transcribed, and one code is deliberately unanchored

Status: Accepted, 2026-08-25.

**Decision.** `libs/ps2-keys`' set 2 table, about eighty-nine make codes of
which twenty-one were anchored, is checked by transcription rather than a better
input (D-142's treatment). Sixty-eight single-byte make codes are written out as
literals from Andries Brouwer's *Keyboard scancodes* §10, the set 2 table
Linux's own documentation points at, and asserted in both directions: every code
names its key, and no two codes name the same key, since a duplicate is a
transcription error the first loop cannot see.

The sixty-ninth, `\`, is left unanchored. The source read `0x5C`; the table has
`0x5D`, mainstream references agree with the table, and no second primary
source could be reached. Asserting `0x5D` restates the definition, the defect
this test exists to remove (D-124); asserting `0x5C` asserts a guess against
working code. **Sixty-eight anchored with the gap named beats sixty-nine with
one invented.** The test comment states the disagreement, so the next person
with a second source knows what to check and what it would settle.

**Rationale.** A table checked against itself agrees with itself, and the only
way to learn that two documents disagree is to compare them; the transcription
turned an invisible uncertainty into a written one. It sets the rule for later
transcriptions: **where the source and the code disagree and neither can be
confirmed, anchor neither and record the conflict.** A claim nobody can check is
worth less than no claim.

**Consequences.**

- Swapping `0x33`/`0x34`, which would exchange H and G on every keyboard, now
  turns the test red. The audit's Tier 3 list is closed.
- **Left owed.** `\`'s make code, against a second primary source: the IBM PS/2
  Technical Reference or a keyboard controller datasheet, before a keyboard
  layout depends on it. Extended codes behind an `E0` prefix (the arrow keys,
  the navigation cluster and the keypad) are not transcribed here, and the same
  treatment applies to them.
