# D-146 Numbers in prose, checked against the code they describe

Status: Accepted, 2026-08-25.

**Context.** The prose audit's Tier 2 was a table of thirteen numbers that
disagree with the code beside them. None is a bug. Collectively they are the
reason a reader stops trusting the prose, which is the asset this project has
been spending three days building.

The ones worth naming:

- A wait was "generous beside a **30 ms** slice". `SLICE_NS` is **3 ms** — the
  wait is ten times more generous than advertised, which is the safe direction
  and still wrong.
- `QUIET_MS = 500` was "**five** times the old tick's period". The tick was
  100 Hz, so it is **fifty** — and the line four above already says *"a number
  that would have been fifty"*.
- `FrameSpace` "occupies **2 MiB** of `.bss`". Two arrays of 65,536 `u64` is
  **1 MiB**, which is what `wost/src/mm/mod.rs` calls "one megabyte" eight
  files away.
- "Sixty-four extents of **twenty-four bytes** is **a page and a half** of
  stack". An `Extent` is 32 bytes, so it is 2 KiB — **half** a page. Both
  halves wrong, in opposite directions.
- "**Four** ways to ask for something that is not a run of owned pages, and
  four refusals" — the body asserts **six**.
- The emitted user program was "**twenty-two bytes** of machine code" before it
  grew three conditional branches and two more behaviours; D-116 says "about a
  hundred" of the same artifact.
- `user::spawn` "called **three** times a boot" — there are **six** spawn
  wrappers, self-numbered "The first" through "A sixth".
- "**three** doors into the higher half" — `may_reach` has five call sites and
  `map_owned` is a fourth door.
- `007` gives host-test counts of nineteen, eleven and seventeen for crates
  that now have forty-three, fourteen and thirty-one — and `007` charters
  itself as "the description of what exists".

**And one that is not an error.** D-52 works through an arena of sixteen 16 KiB
stacks costing 256 KiB. D-113 raised the stack to 32 KiB, so it is 512 KiB. An
ADR records what was decided when it was decided; the numbers stay and the
entry gains an amendment pointer, which is what D-45 and D-51 carry beside it
and this one did not.

**Decision.** Every number is corrected to what the code does, except D-52's,
which is amended rather than rewritten. Where a count will keep drifting — host
tests in a growing crate — the prose says "forty-odd" rather than a figure that
is wrong the next time someone adds a test.

**Rationale.** These are individually trivial and that is the point: none would
justify a commit on its own, so none was ever fixed, and thirteen of them is a
document that reads like a description and is not one. The project's own
standard is `007`'s: *"the description of what exists"*. A count is the easiest
claim to check and the first one a sceptical reader tries.

The "forty-odd" choice is deliberate and worth stating. A precise number in
prose is a hostage: it is right on the day and wrong within a week, and the
person who makes it wrong is adding a test, which is the behaviour the number
exists to encourage. An approximation that stays true is worth more than a
precision that does not — **except** where the number is load-bearing, where
the answer is not prose at all but a check, which is what
`EXPECTED_CHECKS`, `MEASURED` and, at the time, D-124's sysdep tally already
were. D-181 later retired that tally because its categories were false; the
exact per-name register is now the authority.

**Consequences.**

- Thirteen corrections across `selftest.rs`, `frame-space`, `mm`, `user.rs`,
  `program.rs` and `007`.
- D-52 carries an amendment pointer to D-113.
- Nothing enforces any of this. The numbers that *are* enforced are the ones
  that were made into constants and checked; the rest are prose, and prose is
  audited by reading.

**Left owed.** The audit's numeric sweep did not reach `docs/008`–`012` — those
were covered for identifiers only. Counts and sizes in `011` and `012` are
unaudited, and `012` is still untracked.
