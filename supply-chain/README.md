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
before public distribution, run `cargo audit` against the locked graph and
record the `cargo audit --version`, advisory-database revision, UTC time and
result with the qualification evidence. This is deliberately exceptional, as
are Java and VMware: ordinary CI neither downloads an advisory database nor
claims that an absent scanner passed.

## Distribution

A bare `wizard-os.img` is a build artifact. A public distribution must carry
`foundation.spdx`, the repository `LICENSE`, `THIRD-PARTY-NOTICES.md` and the
`licenses/` directory beside it. The runtime rows are the shipped Rust and
boot inputs. Development rows affect builds or verification but do not reach
the disk image.
