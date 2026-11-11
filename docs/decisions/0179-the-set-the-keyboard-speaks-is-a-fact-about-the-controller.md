# D-179 The set the keyboard speaks is a fact about the controller

Status: Accepted, 2026-08-25.

**Decision.** Decode set 1. D-178 established that VMware's controller
translates set 2 into set 1 while reporting its translation bit clear, so
`libs/ps2-keys`, a set 2 crate, received the wrong numbering and typing did
nothing. `Decoder::translated()` sits beside `Decoder::new()` and
`set1::key_for` beside `table::key_for`; the two share the key identities, the
shift state and the layout. The driver picks by asking the keyboard who it is at
bring-up rather than by reading the configuration byte. A controller that will
not answer `0xF2` is taken at the configuration byte's word, the old behaviour
and the right fallback.

Turning translation off was attempted: the driver clears the bit and reads it
back clear, and the controller translates anyway. A filter untranslating set 1
to set 2 cannot be byte-local, because set 1 spells Pause in six bytes and set 2
in eight, so it would need its own state machine and an unchecked table.

**A second transcription is worth more than two transcriptions.** Set 1 and set
2 are different numbers for the same keys, so the tables check each other:
`both_sets_name_the_same_keys` fails if either drops a key, duplicates an arm so
the one below is shadowed, or files a key in the base table that belongs behind
`E0`. D-156 had to resolve a single backslash against two sources by hand; that
is now an assertion.

**`0xAA` is a key, not a reply** in set 1: it is the self-test result and also
`0x2A | 0x80`, the left shift's release, and swallowing it would leave shift held
for ever after the first capital. The self-test result is read by the reset path
directly, before scanning is enabled, where no decoder sees it. Set 2 keeps
swallowing it, where nothing else claims that byte.

**Rationale.** D-18 makes VMware the definition of done, and a machine that
accepts keystrokes and does nothing with them is what a person sees first and
concludes the kernel is broken from.

**Consequences.**

- The bring-up reads the identity before enabling scanning, and the boot line
  says which set it is reading.
- The self-test checks the decoder against a *translating* QEMU, the only way to
  run the set 1 path where the expected key is known independently. The harness
  injects in two rounds, and the second round's wait line is asserted to exist
  in the kernel's source, so a round waiting on a line nothing prints is not red
  for the harness's reason.
- The translated round presses `b`, not the first round's `a`: with `a`, a
  leftover key from the first round satisfied the wait and set 1's tables were
  never consulted. This is D-155's rule (*choose inputs where the answers
  differ*) and D-165's (*a check must reach the code it is about*) together.
  **Reusing a fixture is how a check stops reaching its subject.**
