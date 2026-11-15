# Vendored TLA+ tools

- `tla2tools.jar`: the TLA+ tools and the TLC model checker from the
  [v1.8.0 release](https://github.com/tlaplus/tlaplus/releases/tag/v1.8.0), fetched 2026-08-24
  (MIT, see the LICENSE inside the jar).
- `CHECKSUMS`: its SHA-256. `cargo xtask model-check` refuses a jar that does not match.

`model-check` runs both correct models in `models/` and requires each mutant configuration to
violate its named invariant. It needs a JVM on `PATH`, so it is not part of `cargo xtask ci`.

To update, download a newer `tla2tools.jar` and update `CHECKSUMS` and this note in the same
commit.

```
cargo xtask verify-vendor    # the jar matches CHECKSUMS
cargo xtask model-check      # correct models clean, every mutant caught
```
