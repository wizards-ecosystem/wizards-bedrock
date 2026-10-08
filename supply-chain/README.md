# Supply-chain policy

`inventory.txt` is the reviewed input record. `Cargo.lock` remains the
authority for exact Rust resolution; the inventory adds license conclusions
and `runtime` versus `development` scope, and accounts for every vendored
artifact checked by `cargo xtask verify-vendor`. `foundation.spdx` is generated
from those committed facts and is never edited by hand.

Run `cargo xtask supply-chain` to verify the lock, license policy, vendored
digests and byte-exact SPDX document. After a deliberate inventory update, run
`cargo xtask update-sbom`, inspect the generated diff, then run the verifier.
Both tasks are repository-owned Rust and require neither a Cargo registry cache
nor a network connection.

## Network and qualification boundary

A new checkout needs one hydration step:

```text
cargo fetch --locked
```

Rustup may likewise fetch the toolchain and target pinned by
`rust-toolchain.toml`. Once hydrated, the supported no-network proof is:

```text
$env:CARGO_NET_OFFLINE = "true"
cargo xtask ci
```

Fresh vulnerability data is inherently external. At each phase close and
before public distribution, run `cargo xtask qualify-advisory`. It resolves
`cargo-audit-audit 0.22.2` directly and invalidates any older pass. A first
bounded run populates an empty qualification-local RustSec database; its clean
commit and tree are then frozen for a second `--no-fetch` review of the exact
repository `Cargo.lock`. The review denies warnings and permits no ignored
advisories. Its canonical JSON must have the complete 0.22.2 report schema,
name that database commit, and consistently report no vulnerabilities or
dependency warnings. The checksummed record and bounded canonical transcript
bind the executable digest, database identity, UTC observation, policy, locked
bytes and result. `cargo xtask evidence` includes those artifacts; `cargo xtask
verify-evidence` checks them without a network or scanner. This is deliberately
exceptional, as are Java and VMware: ordinary CI neither downloads an advisory
database nor claims that an absent scanner passed.

## Distribution

A bare `bedrock.img` is a build artifact. A public distribution must carry
`foundation.spdx`, the repository `LICENSE`, `THIRD-PARTY-NOTICES.md` and the
`licenses/` directory beside it. The runtime rows are the shipped Rust and
boot inputs. Development rows affect builds or verification but do not reach
the disk image.

Each runtime row has exactly one entry in `THIRD-PARTY-NOTICES.md`, and that entry
opens with the row's name and version as two backticked fields. `cargo xtask
supply-chain` compares those entries with the runtime rows in both directions
(D-345).
