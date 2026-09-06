Vendored TLA+ tools

- `tla2tools.jar`: the TLA+ tools, including the TLC model checker, from the
  [TLA+ v1.8.0 release](https://github.com/tlaplus/tlaplus/releases/tag/v1.8.0)
  (MIT, see the project's own LICENSE in the jar).
- Fetched 2026-08-24. TLC reports itself as `2026.08.21.155922 (rev: 9787e65)`.
- `CHECKSUMS` pins its SHA-256, and `xtask/src/model.rs` carries the same digest
  so the check refuses to run against a jar it was not pinned to. TLC is a tool
  that decides whether a design is correct, so a silently swapped one is a
  silently swapped answer (D-25, D-112).

It runs both correct models in `models/`, then requires every adjacent mutant
configuration to violate its exact named invariant:

```
cargo xtask model-check      # needs a JVM on PATH
```

**Not in `ci`, deliberately.** It needs a JVM, which is the one tool this
project cannot vendor into a Rust build, and a gate step that fails on a
machine without Java is a gate that teaches people to skip it. It is a named
task the phase runs: the same treatment `vmware` gets, and for the same
reason.

To update: download a newer `tla2tools.jar`, regenerate the digest, and update
`CHECKSUMS`, `JAR_DIGEST` in `xtask/src/model.rs`, and this note in the same
commit so the change is reviewable.

```
cargo xtask verify-vendor    # after updating, must pass
cargo xtask model-check      # correct models clean; deliberate mutants exact
```
