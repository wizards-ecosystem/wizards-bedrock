# D-25 Vendored binaries are checksum-pinned

Status: Accepted, 2026-08-12.

**Decision.** `boot/limine/CHECKSUMS` records a SHA-256 in `sha256sum`
format, checked by `cargo xtask verify-vendor`, by `cargo xtask ci`, and by
an integration test, so tampering fails `cargo test` too. The hash function
lives in `xtask/src/sha256.rs`, verified against the NIST vectors and
cheaper than a dependency for one call site.

**Amended 2026-08-22.** A digest catches a vendored file being *changed*,
not one being *added*. `verify_listing` reads the directory as well as the
listing, and refuses anything neither an entry nor the directory's own
paperwork (`CHECKSUMS`, `LICENSE`, `README.md`) accounts for. The comparison
is a pure, host-tested `unaccounted` function.

**Consequences.** `BOOTX64.EFI` comes prebuilt from a rolling branch;
updating it is now a two-line diff a reviewer can check. Every future
vendored artifact gets the same treatment.
