# D-179 The set the keyboard speaks is a fact about the controller

Status: Accepted (2026-08-25)

**Context.** D-178 established that VMware's controller translates set 2 into
set 1 while reporting its translation bit clear. `libs/ps2-keys` is a set 2
crate by construction — its first line says so — so on that machine every
keystroke reached a decoder reading the wrong numbering, and typing did
nothing. That was the symptom this whole sequence started from.

**Options.**

*Turn translation off.* Already attempted: the driver clears the bit at
bring-up and reads it back clear. The controller agrees and translates anyway.
There is nothing to fix on this side.

*Untranslate the bytes.* A filter in front of the existing decoder, mapping set
1 back to set 2 so the tested state machine handles everything. Attractive
until Pause: set 1 spells it in six bytes and set 2 in eight, so the filter
cannot be byte-local and would need its own state machine — the thing it exists
to avoid. It also needs a byte-to-byte table with nothing to check it against.

*Decode set 1.* A second table and a second state machine, sharing the key
identities, the shift state and the layout — which is all the two sets have in
common.

**Decision.** Decode set 1. `Decoder::translated()` beside `Decoder::new()`,
`set1::key_for` beside `table::key_for`, and the driver picks by asking the
keyboard who it is at bring-up rather than by reading the configuration byte.

**A second transcription is worth more than two transcriptions**, and that is
the argument for this shape rather than the filter's. Set 1 and set 2 are
different numbers for the same hundred-odd keys, so the two tables check each
other: `both_sets_name_the_same_keys` fails if either drops a key, duplicates
an arm so the one below is shadowed, or files a key in the base table that
belongs behind `E0`. There is no third table to ask, and this is the nearest
thing to one. D-156 had to resolve a single backslash against two sources by
hand; that entry is now an assertion, and the other hundred are covered by
construction.

**`0xAA` is a key, not a reply.** The one genuine collision: it is the
self-test result and it is `0x2A | 0x80`, the left shift's release. Swallowing
it would leave shift held down for ever after the first capital letter.
Decided in favour of the key, because the self-test result is read by the reset
path directly, before scanning is enabled, where no decoder can see it. Set 2
keeps swallowing it, where nothing else claims that byte.

**Rationale.** The alternative was to leave VMware's keyboard dead and note it.
D-18 makes that machine the definition of done for every phase, and a boot
where the machine accepts keystrokes and does nothing with them is not a
definition of done — it is the thing a person sees first and concludes the
kernel is broken from, which is exactly what happened.

**Consequences.**

- `libs/ps2-keys` decodes either set. Two new tables, one new state machine,
  eleven new host tests; the layout, the shift state and the event type are
  untouched, which is the measure of whether the split was drawn in the right
  place.
- The bring-up reads the identity before enabling scanning, and the boot line
  says which set it is reading rather than asserting set 2.
- A controller that will not answer `0xF2` is taken at the configuration
  byte's word, which is the old behaviour and the right fallback.
- The self-test checks the decoder against a *translating* QEMU, which is the
  only way to run the set 1 path where the expected key is known
  independently. The harness injects in two rounds, and the second round's
  marker is asserted to exist in the kernel's source beside the first — a
  round waiting on a line nothing prints would be red for the harness's reason
  rather than the kernel's.

**And the break found what reading would not.** The translated round first
pressed `a`, the same key the untranslated round presses. With set 1's code for
`a` deliberately wrong the check stayed **green**: an `a` still in the queue
from the first round satisfied the wait, and the set 1 tables were never
consulted. One character's difference — the round presses `b` now — turned the
same break red.

This is D-155's rule (*choose inputs where the answers differ*) and D-165's (*a
check must reach the code it is about*) arriving together, in the phase that
recorded both. The input was not chosen to make the two paths differ; it was
chosen because the first round already had one, which is the same reasoning that
put `0x1000` in four paging checks two decisions ago. **Reusing a fixture is
how a check stops reaching its subject**, and neither reading the check nor
running it green would have said so.
