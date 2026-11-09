# D-174 The Phase 6 audit: one defect in three places, and a claim nothing checked

Status: Accepted (2026-08-25)

**Context.** D-77's pattern for a phase close: treat every green check as a
claim to attack. Phase 6 added four `libs/` crates, six kernel modules, two
opcodes that take a *pointer-shaped* argument from ring 3, and an ABI entry
that doubled in size. The sweep looked for the classes the phase had already
been bitten by — a check that restates its data, a check that cannot reach its
code, an input where the answers coincide — and for the class a storage phase
adds: a value ring 3 chooses being read as something else.

**What it found.**

**Three silent truncations, and they are one defect.** Every word a submission
carries is written by ring 3 into a page ring 3 owns, and three of them reached
the kernel through an `as` cast:

- A **rights word**: `Rights::from_bits(submission.argument as u32)`. The cast
  drops the high half, so `0xFFFF_FFFF_0000_0003` was read as `DUPLICATE |
  TRANSFER` rather than refused — which defeats the entire reason `from_bits`
  returns an `Option`. A program could set every undefined bit in the world and
  be told yes.
- A **file offset**: `submission.offset as u32`. An offset of four billion and
  one wrapped to one, so a program asking for a byte past every file this
  kernel can hold was handed a byte *inside* it.
- A **cancellation token**: `submission.argument as u32`. `0x1_0000_DEAD` would
  end the operation tagged `0xDEAD` — a program ending an operation it did not
  name.

None is a crash. All three are a program being answered about something other
than what it asked for, which is the worst shape a boundary can fail in: there
is no fault, no log line and no wrong-looking value, only a correct answer to a
different question.

All three are now `try_from`. The rights word and the token are **refused**;
the offset is clamped to `u32::MAX`, which is past every file this kernel can
hold and therefore reads nothing — which is what an offset past the end reads
everywhere else here.

**A doc comment claimed a test that did not exist.** `devfs::DEVICES` said it
was "checked against [the enum] by a test that counts (D-161)". There was no
such test. The compiler does check the match in `Device::name`, so a third
variant is a build failure *there* — and the array would go on listing two,
which would make the listing incomplete and every check that counts devices
agree with it. The test exists now, and it is the same exhaustive-match shape
D-161 used for `Divide`.

That is the more uncomfortable finding. A wrong claim in a doc comment is worse
than a missing test, because it is a reason not to write one.

**What it did not find.** No panicking call in any of the phase's production
code — `libs/fat32`, `libs/path-walk`, `libs/nvme`, `libs/pci-config` and the
six kernel modules have none outside tests and doctests. No reachable indexing
that is not behind a proven bound. No lock taken under a leaf guard that the
machine check did not already catch during the slice that introduced it.

**Consequences.**

- Three fixes, one machine check covering all three, three deliberate breaks —
  each of which reddens it.
- The check is written from the *claim*: it submits a valid low half with a
  garbage high half, so a cast produces a **plausible** answer rather than an
  absurd one. An input where the truncated and untruncated readings both look
  wrong would not have caught anything.
- `devfs` gains the test its documentation promised.
- The pattern for the next phase: **every field a submission carries is a value
  ring 3 chose.** `as` is not a conversion at that boundary; it is a silent
  reinterpretation, and the only casts that belong there are widening ones.
