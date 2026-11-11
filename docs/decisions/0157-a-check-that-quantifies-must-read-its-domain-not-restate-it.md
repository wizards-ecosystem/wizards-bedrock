# D-157 A check that quantifies must read its domain, not restate it

Status: Accepted, 2026-08-25.

**Decision.** **If a check's name begins "every", "all" or "no", its body must
enumerate the domain rather than name its members.** Two checks that iterated a
list copied from their subject now derive the domain from the source of truth:

- `every_invariant_the_model_defines_is_checked` guards D-148 (TLC checks only
  the names in `INVARIANTS`, and an empty block passes having checked nothing).
  It reads `Ring.tla`, takes every top-level definition, subtracts the
  specification's own machinery (`vars`, `Init`, `Next`, `Spec`, and every
  action `Next` disjoins, itself parsed out of `Next`) and requires the rest to
  be named by the config. It reads the config's block in the other direction
  too, so a name checked but not defined is caught, and it refuses an empty
  block.
- `every_task_the_parser_accepts_is_named_in_the_help` reads its own source
  through `include_str!("cli.rs")` and takes the task names out of the parser's
  match arms.

A third check requires `models/` to hold the model and nothing that is not
ignored. It is filesystem-based rather than `git ls-files`, because the gate
does not shell out to git anywhere and should not start here.

**Rationale.** A list restated beside its source is not evidence about the
source: it is a second copy with a test-shaped wrapper, and it passes on the day
the two drift apart, which is the failure it was written to catch. This is
D-124's "write the check from the claim, not the implementation" one level up:
there restating the code passes trivially, here restating the data does. The
tell is the same: the check contains a copy of its subject. `vendor.rs` is the
right shape: it walks `boot/` and `tools/` for `CHECKSUMS` files, cross-checks
both directions and asserts the counts match. The two checks that were wrong
were the two that had a list.

**Consequences.**

- `.gitignore` covered `models/*_TTrace_*.tla` and not the `.bin` TLC writes
  beside it; the rule is extended and the committed traces are untracked.
- `005`'s testing section carries the rule.
