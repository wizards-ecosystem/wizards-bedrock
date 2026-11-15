# Vendored Limine bootloader

- `BOOTX64.EFI`: the Limine UEFI executable from the
  [`v10.x-binary`](https://github.com/limine-bootloader/limine/tree/v10.x-binary) branch,
  fetched 2026-07-18 (BSD-2-Clause, see `LICENSE`).
- `CHECKSUMS`: its SHA-256. Every image build checks the bytes it embeds against it.

To update, download a newer `BOOTX64.EFI` from the latest `vN.x-binary` branch and update
`CHECKSUMS` and this note in the same commit.

```
cargo xtask verify-vendor    # every file here matches CHECKSUMS and none is unlisted
```
