# D-405 Every compiler-authority file is authenticated with bounded resident memory

Status: Accepted, 2026-10-13. Amends D-213, D-403 and D-404; advances PW-001.

**Decision.** Every compiler-authority file operation has an explicit resident
memory bound. Plain source, copied-source, private-Cargo and registry-archive
identity snapshots stream SHA-256 in fixed 64 KiB chunks. Authenticated
registry, generated-input, compiler-runtime and target-library files are opened
once and consumed in the same fixed chunks: one pass computes the identical
SHA-256 digest, detects the raw reserved marker across chunk boundaries and
incrementally classifies UTF-8. Valid UTF-8 remains subject to the existing
decoded-literal token scan within an 8 MiB resident-text ceiling; larger valid
text fails closed rather than escaping semantic marker inspection. Non-UTF-8
authority remains admissible only when it is not Rust source and its complete
byte stream is raw-marker free. Semantic source, manifest, lockfile and
registry-checksum parsing is allowed to retain a whole file only below that
same explicit 8 MiB ceiling. A production source contract forbids unbounded
`std::fs::read` and `std::fs::read_to_string` anywhere in the API-audit
authority path.

Runtime tests prove identical hashing, refusal of a reserved sequence split
across chunks, split-chunk UTF-8 classification, binary admission, the semantic
allocation ceiling and fail-closed oversized UTF-8 handling. Permanent mutations 1,870 through 1,881 cover the combined stream, the
shared plain stream, every authority call boundary, the allocation ceiling,
chunk overlap, oversized text, split UTF-8, digest identity and binary
admission.

**Rationale.** After D-404, the debug launcher failed under Windows commit
pressure while `snapshot_marker_free_file` read the large `rustc_driver` runtime
library whole; D-403 had streamed only the executable digests. Runtime
libraries, copied source, Cargo state and the authenticated registry can each
change compiler meaning, so none leaves compiler identity. Scanning only raw
bytes would let encoded literals forge diagnostic tokens, accepting oversized
text without the semantic scan would weaken that boundary, and one opened
stream for hash and scan gives a tighter identity observation than reopening.
