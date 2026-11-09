# D-436 Mutation readers open checked real-file handles

Status: Accepted (2026-10-23), qualification OPEN; strengthens D-344; advances PW-717

Three mutation-workspace readers still used ordinary `File::open` for bounded
reading or hashing. A final-component link, including one substituted after a
caller's path check, could feed bytes despite the documented checked-handle
contract.

`read_bounded_bytes_with`, `digest_tracked` and `digest_file_with` now use the
existing `open_real_file` boundary. Size limits, streaming hashes, framing and
post-use checks stay intact. The checked-path source contract includes both
confined-path helper names so it also covers callers whose precheck would hide
a stable link from a direct reader fixture.

Repeating a path-only check would retain the open-time gap. Broadening this
repair into directory-capability redesign is unnecessary for its stated
final-component guarantee. Parent-directory replacement, hardlink identity and
same-account content replacement are not newly authenticated by this change.

Two causal final-link fixtures and parsed production-path witnesses have
retained mutations. Their exact reds and restored passes, followed by current
CI and the complete isolated sweep, are required before closing PW-717.
