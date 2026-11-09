# D-157 A check that quantifies must read its domain, not restate it

Status: Accepted, 2026-08-25.

**Context.** A battery run over the finished Phases 0–5 — the gate, TLC, and
all 29 mutations — passed on the first attempt. What it turned up was not in
the kernel but in two of the checks watching it, and both had the same shape:
**a test named for a universal claim whose body iterates a list copied from
the thing it is checking.**

`every_invariant_the_model_defines_is_checked` was written for D-148, whose
whole point is that TLC checks the names in `INVARIANTS` and nothing else — so
an empty block reports "No error has been found" and `cargo xtask model-check`
passes having checked nothing. The test guarded that by looping over five
invariant names written out in an array. It could not see a sixth. A property
added to `Ring.tla` and forgotten in `Ring.cfg` would be unchecked by TLC, and
the test whose name promises otherwise would stay green.

`every_task_the_parser_accepts_is_named_in_the_help` had it worse: seventeen
task names in an array, checked against a parser whose `match` holds the same
seventeen. Two copies of one list, and the test passes on the day they
disagree — because the copy it reads is the copy it was written from.

**Decision.** Both derive the domain from the source of truth.

The model test reads `Ring.tla`, takes every top-level definition, subtracts
the specification's own machinery — `vars`, `Init`, `Next`, `Spec`, and every
action `Next` disjoins, *itself parsed out of `Next`* — and requires whatever
remains to be named by the config. It reads the config's block in the other
direction too, so a name checked but not defined is caught as well, and it
refuses an empty block outright.

The CLI test reads its own source through `include_str!("cli.rs")` and takes
the task names out of the parser's match arms.

**Rationale.** A list restated beside its source is not evidence about the
source. It is a second copy with a test-shaped wrapper, and the failure it
cannot see is exactly the failure it was written to catch: the two drifting
apart. Both of these were sound when written and had already stopped being
sound — the model gained no sixth invariant, but nothing was stopping one.

The rule generalises past these two, which is why it is here rather than in a
commit message: **if a check's name begins "every", "all" or "no", its body
must enumerate the domain rather than name its members.** `vendor.rs` is the
worked example of the right shape — it walks `boot/` and `tools/` for
`CHECKSUMS` files, cross-checks both directions, and asserts the counts match,
so a new vendored directory that nothing verifies is a red test rather than a
silent gap. `tasks.rs` derives from `PASSES`; `qemu.rs` parses the sysdep file
and pins the count at sixteen. The two that were wrong were the two that had a
list.

This is D-124's "write the check from the claim, not the implementation" one
level up. There, restating the code passes trivially; here, restating the
*data* passes trivially. The tell is the same in both: the check contains a
copy of its subject.

**Consequences.**

- Two host tests rewritten to parse their sources. Five deliberate breaks, each
  confirmed red: an invariant the config misses, a config name the model does
  not define, an empty `INVARIANTS` block, an unignored file in `models/`, and
  a parser arm with no help entry.
- A third check added — `models/` holds the model and nothing that is not
  ignored — because the same run found five of TLC's binary counterexample
  traces committed to the repository. `.gitignore` covered
  `models/*_TTrace_*.tla` and not the `.bin` written beside it, and nothing
  looked at that directory. The files are untracked, the rule is extended, and
  the check is filesystem-based rather than `git ls-files` because the gate
  does not shell out to git anywhere and should not start here.
- A sweep of the other fifty-odd universally-named checks found no third
  instance. `tasks.rs`, `qemu.rs`, `vendor.rs` and `paths.rs` all enumerate.
- `005`'s testing section carries the rule.
