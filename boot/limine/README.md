Vendored Limine bootloader

- `BOOTX64.EFI` — Limine UEFI executable, vendored from the [`v10.x-binary`](https://github.com/limine-bootloader/limine/tree/v10.x-binary) branch of the Limine repository (BSD-2-Clause, see `LICENSE`).
- Fetched 2026-07-18.
- `CHECKSUMS` pins its SHA-256. ESP population authenticates the exact bounded
  buffer it embeds; `cargo xtask verify-vendor` and `cargo test` independently
  check the directory, so tampering or a silent swap fails every image route
  (D-25, D-380).

To update: download a newer `BOOTX64.EFI` from the latest `vN.x-binary` branch,
regenerate the digest, and update both this note and `CHECKSUMS` in the same
commit so the change is reviewable.

```
cargo xtask verify-vendor    # after updating, must pass
```
