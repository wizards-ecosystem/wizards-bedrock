# D-385 Qualification command fixtures do not inherit the checkout that compiled them

Status: Accepted, 2026-10-11. Amends D-373.

**Context.** The complete mutation sweep builds and tests a tracked copy under
`target/mutate/<commit>-<registry>/workspace`. The cargo-mutants command tests
used `CARGO_MANIFEST_DIR`'s parent as a convenient repository fixture. In the
ordinary checkout that directory has no repository ancestor carrying Cargo
semantics, but in the isolated sweep its ancestors include the outer checkout
and its tracked `.cargo/config.toml`. D-373's production guard correctly
rejected those aliases and kernel rustflags as configuration above the supposed
repository. The pristine host baseline therefore stopped before exercising any
mutation even though the same source passed from the ordinary checkout.

**Decision.** Filesystem-backed cargo-mutants command fixtures own a uniquely
named real directory below the operating system temporary directory and remove
that exact directory after each guarded fixture operation. Their Cargo discovery ancestry is
therefore independent of where `xtask` was compiled. The tests still run the
complete production environment constructor, including its private Cargo home,
private temporary directory and exact parent-configuration allowlist. No
production command, configuration exception or mutation-workspace location
changes.

**Proof required.** A dedicated host test must construct the owned fixture,
run the production environment constructor and prove that the fixture is not
nested below the compiling checkout. A persistent mutation must restore the
checkout-nested fixture parent and make that exact test red inside the isolated
mutation workspace. Restored host tests must then pass from that same nested
workspace and the complete sweep must advance beyond its pristine baseline.

**Alternatives.** Allowing the outer checkout's full Cargo configuration would
weaken D-373 and permit compiler or runner semantics above a qualification
repository. Special-casing mutation paths would make production policy depend
on an unauthenticated pathname. Moving the multi-gigabyte mutation workspace
outside `target/` would discard its existing bounded cleanup, locking and
resumption authority to solve a test-fixture mistake.
