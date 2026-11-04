# register-index (experimental)

Host-only reader for [`docs/016-pre-wizzy-foundation.md`](../../docs/016-pre-wizzy-foundation.md).
It prints the rows that are still open, and it checks that the v0.2.0 close
still leaves PW-006 and PW-010 as the live rows.

This directory is its own Cargo workspace. It is not a member of the kernel
workspace, not a `libs/` crate, and not part of `cargo xtask ci`. The Markdown
register remains the backlog. If this tool and the register disagree, the
register is right and the tool needs an update.

```text
cargo test --manifest-path experiments/register-index/Cargo.toml
cargo run --manifest-path experiments/register-index/Cargo.toml -- --live
```

`--stopped` prints `TRIGGER` and `WIZZY` rows. `--all` prints every parsed
row. The default prints live rows, then stopped rows.
