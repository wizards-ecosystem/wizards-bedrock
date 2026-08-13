Vendored Limine bootloader

- `BOOTX64.EFI` — Limine UEFI executable, vendored from the [`v10.x-binary`](https://github.com/limine-bootloader/limine/tree/v10.x-binary) branch of the Limine repository (BSD-2-Clause, see `LICENSE`).
- Fetched 2026-07-18.
- `CHECKSUMS` pins its SHA-256. `cargo xtask verify-vendor` and `cargo test`
  both check it, so tampering or a silent swap fails the build (D-25).

To update: download a newer `BOOTX64.EFI` from the latest `vN.x-binary` branch,
regenerate the digest, and update both this note and `CHECKSUMS` in the same
commit so the change is reviewable.

```
cargo xtask verify-vendor    # after updating, must pass
```
