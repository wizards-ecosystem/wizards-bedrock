# D-308 Transcript witnesses preserve every contract except the one under test

Status: Accepted, 2026-09-12. Amends D-244 and D-301; advances PW-001.

**Decision.** Duplicate one complete canonical info record, place invalid UTF-8
inside an otherwise complete additional record, and observe the explicit NUL
check immediately before general canonical-text validation. The NUL and control
checks remain independent: the former rejects a format-significant sentinel
explicitly, while the latter closes the complete control-character class.

**Rationale.** Three survivors weakened canonical VMware transcript validation,
but each old fixture was refused outside the check its mutation removed: the
duplicate messages shared one line, the NUL was also a control character, and
the invalid byte followed the terminal newline.

**Alternatives.** Dropping either redundant character check was rejected
because a masked witness does not make its production defense unnecessary.
Combining message text on one line and appending poison after the transcript
terminator were rejected because those fixtures violate unrelated grammar
before reaching multiplicity or decoding policy.
