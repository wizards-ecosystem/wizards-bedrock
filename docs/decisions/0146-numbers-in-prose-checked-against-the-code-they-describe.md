# D-146 Numbers in prose, checked against the code they describe

Status: Accepted, 2026-08-25.

**Decision.** Thirteen numbers in prose that disagreed with the code beside them
are corrected to what the code does, across `selftest.rs`, `frame-space`, `mm`,
`user.rs`, `program.rs` and `007`. Where a count will keep drifting (host tests
in a growing crate) the prose says "forty-odd" rather than a figure that is
wrong the next time someone adds a test. Where the number is load-bearing, the
answer is not prose at all but a check, as `EXPECTED_CHECKS`, `MEASURED` and,
at the time, D-124's sysdep tally were. D-181 later retired that tally because
its categories were false; the exact per-name register is now the authority.

**A historical number is amended, not rewritten.** D-52 works through sixteen
16 KiB stacks costing 256 KiB; D-113 raised the stack to 32 KiB. An ADR
records what was decided when it was decided, so the numbers stay and the entry
gains an amendment pointer, as D-45 and D-51 carry.

**Rationale.** None would justify a commit on its own, so none was ever fixed,
and thirteen of them is a document that reads like a description and is not
one; `007` charters itself as *"the description of what exists"*. A precise
number in prose is a hostage: right on the day and wrong within a week, made
wrong by someone adding a test, the behaviour it exists to encourage.

**Consequences.** Nothing enforces any of this. The numbers that *are* enforced
are the ones that were made into constants and checked; the rest are prose, and
prose is audited by reading.

**Left owed.** The audit's numeric sweep did not reach `docs/008` to `012`; those
were covered for identifiers only. Counts and sizes in `011` and `012` are
unaudited, and `012` is still untracked.
