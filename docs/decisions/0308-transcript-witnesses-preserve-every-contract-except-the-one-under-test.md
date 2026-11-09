# D-308 Transcript witnesses preserve every contract except the one under test

Status: Accepted (2026-09-12), amends D-244 and D-301; advances PW-001

**Context.** Complete-sweep mutations 122, 126 and 127 weakened canonical VMware
transcript validation but survived one integrated test. The duplicate-marker
fixture placed two messages on one line, yielding zero canonical occurrences
instead of two. The NUL fixture was also rejected by the following general
control-character check. The invalid UTF-8 byte followed the terminal newline,
so lossy decoding still produced text without a terminal newline. Each broad
error assertion was satisfied outside the check its mutation removed.

**Decision.** Duplicate one complete canonical info record, place invalid UTF-8
inside an otherwise complete additional record, and observe the explicit NUL
check immediately before general canonical-text validation. The NUL and control
checks remain independent: the former rejects a format-significant sentinel
explicitly, while the latter closes the complete control-character class.

Dropping either redundant character check was rejected because a masked witness
does not make its production defense unnecessary. Combining marker text on one
line and appending poison after the transcript terminator were rejected because
those fixtures violate unrelated grammar before reaching multiplicity or
decoding policy.

**Proof.** Accepting any nonempty marker population made the canonical duplicate
record pass and failed
`a_vmware_selftest_transcript_is_complete_current_and_unambiguous`. Replacing
the NUL predicate with true made
`a_nul_transcript_is_rejected_before_canonical_text_validation` reach its next
layer and fail. Replacing strict UTF-8 with lossy decoding accepted the interior
invalid byte and failed the integrated self-test transcript witness. All three
exact tests pass with production checks restored; registry source and check-name
proofs pass. The remaining report outcomes and a clean successor sweep stay in
PW-001.
