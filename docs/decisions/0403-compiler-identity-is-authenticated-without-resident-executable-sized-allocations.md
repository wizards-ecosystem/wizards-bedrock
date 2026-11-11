# D-403 Compiler identity is authenticated without resident executable-sized allocations

Status: Accepted, 2026-10-13. Amends D-213 and D-402; advances PW-001.

**Decision.** Compiler and host-linker identity capture and revalidation hash
the exact opened executable stream through `sha256::digest_reader`. They retain
the same SHA-256 equality, canonical tool paths, adjacent-compiler selection
and per-graph revalidation, but no longer allocate in proportion to executable
size. A source contract requires both initial identities and both repeated
checks to call the streaming helper and forbids `std::fs::read` in those two
boundaries. Permanent mutation 1,868 restores a resident compiler read and is
witnessed by that contract.

**Rationale.** The clean sweep on `dd6f145` exhausted the Windows commit limit
during the pristine API-audit baseline while `CompilerIdentity::require_unchanged`
read the 210,562,627-byte `rust-lld.exe` into a `Vec` after every graph,
although the auditor already owned a bounded 64 KiB streaming SHA-256. The audit
must remain bound to one immutable toolchain, so its identity checks stay;
`CARGO_BUILD_JOBS` is an ambient compiler-semantic variable the audit refuses;
stopping unrelated applications is outside repository authority; and the
existing streaming primitive already authenticates every byte with fixed
memory.
