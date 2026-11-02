# Stream 4 — boot path and developer tooling

Scope: `boot/`, `xtask/`, `tools/`, `wost/`, and this note. Other streams own
the root workspace config, `kernel/`, and `libs/`.

## Done

- QEMU resolution no longer reports a missing emulator as missing UEFI
  firmware. `QEMU` and `QEMU_DIR` must name a real file before firmware search.
  Discovery names the Windows default, `PATH`, both variables, and the winget
  install command. Firmware search also covers an FHS prefix (`bin/` next to
  `share/qemu/`) and lists every path it tried, including the required edk2
  image names.
- A mistyped task within edit distance 2 names the nearest command. `run`
  flags that look like QEMU options say they belong after `--`.
- `cargo xtask help` names `QEMU`, `QEMU_DIR`, and `GDB`, and the two-edit
  suggestion window.
- A kernel build checks that the pinned toolchain actually contains `rustc`
  and `rustdoc` before cargo is spawned. The error names the missing file,
  `rust-toolchain.toml`, and `rustup show`.

## In progress

- Reading `wost/src/boot` for a handoff defect a host test can prove.

## Next steps

- Keep looking through boot handoff and image assembly for a defect that is
  wrong, not merely stylistic.
- Update the PR description with the bugs, DX, cleanups, and follow-ups.

## Bugs found

- A `QEMU` or `QEMU_DIR` value that was not a file was accepted. Firmware
  search then failed against that path's parent, so the error said the edk2
  image was missing.
- Firmware was sought only under the executable's directory. A `bin/` install
  with images in the prefix `share/qemu/` was reported missing even when the
  edk2 files were present.

## Decisions

- Do not accept `OVMF_CODE.fd` / `OVMF_VARS.fd` as substitutes. The runner is
  pinned to `edk2-x86_64-code.fd` and `edk2-i386-vars.fd`; the error names the
  difference instead of booting a different firmware image.
- No full `cargo xtask ci`, QEMU boot, or mutation campaign in this stream.
  Focused `cargo test -p xtask --lib` filters and `cargo check -p xtask` only.
