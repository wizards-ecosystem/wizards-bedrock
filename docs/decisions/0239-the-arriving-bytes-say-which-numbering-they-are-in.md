# D-239 The arriving bytes say which numbering they are in

Status: Accepted, 2026-08-29. Supersedes the arbiter of D-178 and D-179; advances PW-003 and PW-006.

**Decision.** The stream decides. `libs/ps2-keys::SetProbe` reads the numbering
off the bytes, and the two rules are read from the tables rather than chosen:
set 2 prefixes every release with `F0` and has no `70` key to make `F0` a set 1
release, so `F0` means set 2 wherever it appears; set 1's makes stop at `5D` so
its releases run to `DD`, while set 2's makes stop at `83`, so a byte above `83`
that is not a protocol reply means set 1. `AA` is excluded because both sets can
send it.

The identity supplies the opening guess and nothing more. Until a decisive byte
arrives the driver *holds* what it has read, because decoding a held byte under
the wrong numbering names a different key; the decision is one keystroke away
and the held bytes are then replayed through the right decoder. Changing
translation re-opens the question rather than re-answering it from the bit that
was written. A keystroke's worth of bytes that distinguishes nothing keeps the
opening guess, so no input is held for ever.

**Rationale.** PW-003's human VMware round decoded set 2 bytes against set 1's
table. D-178's VMware had reported its translation bit clear and translated
anyway, so D-179 made the identity reply the arbiter; this VMware answers the
translated identity `(0xAB, 0x41)` and `0x41` to the scancode-set query while
delivering set 2, with an honest configuration byte. Two real machines, each
with a different probe lying, leave no static probe correct on both.
`decoder_reads_what_arrives` compared `identity()` with a decoder `identity()`
had configured, so it agreed on every machine and never read an arriving byte;
on QEMU the bit and the identity agree, so no gate boot could tell an arbiter
from a guess.

**Alternatives.** Trusting the configuration byte was rejected because D-178
measured it lying. Keeping the identity was rejected for the same reason in the
other direction. Trusting whichever two of the three agree was rejected because
on each machine the two that agree are the two that are wrong together. Deciding
without holding was rejected because it mis-decodes the first keystroke, which
is the one the self-test's human rounds press. Widening `Decoder::feed` to
return several events was rejected as an API change to pure logic that is
already correct for both sets: the ambiguity is the driver's, and so is the
buffer.

**Consequences.** The correction is invisible on any machine whose probes are
honest, so the self-test seeds the numbering *against* the stream before its
translated round and the round can only succeed by reading the bytes. Deciding
the numbering and then not adopting it fails `a translated keystroke decodes to
the same key` and `the keyboard's numbering and the decoder's are the same`
together, and the second could not previously fail at all. Its assertion moved
to after the translated keystroke, where a byte has provably arrived, and the
scancode-set query it used to carry is now a diagnostic warn rather than a
claim. Host tests bind both probe bounds to the two tables and fail if either
grows past them; mutations 154 through 156 keep the `F0` rule, the bound and
the adoption red.
