# D-377 CI reads its registry document through the bounded direct-file boundary

Status: Accepted, 2026-10-10. Amends D-212 and D-344; closes PW-586.

**Decision.** The CI registry preflight treats `README.md` as a hostile
repository input. It uses the existing D-344 direct-file reader with a 64 KiB
ceiling: every ancestor is a real directory, the README is one exact direct
child, the opened handle cannot follow a final-component link, initial length
is checked before allocation, one byte beyond the ceiling detects growth, and
length plus direct-file shape are rechecked after the read. UTF-8 decoding and
the registry comparison happen only after those checks; a normal direct README
keeps the same semantics.

A host witness presents an external sentinel through a `README.md` link and a
direct sparse README one byte beyond the ceiling, requires both to stop at the
file boundary, and keeps a normal bounded document with the one ordered
registry region green. Mutation 1,767 restores the unbounded following read and
fails exactly `ci_readme_refuses_a_redirected_file`.

**Rationale.** Codex Security occurrence `occ_33f31e610855178e51859b01` found
that `cargo xtask ci` read `README.md` with `std::fs::read_to_string` before its
first gate stage, following a final-component link and allocating until end of
file, so a hostile checkout could redirect the preflight or exhaust operator
memory before any other check ran. Parsing only the registry region while
streaming would still follow a redirected path and could scan an unbounded
prefix; checking metadata before the old open leaves a swap window; and a copy
of the generic reader in `gate.rs` would create a second filesystem authority.
