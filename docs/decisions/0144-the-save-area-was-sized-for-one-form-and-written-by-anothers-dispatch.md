# D-144 The save area was sized for one form and written by another's dispatch

Status: Accepted (2026-08-25)

**Context.** `xstate::init` decides whether this processor gets the compacted
`XSAVEC` path or the plain `XSAVE` one, then stores two things: `BYTES`, the
size a save area needs, and `FORM`, the word `save` dispatches on.

Nothing read `FORM`. `main.rs` prints ", compacted" from
`Extended::compacted` — a `CPUID` bool returned by `init` — not from the
dispatch. So storing `FORM = 1` with a compacted `BYTES` writes **past the end
of every save area on the machine**, each of which lives inside a thread's
slot, while the banner goes on announcing the right thing.

They are stored on adjacent lines and agree. What was missing was any way to
find out if they stopped.

**Decision.** Checked **by consequence**: `save_stays_inside_its_area` puts a
sentinel past `bytes()` and requires the save not to touch it. That catches the
disagreement in either direction, and would catch a `BYTES` that was too small
for a reason nobody has thought of yet.

**The first version asserted the form, and was wrong.** It required
`saves_compacted()`, and went red on the gate: QEMU's `-cpu max` reports no
`XSAVEC`, so the kernel correctly chooses the uncompacted path.
`AGENTS.md` names the compacted path as one of the things only the VMware boot
reaches — so a check requiring it is a check about the *hardware*, green on one
machine and red on the other.

That is worth more than the fix. **The property that holds on every processor
is that the area was sized for the form in use** — and a check written that way
is meaningful on both. Since D-135 put the self-test into the VMware image,
this is the first thing that will exercise the compacted path with a check
watching it.

**Rationale.** This is D-124's rule — measure the thing, not the description —
applied where the description was a *flag the code sets about itself*. `FORM`
and `BYTES` are both derived from the same `CPUID` walk, so any check comparing
them to each other, or either to what `init` returned, is comparing two copies
of one decision. The sentinel is outside that circle.

The near-miss is the instructive part: asserting `saves_compacted()` would have
passed review, read like a stronger check, and encoded QEMU's absence of a
feature as a kernel invariant. A check that is red on correct hardware is worse
than no check, because the response to it is to weaken it.

**Consequences.**

- `xstate::saves_compacted` exists and is read by nothing but a diagnostic; the
  check does not use it, deliberately.
- `save_stays_inside_its_area` allocates a 4 KiB aligned area on the stack and
  refuses if `bytes()` plus the sentinel would not fit — so a future `XCR0`
  that grew past it reports rather than overflowing the check itself.
- 144 checks. Halving `BYTES` turns it red.

**Left owed.** The sentinel proves the save did not write past `bytes()`. It
does not prove the save wrote *everything it should have* — a `FORM` of zero
would write nothing at all and pass. `is_enabled()` covers that today because
it reads the same word; a genuinely independent check would compare a restored
register against what was saved, which
`a_vector_register_survives_a_switch` already does for `xmm0` and for nothing
else.
