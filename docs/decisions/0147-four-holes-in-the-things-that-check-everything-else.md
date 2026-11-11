# D-147 Four holes in the things that check everything else

Status: Accepted, 2026-08-25.

**Decision.** Four holes in what runs the checks are checked, because a defect
in a check costs one property and a defect in what runs the checks costs all of
them:

- **The crate boundary is asserted as text.** Every `libs/` crate's `lib.rs`
  must contain `#![forbid(unsafe_code)]`, `kernel/src/main.rs` must `deny`, and
  `wost/src/lib.rs` must `allow`, the last because it is what makes the other
  two mean anything. Counting unsafe **blocks** cannot see an `unsafe fn` or
  `unsafe impl` in a crate that dropped the attribute, and D-13's claim is that
  `wost` is the only crate that may write unsafe.
- **`ci`'s body must contain all of its steps**, read from `tasks.rs`'s source
  the way the boot count already is. Each step is also its own task, so a step
  leaving `ci` (D-26's only gate) left every host test green.
- **`.ld` joins the extensions `decisions.rs` scans**, because the linker script
  cites decisions.
- **`boot_reports` splits the failed-check list and compares whole elements.**
  A substring match let a check whose name contains another's satisfy it,
  D-134's hole re-entered one layer down.

**Rationale.** Three of these are the same shape as the defects they guard
against: the unsafe counter measured a consequence of the boundary and was
taken for the boundary, `ci`'s coverage was inferred from its steps existing as
tasks, and the substring match was a *description* of the check name rather
than the name. Text rather than a build for the boundary, because
`#![forbid(unsafe_code)]` proves itself at compile time *only while something
would violate it*: the attribute is a statement about the future, so it is
checked as a statement.

**Consequences.** `ci`'s step list is stated in two places, `tasks.rs` and the
test, and they must agree. That is a transcription, and it is the right trade
for the only gate there is.

**Left owed.** `ci`'s step list in the test will need updating when a step is
added, which is a reviewable line in a commit, the same mechanism as `MEASURED`
and `ABSENT`. Deriving it instead would mean parsing Rust, which is the thing
this project keeps declining to do for good reasons.

The audit's remaining Tier 3 findings (`ps2-keys` anchoring 21 of ~89
scancodes, `exception::name` never reading `PAGE_FAULT` back,
`ring_proto::VERSION` on both sides of every comparison, the TLA+ config's
unread `INVARIANTS` block) are the D-136 and D-142 family and are not closed
here.
