# D-144 The save area was sized for one form and written by another's dispatch

Status: Accepted, 2026-08-25.

**Decision.** `xstate::init`'s `BYTES` (the size a save area needs) and `FORM`
(the word `save` dispatches on) are checked **by consequence**:
`save_stays_inside_its_area` puts a sentinel past `bytes()` and requires the
save not to touch it. That catches a disagreement in either direction, and a
`BYTES` too small for a reason nobody has thought of yet. It allocates a 4 KiB
aligned area on the stack and refuses if `bytes()` plus the sentinel would not
fit, so a future `XCR0` that grew past it reports rather than overflowing the
check itself.

**The property that holds on every processor is that the area was sized for the
form in use.** A check requiring the compacted form is a check about the
*hardware*: QEMU's `-cpu max` reports no `XSAVEC`, so it is red on correct
hardware there. `xstate::saves_compacted` exists for a diagnostic, and the check
deliberately does not use it.

**Rationale.** `FORM` and `BYTES` derive from the same `CPUID` walk, and the
banner reads a third copy, so any check comparing them to each other compares
two copies of one decision (D-124). A mismatch writes **past the end of every
save area on the machine** while the banner announces the right thing. The
sentinel is outside that circle. A check that is red on correct hardware is
worse than no check, because the response to it is to weaken it.

**Consequences.** Since D-135 put the self-test into the VMware image, this is
the first thing that will exercise the compacted path with a check watching it.

**Left owed.** The sentinel proves the save did not write past `bytes()`. It
does not prove the save wrote *everything it should have*: a `FORM` of zero
would write nothing at all and pass. `is_enabled()` covers that today because
it reads the same word; a genuinely independent check would compare a restored
register against what was saved, which
`a_vector_register_survives_a_switch` already does for `xmm0` and for nothing
else.
