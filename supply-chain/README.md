# Supply chain

- `inventory.txt`: one `name version license` row for every crates.io package in `Cargo.lock`,
  with the license reviewed for it from the closed list in `xtask/src/supply_chain.rs`.
- `THIRD-PARTY-NOTICES.md`: the notices for the third-party components the shipped image
  contains.
- `licenses/`: upstream license texts those notices cite.

`Cargo.lock` stays the authority for exact resolution. When a dependency change adds, removes or
bumps a locked package, add, remove or edit its inventory row in the same commit, reviewing the
license of anything new.

```
cargo xtask supply-chain     # every package from crates.io or the workspace, each with a current row
```

The check runs without a network once `cargo fetch --locked` has hydrated a new checkout.
