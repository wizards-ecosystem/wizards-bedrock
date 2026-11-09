# D-255 One arena guard owns both handle-slot choice and publication

Status: Accepted, 2026-08-30. Closes PW-027 and D-141's fired condition.

**Context.** PW-027 asked for a deterministic two-claimant schedule only if a
free slot could be selected under one guard and filled under another.

**Decision.** That window is absent. `Arena::claim(&mut self, Entry)` calls the
private `find_free`, reads the generation and installs the entry before its
single `&mut Arena` borrow can end. The only production caller invokes it while
holding the one global arena guard. A scheduler seam would test lock exclusion
again rather than a distinct claim protocol.

**Evidence.** The retained source mutation removes the install from
`Arena::claim`; `filling the arena never repeats a slot` then fails because the
next claim selects the same generation and index. Restoring the install passes
the real arena check. Reintroducing a split claim/fill API is therefore a source
change which must also replace this decision and mutation.
