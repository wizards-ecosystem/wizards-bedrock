# D-25 Vendored binaries are checksum-pinned

Status: Accepted, 2026-08-12.

**Context.** `boot/limine/BOOTX64.EFI` is a 300 KB prebuilt binary from
another project, recorded with provenance and a fetch date but no digest, and
referencing a rolling `v10.x-binary` branch. An update was unverifiable.

**Decision.** `boot/limine/CHECKSUMS` records a SHA-256 in `sha256sum` format,
checked by `cargo xtask verify-vendor`, by `cargo xtask ci`, and by an
integration test so that tampering fails `cargo test` too. The hash function
is ~90 lines in `xtask/src/sha256.rs`, verified against the NIST vectors —
cheaper than a dependency for one call site.

**Amended 2026-08-22.** A digest catches a vendored file being *changed*; it
says nothing about one being *added*, and an unlisted binary appearing beside
`BOOTX64.EFI` passed the check silently. `verify_listing` now reads the
directory as well as the listing and refuses anything neither an entry nor the
directory's own paperwork — `CHECKSUMS`, `LICENSE`, `README.md` — accounts
for. The comparison is a pure `unaccounted` function, tested on the host;
the check was broken once with a stray `EXTRA.EFI` and went red.

**Consequences.** Updating Limine is now a two-line diff a reviewer can
actually check. Every future vendored artifact gets the same treatment.
