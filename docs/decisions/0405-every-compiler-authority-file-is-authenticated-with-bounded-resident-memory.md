# D-405 Every compiler-authority file is authenticated with bounded resident memory

Status: Accepted, 2026-10-13. Amends D-213, D-403 and D-404; advances PW-001.

**Context.** After D-404, the current-source debug launcher failed before its
first compiler round while `snapshot_marker_free_file` read the large
`rustc_driver` runtime library into a `Vec`. D-403 streamed the compiler and
linker executable digests, but the broader compiler-input authority still read
runtime libraries and target artifacts whole for their combined marker and
SHA-256 check. The failure was again an allocation under external Windows
commit pressure, not an API verdict.

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

**Proof required.** Source contracts must fail before each plain-snapshot,
archive, registry-input and bounded-parser repair, pass after it and fail under
their retained whole-file or unbounded regressions. Runtime tests must prove
identical hashing, cross-chunk raw-marker refusal, binary admission, the
semantic allocation ceiling and fail-closed oversized UTF-8 handling. The
overlap-retention and oversized-text guards each require their own permanent
mutation witness. Split-chunk UTF-8 classification, digest identity and
marker-free non-Rust binary admission also require direct runtime witnesses.
Permanent mutations 1,870 through 1,881 cover the combined stream, the shared
plain stream, every authority call boundary, the allocation ceiling, chunk
overlap, oversized text, split UTF-8, digest identity and binary admission. The
complete API audit, ordinary gate and a fresh complete mutation sweep must then
pass before release publication.

**Alternatives.** Omitting runtime libraries, copied source, Cargo state or the
authenticated registry from compiler identity was rejected because each can
change compiler meaning. Scanning only raw bytes was rejected because encoded
literals could forge diagnostic markers. Reading large text or archives whole
was rejected because it recreates proportional allocation; accepting oversized
text without the semantic scan was rejected because that weakens the marker
boundary. Reopening marker-bearing files for separate hash and scan passes was
rejected because one opened stream gives a tighter identity observation.
