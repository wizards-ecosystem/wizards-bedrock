# D-239 The arriving bytes say which numbering they are in

Status: Accepted, 2026-08-29. Supersedes the arbiter of D-178 and D-179; advances PW-003 and PW-006.

**Context.** PW-003's human VMware round decoded every key wrongly. `a` typed a
newline, `b` typed two `m`s, `c` typed two `f`s. Those are set 2's codes for
`a`, `b` and `c` read against set 1's table — `1C` is Enter there, `32` is `M`,
`21` is `F` — and the doubling is set 2's `F0` release prefix, which a set 1
decoder does not know, so it reads the code after it as a second press. The
stream was set 2 and the driver had decided set 1.

D-178 had already met the opposite machine. That VMware reported its
translation bit clear and translated anyway, so the configuration byte lied and
the identity reply — `(0xAB, 0x41)` for a translated MF2 against `(0xAB, 0x83)`
for an honest one — was the thing it could not fake. D-179 made the identity the
arbiter on that evidence. This VMware answers `(0xAB, 0x41)` to the identity
*and* `0x41` to the scancode-set query while delivering set 2, and its
configuration byte is the honest one. Two real machines, each with a different
probe lying, and no static probe correct on both.

The check that should have caught it could not. `decoder_reads_what_arrives`
asked `identity()` and compared the answer with a decoder that `identity()` had
configured — one value re-derived from the source the other came from. It agreed
on every machine, including this one, while certifying a decoder reading set 2
as set 1. Its own doc claimed it tested "the numbering arriving"; it never read
an arriving byte. The registry's neighbouring comment had even recorded why the
blind spot was there: on QEMU the bit and the identity agree, so no gate boot can
tell an arbiter apart from a guess.

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
honest, so the self-test now seeds the numbering *against* the stream before its
translated round and the round can only succeed by reading the bytes. Release
QEMU passed all 215 checks through that path; deciding the numbering and then
not adopting it failed `a translated keystroke decodes to the same key` and
`the keyboard's numbering and the decoder's are the same` together at 213/215,
and the second of those is a check that could not previously fail at all. Its
assertion moved to after the translated keystroke, where a byte has provably
arrived, and the scancode-set query it used to carry is now a diagnostic warn
rather than a claim. Host tests bind both probe bounds to the two tables and
fail if either grows past them; the `F0` rule, the bound and the adoption each
reddened only their own witness and persist as mutations 154 through 156. The
registry is 288. No unsafe operation or wire ABI changed, and the machine-check
count is unchanged at 215.
