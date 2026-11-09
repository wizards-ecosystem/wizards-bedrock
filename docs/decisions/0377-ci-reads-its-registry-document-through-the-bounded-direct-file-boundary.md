# D-377 CI reads its registry document through the bounded direct-file boundary

Status: Accepted, 2026-10-10. Amends D-212 and D-344; closes PW-586.

**Context.** Codex Security occurrence `occ_33f31e610855178e51859b01`
found that `cargo xtask ci` read `README.md` with
`std::fs::read_to_string` before its first registered gate stage. That read
followed a final-component link and allocated until end of file. A hostile
checkout could therefore redirect the preflight to an external object or make
the operator process consume an attacker-selected amount of memory before any
format, source, supply-chain or boot check ran.

**Decision.** The CI registry preflight treats `README.md` as a hostile
repository input. It uses the existing D-344 direct-file reader with a 64 KiB
ceiling: every ancestor is a real directory, the README is one exact direct
child, the opened handle cannot follow a final-component link, initial length
is checked before allocation, one byte beyond the ceiling detects growth, and
length plus direct-file shape are rechecked after the read. UTF-8 decoding and
the existing registry-marker comparison happen only after those checks. A
normal direct README retains exactly the same marker semantics.

**Proof required.** A host witness must present an external sentinel through
a `README.md` link and a direct sparse README one byte beyond the ceiling,
require both to stop at the file boundary, and keep a normal bounded document
with the one ordered registry region green. One retained mutation must restore
the unbounded following read and fail that hostile witness.

**Evidence.** Against the old reader, the redirected-sentinel witness failed
because the external valid-looking README was accepted, while the oversized
fixture failed because all 65,537 bytes were consumed before the parser noticed
that no marker existed. With the shared bounded reader, the three focused gate
tests accept the normal document and refuse both hostile inputs. Mutation 1,767
restored the old whole-file read and failed exactly
`ci_readme_refuses_a_redirected_file`; exact restoration returned the focused
suite to green. The registry identity and every-mutation-applies checks pass.
The shared reader's existing growth-after-measurement witness covers the same
one-byte streaming ceiling. Final integrated-gate evidence is recorded by the
closing commit.

**Alternatives.** Parsing only the marker region while streaming would bound
retained memory but still follow a redirected path and could scan an unbounded
prefix. Checking metadata before the old open leaves a swap window. Copying the
generic reader into `gate.rs` would create a second filesystem authority. The
existing checked reader is the narrowest repository-native boundary and keeps
the CI registry parser concerned only with registry semantics.
