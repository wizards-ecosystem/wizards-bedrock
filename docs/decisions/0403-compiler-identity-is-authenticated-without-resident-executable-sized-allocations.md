# D-403 Compiler identity is authenticated without resident executable-sized allocations

Status: Accepted, 2026-10-13. Amends D-213 and D-402; advances PW-001.

**Context.** The clean complete sweep on `dd6f145` passed its pristine host
baseline but exhausted the Windows commit limit during the pristine API-audit
baseline, before mutation case one. The terminal cause was allocation failure
while `CompilerIdentity::require_unchanged` read the 210,562,627-byte
`rust-lld.exe` into a `Vec` for another SHA-256 comparison. The auditor performs
that whole-file allocation after every compiler graph, despite already owning
a bounded 64 KiB streaming SHA-256 implementation. The source and compiler
verdict were not implicated; the machine had less commit headroom than one
resident linker copy plus the running audit.

**Decision.** Compiler and host-linker identity capture and revalidation hash
the exact opened executable stream through `sha256::digest_reader`. They retain
the same SHA-256 equality, canonical tool paths, adjacent-compiler selection
and per-graph revalidation, but no longer allocate in proportion to executable
size. A source contract requires both initial identities and both repeated
checks to call the streaming helper and forbids `std::fs::read` in those two
boundaries. Permanent mutation 1,868 restores a resident compiler read and is
witnessed by that contract.

**Proof required.** The new host contract must fail on the pre-D-403 source,
pass after the streaming conversion and fail when mutation 1,868 is applied.
The ordinary gate and complete API audit must pass under the repaired source.
Because the interrupted sweep never reached mutation case one, a fresh
complete sweep bound to the repaired commit must finish with no survivor or
inconclusive result before release publication.

**Alternatives.** Weakening or reducing compiler-identity checks was rejected
because the audit must remain bound to one immutable toolchain. Supplying
`CARGO_BUILD_JOBS` was rejected because the API audit deliberately refuses
ambient compiler-semantic variables. Killing WSL or unrelated applications was
rejected as outside repository authority. Restarting the idle AppX service was
attempted as a reversible machine recovery but Windows denied control of the
protected service. Reimplementing SHA-256 or keeping a smaller whole-file cap
was rejected because the existing streaming primitive already authenticates
every byte with fixed memory.
