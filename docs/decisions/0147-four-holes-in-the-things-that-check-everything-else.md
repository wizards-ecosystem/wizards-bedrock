# D-147 Four holes in the things that check everything else

Status: Accepted (2026-08-25)

**Context.** The audit's remaining findings about the *gate*. A defect in a
check costs one property; a defect in what runs the checks costs all of them,
which is why these are worth an entry after the code is already fixed.

**The crate boundary was inferred, never asserted.** `no_libs_crate_has_any_unsafe`
measures unsafe **blocks** and requires zero. That is right for the budget —
`unsafe fn` and `unsafe impl` are declarations, not operations. But it leaves
the boundary itself unchecked: a `libs/` crate that dropped
`#![forbid(unsafe_code)]` and grew an `unsafe fn` has zero blocks and passes.
D-13's framekernel claim is that `wost` is the only crate that may write
unsafe, and nothing tested that the other nineteen still said so.

**`ci` could quietly lose a step.** Delete `vendor::verify(paths)?` from its
body and every host test stays green, because each step is also its own task
and the tests exercise the tasks. `ci` is the only gate this project has
(D-26); a step leaving it is the gate shrinking with nothing to say so. Only
the *boot count* was checked, and only because the README states it.

**`kernel/linker.ld` cites two decisions and was scanned by nothing.**
`decisions.rs` walked `.rs`, `.md`, `.toml`, `.h`, `.cpp`, `.tla` and `.cfg`.
The linker script names D-45 and D-21, and is the file whose properties D-40's
read-back exists to check.

**And D-134's own defect had a second door.** `boot_reports` matched the failed
check's name as a **substring** of the `"; "`-joined list. A future check whose
name contains another's would satisfy that other one — which is exactly the
"caught by something, not by its own check" hole D-134 was written to close,
re-entered one layer down.

**Decision.** Each is now checked:

- Every `libs/` crate's `lib.rs` must contain `#![forbid(unsafe_code)]`,
  `kernel/src/main.rs` must `deny`, and `wost/src/lib.rs` must `allow` — the
  last because it is what makes the other two mean anything. Checked as text:
  the attribute is the assertion, and a build that compiles proves only that
  nothing has violated it *yet*.
- `ci`'s body must contain all nine of its steps, read from `tasks.rs`'s source
  the way the boot count already is.
- `.ld` joins the scanned extensions.
- `boot_reports` splits the list and compares whole elements.

All four broken once and watched go red.

**Rationale.** Three of these are the same shape as the defects they guard
against, which is the observation worth keeping. The unsafe counter measures a
consequence of the boundary and was taken for the boundary. `ci`'s coverage was
inferred from its steps existing as tasks. The mutation runner's substring match
was a *description* of the check name rather than the name.

The fourth is different and simpler: a file extension nobody added to a list.
It is here because the cost of finding it was an audit, and the cost of
preventing it was one word.

Why text rather than a build for the boundary: `#![forbid(unsafe_code)]` proves
itself at compile time *only while something would violate it*. A crate with the
attribute removed and no unsafe compiles identically, and the next author adds
the unsafe. The attribute is a statement about the future, so it is checked as a
statement.

**Consequences.**

- Four host tests, all milliseconds, all in the gate.
- `ci`'s step list is now stated in two places — `tasks.rs` and the test — and
  they must agree. That is a transcription, and it is the right trade for the
  only gate there is.
- The mutation runner compares whole names. The 29-mutation sweep is unchanged
  by it, which is the expected result and not evidence the fix was unnecessary.

**Left owed.** `ci`'s step list in the test is a transcription and will need
updating when a step is added — which is a reviewable line in a commit, the
same mechanism as `MEASURED` and `ABSENT`. Deriving it instead would mean
parsing Rust, which is the thing this project keeps declining to do for good
reasons.

The audit's remaining Tier 3 findings — `ps2-keys` anchoring 21 of ~89
scancodes, `exception::name` never reading `PAGE_FAULT` back,
`ring_proto::VERSION` on both sides of every comparison, the TLA+ config's
unread `INVARIANTS` block — are the D-136 and D-142 family and are not closed
here.
