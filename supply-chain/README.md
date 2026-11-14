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

Fresh vulnerability data is inherently external, and no task fetches it:
ordinary CI neither downloads an advisory database nor claims that an absent
scanner passed. The maintainer may run `cargo audit` by hand against the exact
`Cargo.lock`; nothing records the result. The `qualify-advisory` task and its
record were removed by D-454. D-441's experimental source preview requires
disclosure/license review, the bounded source/build checks and explicit
unrun-check disclosures; it distributes no newly built image or evidence
bundle and makes no fresh advisory-review claim.

`cargo xtask evidence` (format v12) binds the source commit and tree, the
locked `Cargo.lock` and toolchain bytes, the tool versions and the two
reproduced kernel and image artifacts. It carries no advisory, Java or VMware
record. `cargo xtask verify-evidence` checks the bundle without a network.

## Distribution

Experimental source previews retain `LICENSE`, this inventory and SPDX,
third-party notices, upstream license texts and reviewed vendored build inputs
in the tagged tree. First-party MIT does not relicense third-party material.

A bare `bedrock.img` is a build artifact. A qualified binary distribution must carry
`foundation.spdx`, the repository `LICENSE`, `THIRD-PARTY-NOTICES.md` and the
`licenses/` directory beside it. The runtime rows are the shipped Rust and
boot inputs. Development rows affect builds or verification but do not reach
the disk image.

Each runtime row has exactly one entry in `THIRD-PARTY-NOTICES.md`, and that entry
opens with the row's name and version as two backticked fields. `cargo xtask
supply-chain` compares those entries with the runtime rows in both directions
(D-345).
