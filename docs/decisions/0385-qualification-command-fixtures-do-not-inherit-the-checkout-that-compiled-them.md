# D-385 Qualification command fixtures do not inherit the checkout that compiled them

Status: Accepted, 2026-10-11. Amends D-373.

**Decision.** Filesystem-backed cargo-mutants command fixtures own a uniquely
named real directory below the operating system temporary directory and remove
that exact directory after each guarded fixture operation. Their Cargo discovery ancestry is
therefore independent of where `xtask` was compiled. The tests still run the
complete production environment constructor, including its private Cargo home,
private temporary directory and exact parent-configuration allowlist. No
production command, configuration exception or mutation-workspace location
changes.

A host test constructs the owned fixture, runs the production environment
constructor and proves that the fixture is not nested below the compiling
checkout. A persistent mutation restores the checkout-nested fixture parent and
makes that exact test red inside the isolated mutation workspace.

**Rationale.** The fixtures had used `CARGO_MANIFEST_DIR`'s parent, whose
ancestors inside the sweep's `target/mutate/<commit>-<registry>/workspace` copy
include the outer checkout and its `.cargo/config.toml`, so D-373's guard
correctly refused and the pristine host baseline stopped before any mutation.
Allowing the outer checkout's Cargo configuration would weaken D-373,
special-casing mutation paths would make policy depend on an unauthenticated
pathname, and moving the mutation workspace outside `target/` would discard its
bounded cleanup, locking and resumption authority to fix a test fixture.
